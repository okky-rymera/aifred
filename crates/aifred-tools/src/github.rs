//! Read-only GitHub tools for aifred, served over MCP (stdio).
//!
//! The server calls the `gh` CLI, so it uses the user's existing `gh` login.
//! It never receives or stores a token itself.
//!
//! Access limit: set `AIFRED_GITHUB_OWNERS` to a comma-separated list of
//! users or orgs (for example `Rymera-Web-Co,okky-rymera`). Searches are then
//! limited to those owners, and detail lookups outside them are refused.
//!
//! search_my_items searches the list that AIfred saved in its database
//! ($AIFRED_HOME, or ~/.aifred). It downloads a new list first when the
//! saved one is too old. item_details saves the description there for search.

use aifred_core::default_data_dir;
use aifred_core::github_list::GithubStore;
use aifred_core::text::cut;

use rmcp::{
    ErrorData, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router, transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};
use tokio::process::Command;

const SEARCH_FIELDS: &str = "repository,number,title,updatedAt,url,labels";
const MAX_LIMIT: u32 = 100;
const MAX_BODY_CHARS: usize = 4000;
/// The last comments that item_details sends, and the size of each one.
const MAX_COMMENTS: usize = 5;
const MAX_COMMENT_CHARS: usize = 1500;

#[derive(Clone)]
struct GithubTools {
    /// Lowercase owner names. Empty means no owner limit.
    owners: Vec<String>,
    /// The aifred data folder, with the saved GitHub list.
    dir: std::path::PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Words to find in the title, the repository, and the saved description. Example: "checkout button".
    query: String,
    /// Also include items that are no longer in the user's list, for example closed issues. Default false.
    include_old: Option<bool>,
    /// Maximum number of items to return (1-50). Default 10.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Maximum number of items to return (1-100). Default 30.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct DetailArgs {
    /// Repository in the form "owner/name".
    repo: String,
    /// Issue or pull request number.
    number: u64,
}

impl GithubTools {
    fn from_env() -> anyhow::Result<Self> {
        let owners = std::env::var("AIFRED_GITHUB_OWNERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        Ok(Self {
            owners,
            dir: default_data_dir()?,
        })
    }

    fn store(&self) -> GithubStore {
        GithubStore::new(&self.dir)
    }

    /// Downloads a new list when the saved one is too old. Returns a note for
    /// the AI when that fails: the search then uses the older saved list.
    async fn refresh(&self) -> Option<String> {
        self.store().refresh_if_old().await
    }

    /// The `gh` arguments for one search (`issues` or `prs`), with the owner limit.
    /// Items in archived repositories are left out.
    fn search_args(&self, kind: &str, filter: &str, limit: Option<u32>) -> Vec<String> {
        let limit = limit.unwrap_or(30).clamp(1, MAX_LIMIT).to_string();
        let mut args: Vec<String> = [
            "search",
            kind,
            filter,
            "--state=open",
            "--archived=false",
            "--json",
            SEARCH_FIELDS,
            "--limit",
            &limit,
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        args.extend(self.owners.iter().map(|o| format!("--owner={o}")));
        args
    }

    async fn search(
        &self,
        kind: &str,
        filter: &str,
        limit: Option<u32>,
    ) -> Result<String, ErrorData> {
        let args = self.search_args(kind, filter, limit);
        run_gh(&args.iter().map(String::as_str).collect::<Vec<_>>()).await
    }

    fn check_repo(&self, repo: &str) -> Result<(), ErrorData> {
        let valid = repo.split('/').count() == 2
            && repo.split('/').all(|p| {
                !p.is_empty()
                    && !p.starts_with('-')
                    && p.chars()
                        .all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            });
        if !valid {
            return Err(ErrorData::invalid_params(
                "repo must look like \"owner/name\"",
                None,
            ));
        }
        let owner = repo.split('/').next().unwrap_or_default().to_lowercase();
        if !self.owners.is_empty() && !self.owners.contains(&owner) {
            return Err(ErrorData::invalid_params(
                format!("access denied: owner \"{owner}\" is not in the allowed list"),
                None,
            ));
        }
        Ok(())
    }
}

#[tool_router]
impl GithubTools {
    #[tool(
        description = "List open GitHub issues assigned to the user. Repositories that are archived are left out."
    )]
    async fn my_issues(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_text(self.search("issues", "--assignee=@me", a.limit).await?)
    }

    #[tool(
        description = "List open pull requests where the user's review is requested. Repositories that are archived are left out."
    )]
    async fn review_requests(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_text(
            self.search("prs", "--review-requested=@me", a.limit)
                .await?,
        )
    }

    #[tool(
        description = "List open pull requests that the user authored. Repositories that are archived are left out."
    )]
    async fn my_pull_requests(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        ok_text(self.search("prs", "--author=@me", a.limit).await?)
    }

    #[tool(
        description = "Search the user's GitHub issues and pull requests (assigned, authored, or review requested) by words in the title, the repository, and the description, best match first. It searches the list that AIfred saved, and downloads a new list first when it is too old. Each result says if it is still in the user's list (in_list) and when AIfred last saw it (last_seen, Unix seconds). A result can have `related`: the items linked to it (parent, subtask, same mission, closes, closed by), each with its key, title, place, and why. Read them to see the work around a result."
    )]
    async fn search_my_items(
        &self,
        Parameters(a): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let note = self.refresh().await;
        let hits = self
            .store()
            // The owner limit is in the search, so other owners do not use up results.
            .search(
                &a.query,
                &self.owners,
                a.include_old.unwrap_or(false),
                a.limit,
            )
            .map_err(|e| ErrorData::internal_error(format!("{e:#}"), None))?;
        let items: Vec<Value> = hits.into_iter().map(|h| h.into_json()).collect();
        let mut out = json!({ "items": items });
        if let Some(note) = note {
            out["note"] = note.into();
        }
        ok_text(out.to_string())
    }

    #[tool(
        description = "Get one issue or pull request: title, state, author, labels, the body (shortened), and the last 5 comments (shortened). Use it only for the items that you must read in full: the overview tool already has a short summary of each item."
    )]
    async fn item_details(
        &self,
        Parameters(a): Parameters<DetailArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        self.check_repo(&a.repo)?;
        let number = a.number.to_string();
        // `gh issue view` also resolves pull request numbers.
        let raw = run_gh(&[
            "issue",
            "view",
            &number,
            "-R",
            &a.repo,
            "--json",
            "title,state,author,labels,body,comments,updatedAt,url",
        ])
        .await?;
        let short = shorten_details(&raw);
        // Save the full description, so search_my_items can find words in it.
        // It is the same text as the list download saves, so the summary of
        // the item (summaries.rs) does not have to be made again.
        if let Some(body) = serde_json::from_str::<Value>(&raw)
            .ok()
            .and_then(|v| v["body"].as_str().map(String::from))
            && let Err(e) = self
                .store()
                .save_details(&format!("{}#{}", a.repo, a.number), &body)
        {
            eprintln!("aifred-github-mcp: cannot save the description: {e:#}");
        }
        ok_text(short)
    }
}

#[tool_handler]
impl ServerHandler for GithubTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

async fn run_gh(args: &[&str]) -> Result<String, ErrorData> {
    let out = Command::new("gh")
        .args(args)
        .output()
        .await
        .map_err(|e| ErrorData::internal_error(format!("cannot run gh: {e}"), None))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(ErrorData::internal_error(
            format!("gh failed: {}", err.trim()),
            None,
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Keeps tool output small. GitHub sends ids, reactions, and other data
/// that the AI does not need, and a long comment history: one pull request
/// was 23,000 characters. This keeps the useful fields, a shortened body, and
/// the last MAX_COMMENTS comments, each one shortened.
fn shorten_details(raw: &str) -> String {
    let Ok(v) = serde_json::from_str::<Value>(raw) else {
        return cut(raw, MAX_BODY_CHARS);
    };
    let comments: Vec<Value> = v["comments"]
        .as_array()
        .map(|all| {
            let skip = all.len().saturating_sub(MAX_COMMENTS);
            all[skip..]
                .iter()
                .map(|c| {
                    json!({
                        "by": c["author"]["login"],
                        "at": c["createdAt"],
                        "text": cut(c["body"].as_str().unwrap_or_default(), MAX_COMMENT_CHARS),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    let total = v["comments"].as_array().map_or(0, Vec::len);
    let mut out = json!({
        "title": v["title"],
        "state": v["state"],
        "url": v["url"],
        "author": v["author"]["login"],
        "updated": v["updatedAt"],
        "labels": v["labels"].as_array().map(|l| l.iter().map(|x| x["name"].clone()).collect::<Vec<_>>()),
        "body": cut(v["body"].as_str().unwrap_or_default(), MAX_BODY_CHARS),
        "last_comments": comments,
    });
    if total > MAX_COMMENTS {
        out["older_comments_not_shown"] = json!(total - MAX_COMMENTS);
    }
    out.to_string()
}

fn ok_text(s: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(s)]))
}

/// Serves these tools over stdio until Claude Code closes the connection.
pub async fn serve() -> anyhow::Result<()> {
    let server = GithubTools::from_env()?.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use aifred_core::files::now;
    use aifred_core::github;

    #[test]
    fn details_keep_only_the_useful_fields_and_the_last_comments() {
        let comment = |n: usize| {
            json!({ "author": { "login": format!("u{n}"), "id": "MDQ6" }, "createdAt": "2026-11-01", "body": "x".repeat(3000),
                    "reactionGroups": [], "id": "IC_1", "url": "u", "authorAssociation": "MEMBER" })
        };
        let raw = json!({
            "title": "Fix cart", "state": "OPEN", "url": "u", "updatedAt": "2026-11-02",
            "author": { "login": "okky", "id": "MDQ6", "is_bot": false, "name": "Okky" },
            "labels": [{ "name": "bug", "color": "d73a4a", "description": "Something is broken", "id": "LA_1" }],
            "body": "b".repeat(5000),
            "comments": (0..8).map(comment).collect::<Vec<_>>(),
        });
        let short: Value = serde_json::from_str(&shorten_details(&raw.to_string())).unwrap();
        assert_eq!(short["author"], "okky");
        assert_eq!(short["labels"], json!(["bug"]));
        let comments = short["last_comments"].as_array().unwrap();
        assert_eq!(comments.len(), 5);
        assert_eq!(comments[0]["by"], "u3", "the oldest comments are left out");
        assert_eq!(short["older_comments_not_shown"], 3);
        assert!(comments[0]["text"].as_str().unwrap().len() < 1600);
        assert!(
            short.to_string().len() < 13_000,
            "got {} characters",
            short.to_string().len()
        );
    }

    #[test]
    fn searches_skip_archived_repos_and_keep_the_owner_limit() {
        let t = GithubTools {
            owners: vec!["acme".into()],
            dir: std::env::temp_dir(),
        };
        let args = t.search_args("prs", "--author=@me", Some(500));
        assert!(
            args.contains(&"--archived=false".to_string()),
            "got {args:?}"
        );
        assert!(args.contains(&"--state=open".to_string()));
        assert!(args.contains(&MAX_LIMIT.to_string()), "limit is capped");
        assert_eq!(args.last().unwrap(), "--owner=acme");
    }

    #[tokio::test]
    async fn search_uses_a_fresh_saved_list_and_keeps_the_owner_limit() {
        let dir =
            std::env::temp_dir().join(format!("aifred-github-mcp-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let item = |repo: &str, n: u64| -> github::GithubItem {
            serde_json::from_value(json!({
                "id": format!("{repo}#{n}"), "repo": repo, "number": n, "title": "Fix the checkout button",
                "url": "u", "reason": "assigned", "updated": "2026-10-01T10:00:00Z"
            }))
            .unwrap()
        };
        // Saved a minute ago: fresh, so no `gh` call.
        GithubStore::new(&dir)
            .save_fetch(
                vec![item("acme/shop", 1), item("other/x", 2)],
                &[],
                now() - 60,
            )
            .unwrap();
        let t = GithubTools {
            owners: vec!["acme".into()],
            dir: dir.clone(),
        };
        let args = Parameters(SearchArgs {
            query: "checkout".into(),
            include_old: None,
            limit: None,
        });
        let r = t.search_my_items(args).await.unwrap();
        let text = serde_json::to_value(&r.content[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        let v: Value = serde_json::from_str(&text).unwrap();
        let items = v["items"].as_array().unwrap();
        assert_eq!(items.len(), 1, "the item of another owner is left out: {v}");
        assert_eq!(items[0]["repo"], "acme/shop");
        assert_eq!(items[0]["in_list"], true);
        assert!(v.get("note").is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
