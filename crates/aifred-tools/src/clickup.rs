//! Read-only ClickUp tools for aifred, served over MCP (stdio).
//!
//! The server reads the ClickUp token from the aifred data folder
//! ($AIFRED_HOME, or ~/.aifred), or from AIFRED_CLICKUP_TOKEN. The token never
//! goes into the tool settings file that Claude reads.
//!
//! These tools only read. The user marks ClickUp tasks done in the window.
//!
//! my_tasks uses the list that the window saved in the database while it is
//! new enough, and saves each new list it gets from ClickUp. search_my_tasks
//! searches that saved list. task_details saves the description for search.

use aifred_core::clickup::{self, Client};
use aifred_core::clickup_list::{ClickupList, ClickupStore};
use aifred_core::files::now;
use aifred_core::{default_data_dir, settings};
use aifred_core::text::cut;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_TEXT_CHARS: usize = 4000;
const MAX_COMMENTS: usize = 5;

#[derive(Clone)]
struct ClickupTools {
    dir: std::path::PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Maximum number of tasks to return (1-200). Default 50.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Words to find in the task name, the workspace and list, and the saved description. Example: "invoice client".
    query: String,
    /// Also include tasks that are no longer in the user's list, for example done tasks. Default false.
    include_old: Option<bool>,
    /// Maximum number of tasks to return (1-50). Default 10.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct DetailArgs {
    /// ClickUp task id from my_tasks (for example "86abc123"), or the team's custom id (for example "DEV-42").
    id: String,
}

fn fail(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn ok_json(v: &Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(v.to_string())]))
}

impl ClickupTools {
    fn token(&self) -> Result<String, ErrorData> {
        clickup::token(&self.dir).ok_or_else(|| {
            ErrorData::invalid_request(
                "ClickUp is not connected. The user can add a ClickUp token on the AIfred Settings page.",
                None,
            )
        })
    }

    fn client(&self) -> Result<Client, ErrorData> {
        self.token().map(|t| Client::new(&t))
    }

    /// The saved list while it is new enough, or else a new list from ClickUp.
    async fn list(&self) -> Result<ClickupList, ErrorData> {
        let token = self.token()?;
        let account = clickup::account_key(&token);
        let store = ClickupStore::new(&self.dir);
        // A broken saved file is not an error here: ask ClickUp.
        if let Ok(Some(list)) = store.cached(&account, settings::tool_max_age_secs(&self.dir), now()) {
            return Ok(list);
        }
        let fetched = Client::new(&token).my_tasks().await.map_err(fail)?;
        match store.save(&account, fetched.clone(), now()) {
            Ok(list) => Ok(list),
            Err(e) => {
                // The list is still good. Only the next call cannot use it.
                eprintln!("aifred-clickup-mcp: cannot save the ClickUp list: {e:#}");
                let mut items = fetched.items;
                items.sort_by_key(|i| std::cmp::Reverse(i.updated));
                Ok(ClickupList { items, fetched_at: now(), warning: clickup::cut_warning(&fetched.cut) })
            }
        }
    }
}

#[tool_router]
impl ClickupTools {
    #[tool(description = "List the user's open ClickUp tasks (assigned to them, in all workspaces), newest change first. fetched_at is when the list came from ClickUp (Unix seconds). A note says when the list is not complete.")]
    async fn my_tasks(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        let list = self.list().await?;
        let mut items = list.items;
        items.truncate(a.limit.unwrap_or(50).clamp(1, 200) as usize);
        let mut out = json!({ "tasks": items, "fetched_at": list.fetched_at });
        if let Some(note) = list.warning {
            out["note"] = json!(note);
        }
        ok_json(&out)
    }

    #[tool(description = "Search the user's ClickUp tasks by words in the name, the workspace and list, and the description, best match first. It searches the list that AIfred saved, and downloads a new list first when it is too old. Each result says if it is still in the user's open list (in_list) and when AIfred last saw it (last_seen, Unix seconds).")]
    async fn search_my_tasks(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, ErrorData> {
        let account = clickup::account_key(&self.token()?);
        // list() downloads a new list when the saved one is too old.
        let note = match self.list().await {
            Ok(list) => list.warning,
            Err(e) => Some(format!("Cannot reach ClickUp, so the results come from the saved list. {}", e.message)),
        };
        let hits = ClickupStore::new(&self.dir)
            .search(&account, &a.query, a.include_old.unwrap_or(false), a.limit)
            .map_err(fail)?;
        let tasks: Vec<Value> = hits.into_iter().map(|h| h.into_json()).collect();
        let mut out = json!({ "tasks": tasks });
        if let Some(note) = note {
            out["note"] = json!(note);
        }
        ok_json(&out)
    }

    #[tool(description = "Get one ClickUp task: name, status, description (shortened), assignees, dates, and the last comments.")]
    async fn task_details(&self, Parameters(a): Parameters<DetailArgs>) -> Result<CallToolResult, ErrorData> {
        clickup::check_task_id(&a.id).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        let (task, comments) = self.client()?.task_with_comments(&a.id).await.map_err(fail)?;
        let short = shorten(&task, &comments);
        // Save the full description, so search_my_tasks can find words in it.
        // It is the same text as the list download saves (description_text),
        // so the summary of the task does not have to be made again.
        // The id from ClickUp, because `a.id` can be the team's custom id.
        let text = clickup::description_text(task["text_content"].as_str(), task["description"].as_str());
        if let Some(id) = task["id"].as_str()
            && let Err(e) = ClickupStore::new(&self.dir).save_details(id, &text)
        {
            eprintln!("aifred-clickup-mcp: cannot save the description: {e:#}");
        }
        ok_json(&short)
    }
}

#[tool_handler]
impl ServerHandler for ClickupTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Keeps the tool output small: only the useful task fields, a shortened
/// description, and the last comments as plain text.
fn shorten(task: &Value, comments: &Value) -> Value {
    let names = |v: &Value| -> Vec<Value> {
        v.as_array().map(|a| a.iter().map(|u| u["username"].clone()).collect()).unwrap_or_default()
    };
    let text = clickup::description_text(task["text_content"].as_str(), task["description"].as_str());
    let mut last: Vec<Value> = comments["comments"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| json!({ "by": c["user"]["username"], "date": c["date"], "text": cut(c["comment_text"].as_str().unwrap_or_default(), MAX_TEXT_CHARS) }))
                .collect()
        })
        .unwrap_or_default();
    // ClickUp sends the newest comment first.
    last.truncate(MAX_COMMENTS);
    json!({
        "id": task["id"],
        "custom_id": task["custom_id"],
        "name": task["name"],
        "url": task["url"],
        "status": task["status"]["status"],
        "list": task["list"]["name"],
        "priority": task["priority"]["priority"],
        "due_date": task["due_date"],
        "date_updated": task["date_updated"],
        "assignees": names(&task["assignees"]),
        "tags": task["tags"].as_array().map(|t| t.iter().map(|x| x["name"].clone()).collect::<Vec<_>>()),
        "description": cut(&text, MAX_TEXT_CHARS),
        "last_comments": last,
    })
}

/// Serves these tools over stdio until Claude Code closes the connection.
pub async fn serve() -> anyhow::Result<()> {
    let server = ClickupTools { dir: default_data_dir()? }.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_keeps_the_useful_fields_and_cuts_long_text() {
        let task = json!({
            "id": "86abc", "custom_id": "DEV-42", "name": "Write notes", "url": "u",
            "status": {"status": "in progress"}, "list": {"name": "Sprint"},
            "priority": {"priority": "high"}, "assignees": [{"username": "okky"}],
            "text_content": "x".repeat(MAX_TEXT_CHARS + 10), "custom_fields": [{"big": "noise"}]
        });
        let comments = json!({"comments": (0..8).map(|i| json!({"comment_text": format!("c{i}"), "user": {"username": "sam"}, "date": "1"})).collect::<Vec<_>>()});
        let s = shorten(&task, &comments);
        assert_eq!(s["status"], "in progress");
        assert_eq!(s["assignees"], json!(["okky"]));
        assert!(s["description"].as_str().unwrap().ends_with("[... shortened]"));
        assert_eq!(s["last_comments"].as_array().unwrap().len(), MAX_COMMENTS);
        assert_eq!(s["last_comments"][0]["text"], "c0", "the newest comment stays");
        assert!(s.get("custom_fields").is_none());
    }

    #[tokio::test]
    async fn tools_without_a_token_say_how_to_connect() {
        let dir = std::env::temp_dir().join(format!("aifred-clickup-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = ClickupTools { dir: dir.clone() };
        // Only when the developer has no token in the environment.
        if std::env::var_os("AIFRED_CLICKUP_TOKEN").is_none() {
            let e = t.my_tasks(Parameters(ListArgs { limit: None })).await.unwrap_err();
            assert!(e.message.contains("Settings page"), "{e:?}");
        }
        let e = t.task_details(Parameters(DetailArgs { id: "../user".into() })).await.unwrap_err();
        assert!(e.message.contains("bad ClickUp task id"), "{e:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn my_tasks_uses_the_saved_list_while_it_is_new() {
        let dir = std::env::temp_dir().join(format!("aifred-clickup-mcp-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        clickup::save_token(&dir, "pk_test").unwrap();
        // The token in use: the file, or AIFRED_CLICKUP_TOKEN when it is set.
        let account = clickup::account_key(&clickup::token(&dir).unwrap());
        let item: clickup::ClickupItem = serde_json::from_value(json!({
            "id": "86abc", "title": "Write notes", "url": "u", "status": "to do",
            "list": "Sprint", "workspace": "Rymera", "updated": 1
        }))
        .unwrap();
        let fetched = clickup::MyTasks { items: vec![item], cut: vec!["Big".into()] };
        ClickupStore::new(&dir).save(&account, fetched, now() - 60).unwrap();
        // No ClickUp call: the fake token would fail if ClickUp were asked.
        let t = ClickupTools { dir: dir.clone() };
        let r = t.my_tasks(Parameters(ListArgs { limit: None })).await.unwrap();
        let text = serde_json::to_value(&r.content[0]).unwrap()["text"].as_str().unwrap().to_string();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["tasks"][0]["title"], "Write notes");
        assert!(v["note"].as_str().unwrap().contains("Big"), "the chat hears that the list is cut: {v}");

        // The search uses the same saved list, also without a ClickUp call.
        let args = Parameters(SearchArgs { query: "notes sprint".into(), include_old: None, limit: None });
        let r = t.search_my_tasks(args).await.unwrap();
        let text = serde_json::to_value(&r.content[0]).unwrap()["text"].as_str().unwrap().to_string();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["tasks"][0]["title"], "Write notes");
        assert_eq!(v["tasks"][0]["in_list"], true);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
