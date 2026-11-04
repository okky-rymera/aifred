//! Direct GitHub calls for the to-do list. They use `gh` without the AI,
//! so the list loads fast and costs nothing.
//! They only read GitHub. AIfred never changes anything on GitHub.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;

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
    /// For a pull request: the issues it closes when it merges.
    /// For an issue: the open pull requests that close it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub links: Vec<Link>,
    /// True when GitHub has more links than `links` holds (it sends at most
    /// LINK_LIMIT). The list then does not hide this pull request inside an
    /// issue, because the missing links would show nowhere.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub more_links: bool,
    /// The description from the search. It goes into the details column of
    /// the database (db.rs), not into the saved item, so the window and the
    /// list tools do not get it.
    #[serde(skip)]
    pub body: Option<String>,
}

/// An issue or pull request that is linked to a list item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// "owner/name#number".
    pub id: String,
    pub repo: String,
    pub number: u64,
    pub title: String,
    pub url: String,
    /// True for a pull request, false for an issue. The window uses it for
    /// the badge label, so it does not need its own copy of the check.
    #[serde(default)]
    pub pull_request: bool,
    /// The reason when the linked item is also in your list. The list view
    /// sets it. GitHub does not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

impl Link {
    fn new(repo: String, number: u64, title: String, url: String) -> Self {
        Link {
            id: item_id(&repo, number),
            pull_request: is_pull_request_url(&url),
            repo,
            number,
            title,
            url,
            reason: None,
        }
    }
}

impl From<&GithubItem> for Link {
    fn from(i: &GithubItem) -> Self {
        Link::new(i.repo.clone(), i.number, i.title.clone(), i.url.clone())
    }
}

/// "owner/name#number".
fn item_id(repo: &str, number: u64) -> String {
    format!("{repo}#{number}")
}

fn is_pull_request_url(url: &str) -> bool {
    url.contains("/pull/")
}

#[derive(Deserialize)]
struct Raw {
    /// GitHub's node ID. The link query uses it.
    id: String,
    repository: RawRepo,
    number: u64,
    title: String,
    url: String,
    #[serde(rename = "updatedAt")]
    updated_at: String,
    #[serde(default)]
    body: Option<String>,
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

/// The `gh` arguments for one search.
fn search_args(kind: &str, filter: &str, owners: &[String]) -> Vec<String> {
    let mut args: Vec<String> = [
        "search",
        kind,
        filter,
        "--state=open",
        // Archived repositories are read-only, so their items need no work.
        "--archived=false",
        "--limit",
        &SEARCH_LIMIT.to_string(),
        "--json",
        "id,repository,number,title,url,updatedAt,body",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    args.extend(owners.iter().map(|o| format!("--owner={o}")));
    args
}

/// One search result: GitHub's node ID and the item.
type Found = (String, GithubItem);

async fn search(kind: &str, filter: &str, reason: &str, owners: &[String]) -> anyhow::Result<Vec<Found>> {
    let out = Command::new("gh").args(search_args(kind, filter, owners)).output().await?;
    if !out.status.success() {
        bail!("gh failed: {}", String::from_utf8_lossy(&out.stderr).trim());
    }
    parse(&out.stdout, reason)
}

/// Turns the JSON output of `gh search` into items.
fn parse(json: &[u8], reason: &str) -> anyhow::Result<Vec<Found>> {
    let raw: Vec<Raw> = serde_json::from_slice(json)?;
    Ok(raw
        .into_iter()
        .map(|r| {
            let item = GithubItem {
                id: item_id(&r.repository.name_with_owner, r.number),
                repo: r.repository.name_with_owner,
                number: r.number,
                title: r.title,
                url: r.url,
                reason: reason.into(),
                updated: r.updated_at,
                links: Vec::new(),
                more_links: false,
                body: r.body,
            };
            (r.id, item)
        })
        .collect())
}

/// The most items in one link query.
const LINK_BATCH: usize = 50;
/// Reads up to 10 (LINK_LIMIT) links for each item, and the real count.
const LINK_QUERY: &str = "query($ids: [ID!]!) { nodes(ids: $ids) { \
    ... on PullRequest { closingIssuesReferences(first: 10) { totalCount nodes { ...L } } } \
    ... on Issue { closedByPullRequestsReferences(first: 10, includeClosedPrs: false) { totalCount nodes { ...L } } } } } \
    fragment L on Node { ... on Issue { number title url repository { nameWithOwner } } \
    ... on PullRequest { number title url repository { nameWithOwner } } }";

#[derive(Deserialize)]
struct LinkReply {
    data: Option<LinkData>,
}

#[derive(Deserialize)]
struct LinkData {
    nodes: Vec<Option<LinkNode>>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkNode {
    closing_issues_references: Option<LinkNodes>,
    closed_by_pull_requests_references: Option<LinkNodes>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LinkNodes {
    #[serde(default)]
    total_count: usize,
    nodes: Vec<Option<RawLink>>,
}

/// The links of one item, as GitHub sent them.
#[derive(Debug, Default)]
struct ItemLinks {
    links: Vec<Link>,
    /// GitHub has more links than it sent.
    more: bool,
}

#[derive(Deserialize)]
struct RawLink {
    number: u64,
    title: String,
    url: String,
    repository: RawRepo,
}

/// Reads the reply to LINK_QUERY. The links come in the same order as the IDs.
/// A missing item (for example, deleted since the search) gets no links.
fn parse_links(json: &[u8]) -> anyhow::Result<Vec<ItemLinks>> {
    let reply: LinkReply = serde_json::from_slice(json)?;
    let Some(data) = reply.data else { bail!("GitHub sent no link data") };
    Ok(data
        .nodes
        .into_iter()
        .map(|n| {
            let Some(n) = n else { return ItemLinks::default() };
            let Some(list) = n.closing_issues_references.or(n.closed_by_pull_requests_references) else {
                return ItemLinks::default();
            };
            let more = list.total_count > list.nodes.len();
            let links = list
                .nodes
                .into_iter()
                .flatten()
                .map(|r| Link::new(r.repository.name_with_owner, r.number, r.title, r.url))
                .collect();
            ItemLinks { links, more }
        })
        .collect())
}

/// Asks GitHub for the links of up to LINK_BATCH items in one call.
async fn links_batch(ids: &[String]) -> anyhow::Result<Vec<ItemLinks>> {
    use tokio::io::AsyncWriteExt;
    let body = serde_json::json!({ "query": LINK_QUERY, "variables": { "ids": ids } });
    let mut child = Command::new("gh")
        .args(["api", "graphql", "--input", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        // When the fetch stops early (another search failed), stop gh too.
        .kill_on_drop(true)
        .spawn()?;
    let mut stdin = child.stdin.take().expect("stdin is piped");
    // gh can stop before it reads the query (for example, when you are not
    // logged in). Then the write fails, but gh's own message tells why.
    // So keep the write error and read gh's output first.
    let written = stdin.write_all(body.to_string().as_bytes()).await;
    drop(stdin);
    let out = child.wait_with_output().await?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    if let Err(e) = written {
        bail!("cannot send the link query to gh: {e}; gh said: {}", stderr.trim());
    }
    // gh fails when one item is missing, but the reply still has the others.
    // So read the reply first, and use the exit status only when it has no data.
    match parse_links(&out.stdout) {
        Ok(l) if l.len() == ids.len() => Ok(l),
        Ok(l) => bail!("gh link query sent {} entries for {} items: {}", l.len(), ids.len(), stderr.trim()),
        Err(e) => bail!("gh link query failed: {e}; gh said: {}", stderr.trim()),
    }
}

/// Adds the links to the items at the positions in `which`. The batches run
/// at the same time. Most lists need only one batch. If the caller stops
/// early, the JoinSet stops the batches and kill_on_drop stops gh.
///
/// Returns the IDs ("owner/name#number") of the items whose batch failed.
/// Their links are unknown, not empty: the caller keeps the saved links.
async fn add_links(found: &mut [Found], which: &[usize]) -> HashSet<String> {
    let batches: Vec<&[usize]> = which.chunks(LINK_BATCH).collect();
    let mut calls = tokio::task::JoinSet::new();
    for (n, batch) in batches.iter().enumerate() {
        let ids: Vec<String> = batch.iter().map(|&i| found[i].0.clone()).collect();
        calls.spawn(async move { (n, links_batch(&ids).await) });
    }
    let mut ok = HashSet::new();
    while let Some(done) = calls.join_next().await {
        match done {
            Ok((n, Ok(links))) => {
                for (&i, l) in batches[n].iter().zip(links) {
                    found[i].1.links = l.links;
                    found[i].1.more_links = l.more;
                }
                ok.insert(n);
            }
            Ok((_, Err(e))) => eprintln!("aifred: cannot read GitHub links: {e}"),
            // The batch task panicked. Its items count as failed below.
            Err(e) => eprintln!("aifred: cannot read GitHub links: {e}"),
        }
    }
    batches
        .iter()
        .enumerate()
        .filter(|(n, _)| !ok.contains(n))
        .flat_map(|(_, b)| b.iter().map(|&i| found[i].1.id.clone()))
        .collect()
}

/// The open items from GitHub.
pub struct Fetch {
    pub items: Vec<GithubItem>,
    /// The IDs of items whose links GitHub did not send. Keep their saved
    /// links (GithubStore::save_fetch does this), so the rows do not change shape.
    pub no_links: Vec<String>,
}

/// The result of one search and its link lookup.
struct Searched {
    found: Vec<Found>,
    /// The items whose links this search looked up.
    looked_up: HashSet<String>,
    /// The items whose link lookup failed.
    failed: HashSet<String>,
}

/// One search, then the links of its results. Each search asks for its links
/// when it ends, so the link calls do not wait for the slowest search.
///
/// An item can be in two searches. `claimed` holds the items that a search
/// already looks up, so each item is looked up only once.
async fn search_with_links(
    kind: &str,
    filter: &str,
    reason: &str,
    owners: &[String],
    claimed: &Mutex<HashSet<String>>,
) -> anyhow::Result<Searched> {
    let mut found = search(kind, filter, reason, owners).await?;
    let which: Vec<usize> = {
        let mut c = claimed.lock().expect("no panic holds this lock");
        (0..found.len()).filter(|&i| c.insert(found[i].1.id.clone())).collect()
    };
    let looked_up = which.iter().map(|&i| found[i].1.id.clone()).collect();
    // The links only join an issue and its pull requests in one row.
    // Without them the list still works, so a failure here is not an error.
    let failed = add_links(&mut found, &which).await;
    for (_, item) in &mut found {
        drop_other_owners(&mut item.links, owners);
    }
    Ok(Searched { found, looked_up, failed })
}

/// Removes the links to repositories outside AIFRED_GITHUB_OWNERS. The search
/// uses the owners, but a link can point to any repository.
fn drop_other_owners(links: &mut Vec<Link>, owners: &[String]) {
    if owners.is_empty() {
        return;
    }
    // GitHub owner names do not depend on case.
    let owner = |repo: &str| repo.split('/').next().unwrap_or_default().to_string();
    links.retain(|l| owners.iter().any(|o| o.eq_ignore_ascii_case(&owner(&l.repo))));
}

/// Issues assigned to the user, reviews asked of the user, and the user's
/// own open pull requests. The three searches run at the same time.
pub async fn my_items(owners: &[String]) -> anyhow::Result<Fetch> {
    let claimed = Mutex::new(HashSet::new());
    let (a, b, c) = tokio::try_join!(
        search_with_links("prs", "--review-requested=@me", "review_requested", owners, &claimed),
        search_with_links("issues", "--assignee=@me", "assigned", owners, &claimed),
        search_with_links("prs", "--author=@me", "your_pr", owners, &claimed),
    )?;
    Ok(combine([a, b, c]))
}

/// Merges the three searches. merge() keeps the first copy of an item, but
/// the links can be on the copy of another search: the one that looked them up.
fn combine(lists: [Searched; 3]) -> Fetch {
    let mut links = HashMap::new();
    let mut no_links = Vec::new();
    for s in &lists {
        for (_, i) in &s.found {
            if !s.looked_up.contains(&i.id) {
                continue;
            }
            if s.failed.contains(&i.id) {
                no_links.push(i.id.clone());
            } else {
                links.insert(i.id.clone(), (i.links.clone(), i.more_links));
            }
        }
    }
    let items = merge(lists.map(|s| s.found))
        .into_iter()
        .map(|(_, mut i)| {
            if let Some((l, more)) = links.remove(&i.id) {
                i.links = l;
                i.more_links = more;
            }
            i
        })
        .collect();
    Fetch { items, no_links }
}

impl GithubItem {
    pub fn is_pull_request(&self) -> bool {
        is_pull_request_url(&self.url)
    }
}

/// An item can match more than one search. Keep the first (most urgent) one.
fn merge(lists: [Vec<Found>; 3]) -> Vec<Found> {
    let mut seen = HashSet::new();
    lists.into_iter().flatten().filter(|(_, i)| seen.insert(i.id.clone())).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"[
        {"id":"PR_7","repository":{"name":"shop","nameWithOwner":"acme/shop"},"number":7,"title":"Fix cart",
         "url":"https://github.com/acme/shop/pull/7","updatedAt":"2026-10-01T10:00:00Z"},
        {"id":"PR_12","repository":{"name":"docs","nameWithOwner":"acme/docs"},"number":12,"title":"Typo",
         "url":"https://github.com/acme/docs/pull/12","updatedAt":"2026-09-01T10:00:00Z"}
    ]"#;

    #[test]
    fn parse_reads_gh_output() {
        let items = parse(SAMPLE.as_bytes(), "review_requested").unwrap();
        assert_eq!(items.len(), 2);
        let (node, i) = &items[0];
        assert_eq!(node, "PR_7");
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
        assert!(all.iter().all(|(_, i)| i.reason == "review_requested"));
        assert_eq!(all[0].1.id, "acme/shop#7");
    }

    #[test]
    fn parse_links_reads_both_directions_and_keeps_the_order() {
        let reply = r#"{"data":{"nodes":[
            {"closingIssuesReferences":{"totalCount":1,"nodes":[
                {"number":142,"title":"Merge","url":"https://github.com/acme/shop/issues/142",
                 "repository":{"nameWithOwner":"acme/shop"}}]}},
            null,
            {"closedByPullRequestsReferences":{"totalCount":3,"nodes":[
                {"number":143,"title":"PR A","url":"https://github.com/acme/shop/pull/143","repository":{"nameWithOwner":"acme/shop"}},
                {"number":9,"title":"PR B","url":"https://github.com/acme/docs/pull/9","repository":{"nameWithOwner":"acme/docs"}}]}},
            {}
        ]}}"#;
        let r = parse_links(reply.as_bytes()).unwrap();
        assert_eq!(r.len(), 4, "one entry for each ID, also for a missing item");
        let l: Vec<&Vec<Link>> = r.iter().map(|x| &x.links).collect();
        assert_eq!(l[0].iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["acme/shop#142"]);
        assert!(l[1].is_empty());
        assert_eq!(l[2].iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), vec!["acme/shop#143", "acme/docs#9"]);
        assert_eq!(l[2][1].repo, "acme/docs");
        assert!(l[3].is_empty());
        assert!(parse_links(br#"{"errors":[]}"#).is_err());
        assert!(!l[0][0].pull_request, "142 is an issue");
        assert!(l[2][0].pull_request, "143 is a pull request");
        assert!(!r[0].more, "GitHub sent all links of 142");
        assert!(r[2].more, "GitHub has 3 links but sent 2");
    }

    #[test]
    fn links_to_other_owners_are_removed() {
        let link = |repo: &str| Link::new(repo.into(), 1, "T".into(), format!("https://github.com/{repo}/pull/1"));
        let mut l = vec![link("acme/shop"), link("personal/fork"), link("ACME/docs")];
        drop_other_owners(&mut l, &["acme".into()]);
        assert_eq!(l.iter().map(|x| x.repo.as_str()).collect::<Vec<_>>(), vec!["acme/shop", "ACME/docs"]);
        let mut l = vec![link("personal/fork")];
        drop_other_owners(&mut l, &[]);
        assert_eq!(l.len(), 1, "no owners means all owners");
    }

    #[test]
    fn combine_takes_the_links_from_the_search_that_looked_them_up() {
        let set = |ids: &[&str]| ids.iter().map(|s| s.to_string()).collect::<HashSet<_>>();
        let searched = |reason: &str, looked: &[&str], failed: &[&str]| Searched {
            found: parse(SAMPLE.as_bytes(), reason).unwrap(),
            looked_up: set(looked),
            failed: set(failed),
        };
        let empty = || Searched { found: Vec::new(), looked_up: HashSet::new(), failed: HashSet::new() };
        // The "your PR" search ended first and looked up #7. The review copy is kept.
        let mut mine = searched("your_pr", &["acme/shop#7"], &[]);
        mine.found[0].1.links = vec![Link::new("acme/shop".into(), 6, "Bug".into(), "u".into())];
        let f = combine([searched("review_requested", &["acme/docs#12"], &[]), empty(), mine]);
        assert_eq!(f.items[0].reason, "review_requested", "merge keeps the first copy");
        assert_eq!(f.items[0].links.len(), 1, "with the links from the other search");
        assert!(f.no_links.is_empty());
        // The lookup of #7 failed: its links are unknown.
        let f = combine([searched("review_requested", &[], &[]), empty(), searched("your_pr", &["acme/shop#7"], &["acme/shop#7"])]);
        assert_eq!(f.no_links, vec!["acme/shop#7"]);
    }

    #[test]
    fn items_without_links_load_from_an_old_saved_list() {
        let i: GithubItem = serde_json::from_str(
            r#"{"id":"a/b#1","repo":"a/b","number":1,"title":"T","url":"u","reason":"assigned","updated":""}"#,
        )
        .unwrap();
        assert!(i.links.is_empty());
    }

    #[test]
    fn searches_skip_archived_repos_and_keep_the_owner_limit() {
        let args = search_args("issues", "--assignee=@me", &["acme".into(), "me".into()]);
        assert!(args.contains(&"--archived=false".to_string()), "got {args:?}");
        assert!(args.contains(&"--state=open".to_string()));
        assert!(args.ends_with(&["--owner=acme".to_string(), "--owner=me".to_string()]));
    }

    #[test]
    fn owners_are_trimmed_and_blanks_dropped() {
        assert_eq!(parse_owners(" acme , ,me,"), vec!["acme", "me"]);
        assert!(parse_owners("").is_empty());
    }
}
