//! Direct GitHub calls for the to-do list. They use `gh` without the AI,
//! so the list loads fast and costs nothing.
//! The only change is close_item(), and only the desktop app calls it, after
//! the user clicks a button. The AI tools stay read-only.

use anyhow::bail;
use serde::{Deserialize, Serialize};
use tokio::process::Command;

/// One open GitHub item that needs the user.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GithubItem {
    /// "owner/name#number".
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// "assigned", "review_requested", or "your_pr".
    pub reason: String,
    pub updated: String,
}

#[derive(Deserialize)]
struct Raw {
    repository: RawRepo,
    number: u64,
    title: String,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
}

#[derive(Deserialize)]
struct RawRepo {
    #[serde(rename = "nameWithOwner")]
    name_with_owner: String,
}

/// Owners from AIFRED_GITHUB_OWNERS (comma-separated). Empty means all.
pub fn owners_from_env() -> Vec<String> {
    parse_owners(&std::env::var("AIFRED_GITHUB_OWNERS").unwrap_or_default())
}

fn parse_owners(s: &str) -> Vec<String> {
    s.split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

/// The most results one search returns.
const SEARCH_LIMIT: usize = 50;

async fn search(kind: &str, filter: &str, reason: &str, owners: &[String]) -> anyhow::Result<Vec<GithubItem>> {
    let mut cmd = Command::new("gh");
    let limit = SEARCH_LIMIT.to_string();
    cmd.args(["search", kind, filter, "--state=open", "--limit", &limit])
        .args(["--json", "repository,number,title,url,updatedAt"]);
    for o in owners {
        cmd.arg(format!("--owner={o}"));
    }
    let out = cmd.output().await?;
    if !out.status.success() {
        bail!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse(&out.stdout, reason)
}

/// Turns the JSON output of `gh search` into items.
fn parse(json: &[u8], reason: &str) -> anyhow::Result<Vec<GithubItem>> {
    let raw: Vec<Raw> = serde_json::from_slice(json)?;
    Ok(raw
        .into_iter()
        .map(|r| GithubItem {
            id: format!("{}#{}", r.repository.name_with_owner, r.number),
            repo: r.repository.name_with_owner,
            number: r.number,
            title: r.title,
            url: r.url,
            reason: reason.into(),
            updated: r.updated_at,
        })
        .collect())
}

/// The open items from GitHub.
pub struct Fetch {
    pub items: Vec<GithubItem>,
    /// False if a search hit its limit. Then an item that is not in `items`
    /// can still be open on GitHub.
    pub complete: bool,
}

/// Issues assigned to the user, reviews asked of the user, and the user's
/// own open pull requests. The three searches run at the same time.
pub async fn my_items(owners: &[String]) -> anyhow::Result<Fetch> {
    let (a, b, c) = tokio::try_join!(
        search("prs", "--review-requested=@me", "review_requested", owners),
        search("issues", "--assignee=@me", "assigned", owners),
        search("prs", "--author=@me", "your_pr", owners),
    )?;
    let complete = [&a, &b, &c].iter().all(|l| l.len() < SEARCH_LIMIT);
    Ok(Fetch { items: merge([a, b, c]), complete })
}

impl GithubItem {
    pub fn is_pull_request(&self) -> bool {
        self.url.contains("/pull/")
    }

    /// The user cannot close a pull request that only asks for their review.
    pub fn can_close(&self) -> bool {
        self.reason != "review_requested"
    }
}

/// True if the repository owner is allowed by AIFRED_GITHUB_OWNERS.
fn owner_allowed(repo: &str, owners: &[String]) -> bool {
    let owner = repo.split('/').next().unwrap_or_default();
    owners.is_empty() || owners.iter().any(|o| o.eq_ignore_ascii_case(owner))
}

/// A repository name must look like "owner/name", so it cannot pass as a `gh`
/// option or change the API path ("." and ".." are refused).
fn check_repo(item: &GithubItem, owners: &[String]) -> anyhow::Result<()> {
    let ok_part = |p: &str| {
        !p.is_empty()
            && p != "."
            && p != ".."
            && !p.starts_with('-')
            && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
    };
    let parts: Vec<&str> = item.repo.split('/').collect();
    if parts.len() != 2 || !parts.iter().all(|p| ok_part(p)) {
        bail!("bad repository name {:?}", item.repo);
    }
    if !owner_allowed(&item.repo, owners) {
        bail!("{} is not in AIFRED_GITHUB_OWNERS", item.repo);
    }
    Ok(())
}

async fn gh(args: &[&str]) -> anyhow::Result<String> {
    let out = Command::new("gh").args(args).output().await?;
    if !out.status.success() {
        bail!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

/// True if the issue or pull request is still open on GitHub.
pub async fn is_open(item: &GithubItem, owners: &[String]) -> anyhow::Result<bool> {
    check_repo(item, owners)?;
    // The issues endpoint also answers for pull requests.
    let path = format!("repos/{}/issues/{}", item.repo, item.number);
    Ok(gh(&["api", &path, "--jq", ".state"]).await? == "open")
}

/// Closes the issue or pull request on GitHub. A pull request is closed
/// without a merge.
pub async fn close_item(item: &GithubItem, owners: &[String]) -> anyhow::Result<()> {
    check_repo(item, owners)?;
    if !item.can_close() {
        bail!("this pull request only asks for your review, so aifred does not close it");
    }
    let kind = if item.is_pull_request() { "pr" } else { "issue" };
    gh(&[kind, "close", &item.number.to_string(), "--repo", &item.repo]).await?;
    Ok(())
}

/// An item can match more than one search. Keep the first (most urgent) one.
fn merge(lists: [Vec<GithubItem>; 3]) -> Vec<GithubItem> {
    let mut seen = std::collections::HashSet::new();
    lists.into_iter().flatten().filter(|i| seen.insert(i.id.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
        {"repository":{"name":"shop","nameWithOwner":"acme/shop"},"number":7,"title":"Fix cart",
         "url":"https://github.com/acme/shop/pull/7","updatedAt":"2026-10-01T10:00:00Z"},
        {"repository":{"name":"docs","nameWithOwner":"acme/docs"},"number":12,"title":"Typo",
         "url":"https://github.com/acme/docs/pull/12","updatedAt":"2026-09-01T10:00:00Z"}
    ]"#;

    fn item(repo: &str, reason: &str, url: &str) -> GithubItem {
        GithubItem {
            id: format!("{repo}#1"),
            repo: repo.into(),
            number: 1,
            title: "T".into(),
            url: url.into(),
            reason: reason.into(),
            updated: String::new(),
        }
    }

    #[test]
    fn repo_names_and_owner_limit_are_checked_before_gh_runs() {
        let ok = item("acme/shop", "assigned", "https://github.com/acme/shop/issues/1");
        assert!(check_repo(&ok, &[]).is_ok());
        assert!(check_repo(&ok, &["ACME".into()]).is_ok(), "owner names ignore case");
        assert!(check_repo(&ok, &["other".into()]).is_err());
        assert!(check_repo(&item("acme/shop.js", "assigned", ""), &[]).is_ok(), "dots inside a name are fine");
        for bad in ["--help", "acme", "acme/shop/x", "acme/-x", "acme/sh op", "/shop", "acme/..", "../shop", "acme/."] {
            assert!(check_repo(&item(bad, "assigned", ""), &[]).is_err(), "{bad} must be refused");
        }
    }

    #[tokio::test]
    async fn review_requests_cannot_be_closed() {
        let r = item("acme/shop", "review_requested", "https://github.com/acme/shop/pull/1");
        assert!(!r.can_close());
        assert!(r.is_pull_request());
        let e = close_item(&r, &[]).await.unwrap_err().to_string();
        assert!(e.contains("review"), "got {e}");
    }

    #[test]
    fn parse_reads_gh_output() {
        let items = parse(SAMPLE.as_bytes(), "review_requested").unwrap();
        assert_eq!(items.len(), 2);
        let i = &items[0];
        assert_eq!(i.id, "acme/shop#7");
        assert_eq!(i.repo, "acme/shop");
        assert_eq!(i.number, 7);
        assert_eq!(i.title, "Fix cart");
        assert_eq!(i.reason, "review_requested");
        assert_eq!(i.updated, "2026-10-01T10:00:00Z");
    }

    #[test]
    fn parse_accepts_empty_and_rejects_bad_json() {
        assert!(parse(b"[]", "assigned").unwrap().is_empty());
        assert!(parse(b"not json", "assigned").is_err());
    }

    #[test]
    fn merge_keeps_first_reason_and_order() {
        let reviews = parse(SAMPLE.as_bytes(), "review_requested").unwrap();
        let assigned = parse(b"[]", "assigned").unwrap();
        let mine = parse(SAMPLE.as_bytes(), "your_pr").unwrap();
        let all = merge([reviews, assigned, mine]);
        assert_eq!(all.len(), 2, "duplicates are removed");
        assert!(all.iter().all(|i| i.reason == "review_requested"));
        assert_eq!(all[0].id, "acme/shop#7");
    }

    #[test]
    fn owners_are_trimmed_and_blanks_dropped() {
        assert_eq!(parse_owners(" acme , ,me,"), vec!["acme", "me"]);
        assert!(parse_owners("").is_empty());
    }
}
