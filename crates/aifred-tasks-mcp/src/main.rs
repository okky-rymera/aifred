//! Local task tools for aifred, served over MCP (stdio).
//!
//! These tools only change the tasks in the aifred database
//! ($AIFRED_HOME/aifred.db, or ~/.aifred/aifred.db). They cannot reach any other file.
//!
//! The overview tool also reads the saved GitHub and ClickUp lists in the same
//! database. It downloads a new list first when the saved one is too old.

use aifred_core::tasks::TaskStore;
use aifred_core::{clickup, default_data_dir, github_list::GithubStore, overview};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;

#[derive(Clone)]
struct TaskTools {
    dir: std::path::PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Also include tasks that are done. Default false.
    include_done: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Words to find in the task title and notes. Example: "invoice Sam".
    query: String,
    /// Also include tasks that are done. Default false.
    include_done: Option<bool>,
    /// Maximum number of tasks to return (1-50). Default 10.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    /// Short task title, written as an action. Example: "Reply to Sam about the invoice".
    title: String,
    /// Optional extra detail, such as a link or a due date.
    notes: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    /// Task id, from list_tasks.
    id: String,
}

fn fail(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn json<T: serde::Serialize>(v: &T) -> Result<CallToolResult, ErrorData> {
    let s = serde_json::to_string(v).map_err(|e| fail(e.into()))?;
    Ok(CallToolResult::success(vec![ContentBlock::text(s)]))
}

impl TaskTools {
    fn store(&self) -> TaskStore {
        TaskStore::new(&self.dir)
    }
}

#[tool_router]
impl TaskTools {
    #[tool(description = "Everything on the user's plate in one call: the open local to-do tasks, the open GitHub items (assigned issues, review requests, and the user's own pull requests), and the open ClickUp tasks. Each item has a short summary of its description. Use this first when the user asks what to do, what is open, or for a plan. Then use item_details or task_details only for the few items that need a closer look. fetched_at is when each list came from GitHub or ClickUp (Unix seconds).")]
    async fn overview(&self) -> Result<CallToolResult, ErrorData> {
        // Both downloads at the same time: each can take some seconds.
        let gh = GithubStore::new(&self.dir);
        let (gh_note, cu_note) = tokio::join!(gh.refresh_if_old(), aifred_core::clickup_list::refresh_if_old(&self.dir));
        let account = clickup::token(&self.dir).map(|t| clickup::account_key(&t));
        let mut out = overview::build(&self.dir, account.as_deref()).map_err(fail)?;
        let notes: Vec<String> = [gh_note, cu_note].into_iter().flatten().collect();
        if !notes.is_empty() {
            out["notes"] = notes.into();
        }
        json(&out)
    }

    #[tool(description = "List the user's local to-do tasks (not GitHub or ClickUp items).")]
    async fn list_tasks(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        let all = self.store().load().map_err(fail)?;
        let keep_done = a.include_done.unwrap_or(false);
        json(&all.into_iter().filter(|t| keep_done || !t.done).collect::<Vec<_>>())
    }

    #[tool(description = "Search the user's local to-do tasks by words in the title and notes, best match first. A task matches when it has any of the words, also other forms of a word (\"invoices\" finds \"invoice\").")]
    async fn search_tasks(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, ErrorData> {
        json(&self.store().search(&a.query, a.include_done.unwrap_or(false), a.limit).map_err(fail)?)
    }

    #[tool(description = "Add a task to the user's local to-do list. Use it when the user asks you to note, remember to do, or track something.")]
    async fn add_task(&self, Parameters(a): Parameters<AddArgs>) -> Result<CallToolResult, ErrorData> {
        json(&self.store().add(&a.title, a.notes.as_deref().unwrap_or(""), "aifred").map_err(fail)?)
    }

    #[tool(description = "Mark a local task as done. Only do this when the user says the task is done.")]
    async fn complete_task(&self, Parameters(a): Parameters<IdArgs>) -> Result<CallToolResult, ErrorData> {
        json(&self.store().set_done(&a.id, true).map_err(fail)?)
    }
}

#[tool_handler]
impl ServerHandler for TaskTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let dir = default_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let server = TaskTools { dir }.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(name: &str) -> TaskTools {
        let dir = std::env::temp_dir().join(format!("aifred-tasks-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TaskTools { dir }
    }

    /// The JSON text that the tool sends back to Claude.
    fn body(r: CallToolResult) -> serde_json::Value {
        let v = serde_json::to_value(r).unwrap();
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    #[tokio::test]
    async fn add_task_marks_it_as_added_by_aifred() {
        let t = tools("add");
        let r = t
            .add_task(Parameters(AddArgs { title: "Send invoice".into(), notes: Some("to Sam".into()) }))
            .await
            .unwrap();
        let task = body(r);
        assert_eq!(task["title"], "Send invoice");
        assert_eq!(task["notes"], "to Sam");
        assert_eq!(task["added_by"], "aifred");
        assert_eq!(t.store().load().unwrap().len(), 1, "the task is saved to the file");
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn list_hides_done_tasks_unless_asked() {
        let t = tools("list");
        let s = t.store();
        let a = s.add("Open one", "", "you").unwrap();
        let b = s.add("Done one", "", "you").unwrap();
        s.set_done(&b.id, true).unwrap();

        let open = body(t.list_tasks(Parameters(ListArgs { include_done: None })).await.unwrap());
        assert_eq!(open.as_array().unwrap().len(), 1);
        assert_eq!(open[0]["id"], a.id.as_str());

        let all = body(t.list_tasks(Parameters(ListArgs { include_done: Some(true) })).await.unwrap());
        assert_eq!(all.as_array().unwrap().len(), 2);
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn complete_task_marks_done_and_rejects_unknown_id() {
        let t = tools("complete");
        let a = t.store().add("Finish", "", "you").unwrap();
        let done = body(t.complete_task(Parameters(IdArgs { id: a.id.clone() })).await.unwrap());
        assert_eq!(done["done"], true);
        assert!(t.complete_task(Parameters(IdArgs { id: "nope".into() })).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn search_tasks_finds_words_and_hides_done_tasks() {
        let t = tools("search");
        let s = t.store();
        let a = s.add("Pay the invoice", "", "you").unwrap();
        s.add("Buy milk", "", "you").unwrap();
        let args = |done| Parameters(SearchArgs { query: "invoices".into(), include_done: done, limit: None });
        let found = body(t.search_tasks(args(None)).await.unwrap());
        assert_eq!(found.as_array().unwrap().len(), 1);
        assert_eq!(found[0]["id"], a.id.as_str());
        s.set_done(&a.id, true).unwrap();
        assert!(body(t.search_tasks(args(None)).await.unwrap()).as_array().unwrap().is_empty());
        assert_eq!(body(t.search_tasks(args(Some(true))).await.unwrap()).as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn add_task_rejects_empty_title() {
        let t = tools("empty");
        assert!(t.add_task(Parameters(AddArgs { title: "  ".into(), notes: None })).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }
}
