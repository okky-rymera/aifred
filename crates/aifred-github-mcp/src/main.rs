//! Read-only GitHub tools for aifred, served over MCP (stdio).
//!
//! The server calls the `gh` CLI, so it uses the user's existing `gh` login.
//! It never receives or stores a token itself.
//!
//! Access limit: set `AIFRED_GITHUB_OWNERS` to a comma-separated list of
//! users or orgs (for example `Rymera-Web-Co,okky-rymera`). Searches are then
//! limited to those owners, and detail lookups outside them are refused.

use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use tokio::process::Command;

const SEARCH_FIELDS: &str = "repository,number,title,updatedAt,url,labels";
const MAX_LIMIT: u32 = 100;
const MAX_BODY_CHARS: usize = 4000;

#[derive(Clone)]
struct GithubTools {
    /// Lowercase owner names. Empty means no owner limit.
    owners: Vec<String>,
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
    fn from_env() -> Self {
        let owners = std::env::var("AIFRED_GITHUB_OWNERS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_lowercase())
            .filter(|s| !s.is_empty())
            .collect();
        Self { owners }
    }

    /// Runs one `gh search` query (`issues` or `prs`) with the owner limit applied.
    async fn search(&self, kind: &str, filter: &str, limit: Option<u32>) -> Result<String, ErrorData> {
        let limit = limit.unwrap_or(30).clamp(1, MAX_LIMIT).to_string();
        let mut args = vec!["search", kind, filter, "--state=open", "--json", SEARCH_FIELDS, "--limit", &limit];
        let owner_args: Vec<String> = self.owners.iter().map(|o| format!("--owner={o}")).collect();
        args.extend(owner_args.iter().map(String::as_str));
        run_gh(&args).await
    }

    fn check_repo(&self, repo: &str) -> Result<(), ErrorData> {
        let valid = repo.split('/').count() == 2
            && repo.split('/').all(|p| {
                !p.is_empty() && !p.starts_with('-') && p.chars().all(|c| c.is_ascii_alphanumeric() || "-_.".contains(c))
            });
        if !valid {
            return Err(ErrorData::invalid_params("repo must look like \"owner/name\"", None));
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
    #[tool(description = "List open GitHub issues assigned to the user.")]
    async fn my_issues(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        ok_text(self.search("issues", "--assignee=@me", a.limit).await?)
    }

    #[tool(description = "List open pull requests where the user's review is requested.")]
    async fn review_requests(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        ok_text(self.search("prs", "--review-requested=@me", a.limit).await?)
    }

    #[tool(description = "List open pull requests that the user authored.")]
    async fn my_pull_requests(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        ok_text(self.search("prs", "--author=@me", a.limit).await?)
    }

    #[tool(description = "Get one issue or pull request: title, state, body (shortened), labels, and the last comments.")]
    async fn item_details(&self, Parameters(a): Parameters<DetailArgs>) -> Result<CallToolResult, ErrorData> {
        self.check_repo(&a.repo)?;
        let number = a.number.to_string();
        // `gh issue view` also resolves pull request numbers.
        let raw = run_gh(&[
            "issue", "view", &number, "-R", &a.repo, "--json",
            "title,state,author,labels,body,comments,updatedAt,url",
        ])
        .await?;
        ok_text(shorten_details(&raw))
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
        return Err(ErrorData::internal_error(format!("gh failed: {}", err.trim()), None));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Keeps tool output small: cuts the body and keeps only the last 5 comments.
fn shorten_details(raw: &str) -> String {
    let Ok(mut v) = serde_json::from_str::<serde_json::Value>(raw) else {
        return raw.chars().take(MAX_BODY_CHARS).collect();
    };
    if let Some(body) = v.get("body").and_then(|b| b.as_str()) {
        if body.chars().count() > MAX_BODY_CHARS {
            let cut: String = body.chars().take(MAX_BODY_CHARS).collect();
            v["body"] = format!("{cut}\n[... shortened]").into();
        }
    }
    if let Some(comments) = v.get_mut("comments").and_then(|c| c.as_array_mut()) {
        let skip = comments.len().saturating_sub(5);
        *comments = comments.drain(skip..).collect();
    }
    v.to_string()
}

fn ok_text(s: String) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(s)]))
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let server = GithubTools::from_env().serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}
