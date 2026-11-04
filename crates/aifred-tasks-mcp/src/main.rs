//! Local task tools for aifred, served over MCP (stdio).
//!
//! These tools only change the tasks in the aifred database
//! ($AIFRED_HOME/aifred.db, or ~/.aifred/aifred.db). They cannot reach any other file.
//!
//! The overview tool also reads the saved GitHub and ClickUp lists in the same
//! database. It downloads a new list first when the saved one is too old.

use aifred_core::tasks::{StepFields, Task, TaskFields, TaskStore, clean_tag};
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
    /// Only tasks with this tag. Capital letters do not matter. Example: "work".
    tag: Option<String>,
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
    /// Optional extra detail, such as a link.
    notes: Option<String>,
    /// The day the task must be done, "YYYY-MM-DD". Only when the user gives a day.
    due: Option<String>,
    /// The first day to work on the task, "YYYY-MM-DD". Before it, the task waits. Only when the user gives a day.
    start: Option<String>,
    /// "urgent", "high", "normal", or "low". Only when the user says how important it is.
    priority: Option<String>,
    /// Short labels. Example: ["work", "travel"].
    tags: Option<Vec<String>>,
    /// The steps of the task, one text for each step. Example: ["Pick dates", "Pay"].
    checklist: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
struct CheckArg {
    /// The text of the step.
    text: String,
    /// True when the step is done. Leave out to keep the done value of the
    /// old step with the same text (a new step is not done).
    done: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateArgs {
    /// Task id, from list_tasks, search_tasks, or overview.
    id: String,
    /// A new title. Leave out to keep it.
    title: Option<String>,
    /// A new description. Leave out to keep it.
    notes: Option<String>,
    /// A new due day, "YYYY-MM-DD". "" removes it. Leave out to keep it.
    due: Option<String>,
    /// A new start day, "YYYY-MM-DD". "" removes it. Leave out to keep it.
    start: Option<String>,
    /// "urgent", "high", "normal", or "low". "" removes it. Leave out to keep it.
    priority: Option<String>,
    /// The full new list of tags. It replaces the old list. Leave out to keep it.
    tags: Option<Vec<String>>,
    /// The full new list of steps. It replaces the old list. Leave out to keep it.
    checklist: Option<Vec<CheckArg>>,
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

    #[tool(description = "List the user's local to-do tasks (not GitHub or ClickUp items). Each task can have a due day, a start day (the task waits until then), a priority, tags, and a checklist of steps.")]
    async fn list_tasks(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        let all = self.store().load().map_err(fail)?;
        let keep_done = a.include_done.unwrap_or(false);
        // Clean the tag the same way as when tags are saved.
        let tag = a.tag.map(|t| clean_tag(&t).to_lowercase());
        let has_tag = |t: &Task| tag.as_ref().is_none_or(|want| t.tags.iter().any(|x| x.to_lowercase() == *want));
        json(&all.into_iter().filter(|t| (keep_done || !t.done) && has_tag(t)).collect::<Vec<_>>())
    }

    #[tool(description = "Search the user's local to-do tasks by words in the title and notes, best match first. A task matches when it has any of the words, also other forms of a word (\"invoices\" finds \"invoice\").")]
    async fn search_tasks(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, ErrorData> {
        json(&self.store().search(&a.query, a.include_done.unwrap_or(false), a.limit).map_err(fail)?)
    }

    #[tool(description = "Add a task to the user's local to-do list. Use it when the user asks you to note, remember to do, or track something. Set due, start, priority, and tags only when the user gives them. Write each day as YYYY-MM-DD: work it out from today's date.")]
    async fn add_task(&self, Parameters(a): Parameters<AddArgs>) -> Result<CallToolResult, ErrorData> {
        let fields = TaskFields {
            title: Some(a.title),
            notes: a.notes,
            due: a.due,
            start: a.start,
            priority: a.priority,
            tags: a.tags,
            checklist: a.checklist.map(|l| l.into_iter().map(|text| StepFields { text, done: Some(false) }).collect()),
        };
        json(&self.store().create(fields, "aifred").map_err(fail)?)
    }

    #[tool(description = "Change a local task: its title, description, due day, start day, priority, tags, or checklist. Only the fields you send change. Use it only when the user asks for the change. To tick a step, send the full checklist with that step done. To add or remove steps, send the full checklist with no done values: the steps keep their ticks.")]
    async fn update_task(&self, Parameters(a): Parameters<UpdateArgs>) -> Result<CallToolResult, ErrorData> {
        let fields = TaskFields {
            title: a.title,
            notes: a.notes,
            due: a.due,
            start: a.start,
            priority: a.priority,
            tags: a.tags,
            checklist: a.checklist.map(|l| l.into_iter().map(|c| StepFields { text: c.text, done: c.done }).collect()),
        };
        json(&self.store().update(&a.id, fields).map_err(fail)?)
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

    fn add(title: &str, notes: Option<&str>) -> AddArgs {
        AddArgs { title: title.into(), notes: notes.map(String::from), due: None, start: None, priority: None, tags: None, checklist: None }
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
            .add_task(Parameters(add("Send invoice", Some("to Sam"))))
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

        let open = body(t.list_tasks(Parameters(ListArgs { include_done: None, tag: None })).await.unwrap());
        assert_eq!(open.as_array().unwrap().len(), 1);
        assert_eq!(open[0]["id"], a.id.as_str());

        let all = body(t.list_tasks(Parameters(ListArgs { include_done: Some(true), tag: None })).await.unwrap());
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
        assert!(t.add_task(Parameters(add("  ", None))).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn add_task_sets_the_extra_fields() {
        let t = tools("extra");
        let args = AddArgs {
            due: Some("2026-12-20".into()),
            start: Some("2026-12-01".into()),
            priority: Some("high".into()),
            tags: Some(vec!["travel".into()]),
            checklist: Some(vec!["Pick dates".into(), "Pay".into()]),
            ..add("Book flights", None)
        };
        let task = body(t.add_task(Parameters(args)).await.unwrap());
        assert_eq!((&task["due"], &task["start"], &task["priority"]), (&"2026-12-20".into(), &"2026-12-01".into(), &"high".into()));
        assert_eq!(task["tags"], serde_json::json!(["travel"]));
        assert_eq!(task["checklist"], serde_json::json!([{ "text": "Pick dates", "done": false }, { "text": "Pay", "done": false }]));
        let bad = AddArgs { due: Some("next Friday".into()), ..add("Book flights", None) };
        let e = t.add_task(Parameters(bad)).await.unwrap_err();
        assert!(e.message.contains("YYYY-MM-DD"), "the AI learns the right form: {}", e.message);
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn update_task_changes_only_the_sent_fields() {
        let t = tools("update");
        let a = t.store().add("Pay rent", "Before Friday", "you").unwrap();
        let args = UpdateArgs {
            id: a.id.clone(),
            title: None,
            notes: None,
            due: Some("2026-12-01".into()),
            start: None,
            priority: Some("urgent".into()),
            tags: None,
            checklist: Some(vec![CheckArg { text: "Ask bank".into(), done: Some(true) }]),
        };
        let task = body(t.update_task(Parameters(args)).await.unwrap());
        assert_eq!((&task["title"], &task["notes"], &task["due"], &task["priority"]), (&"Pay rent".into(), &"Before Friday".into(), &"2026-12-01".into(), &"urgent".into()));
        assert_eq!(task["checklist"][0]["done"], true);
        // The AI adds a step and sends no done values: the ticked step stays ticked.
        let steps = vec![CheckArg { text: "Ask bank".into(), done: None }, CheckArg { text: "Pay".into(), done: None }];
        let more = UpdateArgs { id: a.id.clone(), title: None, notes: None, due: None, start: None, priority: None, tags: None, checklist: Some(steps) };
        let task = body(t.update_task(Parameters(more)).await.unwrap());
        assert_eq!(task["checklist"], serde_json::json!([{ "text": "Ask bank", "done": true }, { "text": "Pay", "done": false }]));
        let none = UpdateArgs { id: "nope".into(), title: None, notes: None, due: None, start: None, priority: None, tags: None, checklist: None };
        assert!(t.update_task(Parameters(none)).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn list_tasks_can_keep_one_tag() {
        let t = tools("tag");
        let s = t.store();
        let a = s.create(TaskFields { title: Some("Book flights".into()), tags: Some(vec!["Travel".into()]), ..Default::default() }, "you").unwrap();
        s.add("Buy milk", "", "you").unwrap();
        let found = body(t.list_tasks(Parameters(ListArgs { include_done: None, tag: Some(" # travel".into()) })).await.unwrap());
        assert_eq!(found.as_array().unwrap().len(), 1);
        assert_eq!(found[0]["id"], a.id.as_str());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }
}
