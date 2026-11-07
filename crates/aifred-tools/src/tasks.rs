//! Local task tools for aifred, served over MCP (stdio).
//!
//! These tools only change the tasks in the aifred database
//! ($AIFRED_HOME/aifred.db, or ~/.aifred/aifred.db). They cannot reach any other file.
//!
//! The overview tool also reads the saved GitHub and ClickUp lists in the same
//! database. It downloads a new list first when the saved one is too old.
//!
//! The goal tools change the user's goals (goals.rs) in the same database.
//!
//! The waiting tools change the user's "waiting for" list (waiting.rs) in
//! the same database. They cannot send or change email.

use aifred_core::tasks::{StepFields, Task, TaskFields, TaskStore, clean_tag};
use aifred_core::{
    clickup, default_data_dir, files::now, github_list::GithubStore, gmail, goals, overview, plan,
    settings, waiting,
};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router, transport::stdio,
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
    /// Ids of files that the user attached to this message, to keep on the task. The ids are in the file note of the message.
    files: Option<Vec<String>>,
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
struct DayArgs {
    /// The day, "YYYY-MM-DD". Leave out for today.
    day: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct PlanItemArg {
    /// The item key from the overview tool, for example "task:t123", "github:owner/repo#7", "clickup:86a", or "gmail:18c2a".
    key: String,
    /// How many minutes the work takes. Give it when you can guess it. It does not replace a time that the user set.
    minutes: Option<u32>,
    /// Why the item is at this place, at most 15 words.
    why: Option<String>,
    /// The id of the user's goal that the item serves, from the goals in overview. Leave out when it serves no goal.
    goal: Option<i64>,
}

#[derive(Deserialize, JsonSchema)]
struct GoalTextArgs {
    /// The goal in one short line, with a date when the user gives one. Example: "Ship StoreAgent 2.0 by 30 Nov".
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct GoalEditArgs {
    /// The goal id, from the goals in overview.
    id: i64,
    /// The new text of the goal, in one short line.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct GoalIdArgs {
    /// The goal id, from the goals in overview.
    id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct WaitingListArgs {
    /// Also include the closed items. Default false: the open items, and the items closed in the last 3 days.
    all: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct AddWaitingArgs {
    /// What the user waits for, in one short line. Example: "The signed contract". Leave out for an email: AIfred uses its subject.
    what: Option<String>,
    /// Who the user waits for. Example: "Ana Lee". Leave out for an email: AIfred uses the people it went to.
    who: Option<String>,
    /// Since when the user waits, "YYYY-MM-DD". Leave out for today, or for the day of the email.
    since: Option<String>,
    /// The day to check, "YYYY-MM-DD". On this day AIfred tells the user when the item is still open. Only when the user gives a day.
    follow_up: Option<String>,
    /// The thread of an email from no_reply_yet in the overview. The item then closes by itself when the person answers.
    thread: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateWaitingArgs {
    /// The item id, from waiting_list or overview.
    id: i64,
    /// A new text of what the user waits for. Leave out to keep it.
    what: Option<String>,
    /// A new person. Leave out to keep it.
    who: Option<String>,
    /// A new day to check, "YYYY-MM-DD". "" removes it. Leave out to keep it.
    follow_up: Option<String>,
    /// true closes the item (it came, or the user stopped waiting). false opens it again.
    done: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct SetPlanArgs {
    /// The day, "YYYY-MM-DD". Leave out for today.
    day: Option<String>,
    /// All items of the plan, in the order of the day: the first item first. It replaces the old plan.
    items: Vec<PlanItemArg>,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    /// Task id, from list_tasks.
    id: String,
}

/// The day of the plan tools when the AI gives none: the day of the plan
/// chat that started this server, else today.
fn default_day() -> String {
    std::env::var(aifred_core::assistant::PLAN_DAY_ENV)
        .ok()
        .filter(|d| !d.is_empty())
        .unwrap_or_else(plan::today)
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

    /// The plan of `day` for the AI: each item with its time guess, and
    /// the planned minutes next to the minutes of the work day.
    fn plan_view(&self, day: &str) -> anyhow::Result<serde_json::Value> {
        let s = settings::load_or_default(&self.dir);
        let guesses = plan::estimates(&self.dir)?;
        let p = plan::get(&self.dir, day)?;
        let items: Vec<serde_json::Value> = p
            .as_ref()
            .map(|p| &p.items[..])
            .unwrap_or_default()
            .iter()
            .map(|i| serde_json::json!({ "key": i.key, "why": i.why, "goal": i.goal, "minutes": guesses.get(&i.key).map(|g| g.minutes) }))
            .collect();
        let planned: u32 = items
            .iter()
            .map(|i| i["minutes"].as_u64().unwrap_or(30) as u32)
            .sum();
        let cal = aifred_core::calendar::day_for_ai(&self.dir, day, &s)?;
        Ok(serde_json::json!({
            "day": day,
            "work_day": settings::day_text(&s),
            "made_by": p.as_ref().map(|p| match p.mode.as_str() {
                "ai" => "AIfred",
                aifred_core::plan::REVIEW => "the reviews: items that the user moved to this day. The user did not plan the day yet",
                _ => "the user",
            }),
            "items": items,
            "planned_minutes": planned,
            "work_minutes": cal["free_minutes"],
            "meetings": cal["meetings"],
            "calendar": cal["calendar"],
            "note": "An item with no minutes counts as 30 minutes. work_minutes is the work day less lunch and the meetings. The app places the items between the meetings."
        }))
    }
}

#[tool_router]
impl TaskTools {
    #[tool(
        description = "Everything on the user's plate in one call: the user's goals, the open tasks (doer \"you\": the user's local to-do tasks; doer \"aifred\": the reminders and checks that AIfred runs by itself, with no key; status \"paused\" means that AIfred does not run it now), the open GitHub items (assigned issues, review requests, and the user's own pull requests), the open ClickUp tasks, the calendar events of the next 7 days when the calendar is connected, the emails that wait for the user's answer and the emails where the user waits for an answer when Gmail is connected, and the user's waiting-for list. Each item has a short summary of its description. Use this first when the user asks what to do, what is open, or for a plan. Then use item_details or task_details only for the few items that need a closer look. The goals (at most 5) say what matters most to the user now: an item that serves a goal is more important. fetched_at is when each list came from GitHub or ClickUp (Unix seconds). review_picks are the user's picks in the weekly review: an item with drop or delegate does not need the user; an item with plan waits until plan_day."
    )]
    async fn overview(&self) -> Result<CallToolResult, ErrorData> {
        // Both downloads at the same time: each can take some seconds.
        let gh = GithubStore::new(&self.dir);
        let (gh_note, cu_note, cal_note, mail_note) = tokio::join!(
            gh.refresh_if_old(),
            aifred_core::clickup_list::refresh_if_old(&self.dir),
            aifred_core::calendar::refresh_if_old(&self.dir),
            gmail::refresh_if_old(&self.dir, settings::tool_max_age_secs(&self.dir))
        );
        let account = clickup::token(&self.dir).map(|t| clickup::account_key(&t));
        let mut out = overview::build(&self.dir, account.as_deref()).map_err(fail)?;
        let notes: Vec<String> = [gh_note, cu_note, cal_note, mail_note]
            .into_iter()
            .flatten()
            .collect();
        if !notes.is_empty() {
            out["notes"] = notes.into();
        }
        json(&out)
    }

    #[tool(
        description = "List the user's local to-do tasks (not GitHub or ClickUp items). Each task can have a due day, a start day (the task waits until then), a priority, tags, and a checklist of steps."
    )]
    async fn list_tasks(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let all = self.store().load().map_err(fail)?;
        let keep_done = a.include_done.unwrap_or(false);
        // Clean the tag the same way as when tags are saved.
        let tag = a.tag.map(|t| clean_tag(&t).to_lowercase());
        let has_tag = |t: &Task| {
            tag.as_ref()
                .is_none_or(|want| t.tags.iter().any(|x| x.to_lowercase() == *want))
        };
        json(
            &all.into_iter()
                .filter(|t| (keep_done || !t.done) && has_tag(t))
                .collect::<Vec<_>>(),
        )
    }

    #[tool(
        description = "Search the user's local to-do tasks by words in the title and notes, best match first. A task matches when it has any of the words, also other forms of a word (\"invoices\" finds \"invoice\")."
    )]
    async fn search_tasks(
        &self,
        Parameters(a): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        json(
            &self
                .store()
                .search(&a.query, a.include_done.unwrap_or(false), a.limit)
                .map_err(fail)?,
        )
    }

    #[tool(
        description = "Add a task to the user's local to-do list. Use it when the user asks you to note, remember to do, or track something. Set due, start, priority, and tags only when the user gives them. Write each day as YYYY-MM-DD: work it out from today's date."
    )]
    async fn add_task(
        &self,
        Parameters(a): Parameters<AddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let fields = TaskFields {
            title: Some(a.title),
            notes: a.notes,
            due: a.due,
            start: a.start,
            priority: a.priority,
            tags: a.tags,
            checklist: a.checklist.map(|l| {
                l.into_iter()
                    .map(|text| StepFields {
                        text,
                        done: Some(false),
                    })
                    .collect()
            }),
            files: a.files,
        };
        json(&self.store().create(fields, "aifred").map_err(fail)?)
    }

    #[tool(
        description = "Change a local task: its title, description, due day, start day, priority, tags, or checklist. Only the fields you send change. Use it only when the user asks for the change. To tick a step, send the full checklist with that step done. To add or remove steps, send the full checklist with no done values: the steps keep their ticks."
    )]
    async fn update_task(
        &self,
        Parameters(a): Parameters<UpdateArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let fields = TaskFields {
            title: a.title,
            notes: a.notes,
            due: a.due,
            start: a.start,
            priority: a.priority,
            tags: a.tags,
            checklist: a.checklist.map(|l| {
                l.into_iter()
                    .map(|c| StepFields {
                        text: c.text,
                        done: c.done,
                    })
                    .collect()
            }),
            files: None,
        };
        json(&self.store().update(&a.id, fields).map_err(fail)?)
    }

    #[tool(
        description = "The user's day plan: the items they want to do on one day, in order, with a time guess for each. Also the work day, the meetings of the day, the planned minutes, and the minutes of work time that are free after lunch and the meetings. The app places the items on the day around lunch. Use overview for the titles of the items."
    )]
    async fn day_plan(
        &self,
        Parameters(a): Parameters<DayArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let day = a.day.unwrap_or_else(default_day);
        // The meetings come from the saved events: get new ones when they are old.
        let note = aifred_core::calendar::refresh_if_old(&self.dir).await;
        let mut out = self.plan_view(&day).map_err(fail)?;
        if let Some(n) = note {
            out["notes"] = vec![n].into();
        }
        json(&out)
    }

    #[tool(
        description = "Save the user's day plan: all items in the order of the day, first item first. It replaces the old plan. Use it only when the user asks you to plan the day or to change the plan. Take the keys from overview. Put urgent and overdue work first, and do not plan more minutes than work_minutes in day_plan (the work day less lunch and meetings). Do not plan a local task whose start day is after the day. Give minutes and a short why for each item. Give the goal id when the item serves one of the user's goals. When two items are equally urgent, put the item that serves a goal first."
    )]
    async fn set_day_plan(
        &self,
        Parameters(a): Parameters<SetPlanArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let day = a.day.unwrap_or_else(default_day);
        // The plan cleans the spaces around each key. The time guesses use the same clean key.
        let items: Vec<plan::PlanItem> = a
            .items
            .iter()
            .map(|i| plan::PlanItem {
                key: i.key.trim().to_string(),
                why: i.why.clone(),
                goal: i.goal,
            })
            .collect();
        plan::set(&self.dir, &day, "ai", items, true).map_err(fail)?;
        for i in &a.items {
            if i.minutes.is_some() {
                plan::set_estimate(&self.dir, i.key.trim(), i.minutes, "aifred").map_err(fail)?;
            }
        }
        json(&self.plan_view(&day).map_err(fail)?)
    }

    #[tool(
        description = "Add one of the user's goals: what matters most to them now, in one short line. The user keeps at most 5 goals. Use it only when the user tells you a goal or asks you to add one."
    )]
    async fn add_goal(
        &self,
        Parameters(a): Parameters<GoalTextArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        json(&goals::add(&self.dir, &a.text, now()).map_err(fail)?)
    }

    #[tool(
        description = "Change the text of one of the user's goals. Use it only when the user asks for the change."
    )]
    async fn edit_goal(
        &self,
        Parameters(a): Parameters<GoalEditArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        json(&goals::edit(&self.dir, a.id, &a.text, now()).map_err(fail)?)
    }

    #[tool(
        description = "Remove one of the user's goals, for example when it is reached or no longer matters. Use it only when the user asks for it."
    )]
    async fn remove_goal(
        &self,
        Parameters(a): Parameters<GoalIdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        goals::remove(&self.dir, a.id, now()).map_err(fail)?;
        json(&goals::list(&self.dir).map_err(fail)?)
    }

    #[tool(
        description = "The user's \"waiting for\" list: what the user waits for from other people, since when, and the follow-up day. An item from an email closes by itself when the person answers."
    )]
    async fn waiting_list(
        &self,
        Parameters(a): Parameters<WaitingListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        json(&waiting::list(&self.dir, a.all.unwrap_or(false), now()).map_err(fail)?)
    }

    #[tool(
        description = "Add an item to the user's \"waiting for\" list. Use it when the user says they wait for something from another person, or asks to track an email (give its thread from no_reply_yet in overview). For \"tell me if Ana has not answered by Friday\", give follow_up: on that day AIfred tells the user when the item is still open. Use this, not a job, for that."
    )]
    async fn add_waiting(
        &self,
        Parameters(a): Parameters<AddWaitingArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let n = waiting::NewWaiting {
            what: a.what.unwrap_or_default(),
            who: a.who.unwrap_or_default(),
            since: a.since,
            follow_up: a.follow_up,
            thread: a.thread,
        };
        json(&waiting::add(&self.dir, n, now()).map_err(fail)?)
    }

    #[tool(
        description = "Change an item of the user's \"waiting for\" list: its text, the person, or the follow-up day. Or close it (done true) when it came or the user stopped waiting. Use it only when the user asks."
    )]
    async fn update_waiting(
        &self,
        Parameters(a): Parameters<UpdateWaitingArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let ch = waiting::WaitingChange {
            what: a.what,
            who: a.who,
            follow_up: a.follow_up,
            done: a.done,
        };
        json(&waiting::update(&self.dir, a.id, ch, now()).map_err(fail)?)
    }

    #[tool(
        description = "Mark a local task as done. Only do this when the user says the task is done."
    )]
    async fn complete_task(
        &self,
        Parameters(a): Parameters<IdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        json(&self.store().set_done(&a.id, true).map_err(fail)?)
    }
}

#[tool_handler]
impl ServerHandler for TaskTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Serves these tools over stdio until Claude Code closes the connection.
pub async fn serve() -> anyhow::Result<()> {
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
        let dir =
            std::env::temp_dir().join(format!("aifred-tasks-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TaskTools { dir }
    }

    fn add(title: &str, notes: Option<&str>) -> AddArgs {
        AddArgs {
            title: title.into(),
            notes: notes.map(String::from),
            due: None,
            start: None,
            priority: None,
            tags: None,
            checklist: None,
            files: None,
        }
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
        assert_eq!(
            t.store().load().unwrap().len(),
            1,
            "the task is saved to the file"
        );
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn list_hides_done_tasks_unless_asked() {
        let t = tools("list");
        let s = t.store();
        let a = s.add("Open one", "", "you").unwrap();
        let b = s.add("Done one", "", "you").unwrap();
        s.set_done(&b.id, true).unwrap();

        let open = body(
            t.list_tasks(Parameters(ListArgs {
                include_done: None,
                tag: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(open.as_array().unwrap().len(), 1);
        assert_eq!(open[0]["id"], a.id.as_str());

        let all = body(
            t.list_tasks(Parameters(ListArgs {
                include_done: Some(true),
                tag: None,
            }))
            .await
            .unwrap(),
        );
        assert_eq!(all.as_array().unwrap().len(), 2);
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn complete_task_marks_done_and_rejects_unknown_id() {
        let t = tools("complete");
        let a = t.store().add("Finish", "", "you").unwrap();
        let done = body(
            t.complete_task(Parameters(IdArgs { id: a.id.clone() }))
                .await
                .unwrap(),
        );
        assert_eq!(done["done"], true);
        assert!(
            t.complete_task(Parameters(IdArgs { id: "nope".into() }))
                .await
                .is_err()
        );
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn search_tasks_finds_words_and_hides_done_tasks() {
        let t = tools("search");
        let s = t.store();
        let a = s.add("Pay the invoice", "", "you").unwrap();
        s.add("Buy milk", "", "you").unwrap();
        let args = |done| {
            Parameters(SearchArgs {
                query: "invoices".into(),
                include_done: done,
                limit: None,
            })
        };
        let found = body(t.search_tasks(args(None)).await.unwrap());
        assert_eq!(found.as_array().unwrap().len(), 1);
        assert_eq!(found[0]["id"], a.id.as_str());
        s.set_done(&a.id, true).unwrap();
        assert!(
            body(t.search_tasks(args(None)).await.unwrap())
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            body(t.search_tasks(args(Some(true))).await.unwrap())
                .as_array()
                .unwrap()
                .len(),
            1
        );
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
        assert_eq!(
            (&task["due"], &task["start"], &task["priority"]),
            (&"2026-12-20".into(), &"2026-12-01".into(), &"high".into())
        );
        assert_eq!(task["tags"], serde_json::json!(["travel"]));
        assert_eq!(
            task["checklist"],
            serde_json::json!([{ "text": "Pick dates", "done": false }, { "text": "Pay", "done": false }])
        );
        let bad = AddArgs {
            due: Some("next Friday".into()),
            ..add("Book flights", None)
        };
        let e = t.add_task(Parameters(bad)).await.unwrap_err();
        assert!(
            e.message.contains("YYYY-MM-DD"),
            "the AI learns the right form: {}",
            e.message
        );
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
            checklist: Some(vec![CheckArg {
                text: "Ask bank".into(),
                done: Some(true),
            }]),
        };
        let task = body(t.update_task(Parameters(args)).await.unwrap());
        assert_eq!(
            (
                &task["title"],
                &task["notes"],
                &task["due"],
                &task["priority"]
            ),
            (
                &"Pay rent".into(),
                &"Before Friday".into(),
                &"2026-12-01".into(),
                &"urgent".into()
            )
        );
        assert_eq!(task["checklist"][0]["done"], true);
        // The AI adds a step and sends no done values: the ticked step stays ticked.
        let steps = vec![
            CheckArg {
                text: "Ask bank".into(),
                done: None,
            },
            CheckArg {
                text: "Pay".into(),
                done: None,
            },
        ];
        let more = UpdateArgs {
            id: a.id.clone(),
            title: None,
            notes: None,
            due: None,
            start: None,
            priority: None,
            tags: None,
            checklist: Some(steps),
        };
        let task = body(t.update_task(Parameters(more)).await.unwrap());
        assert_eq!(
            task["checklist"],
            serde_json::json!([{ "text": "Ask bank", "done": true }, { "text": "Pay", "done": false }])
        );
        let none = UpdateArgs {
            id: "nope".into(),
            title: None,
            notes: None,
            due: None,
            start: None,
            priority: None,
            tags: None,
            checklist: None,
        };
        assert!(t.update_task(Parameters(none)).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn list_tasks_can_keep_one_tag() {
        let t = tools("tag");
        let s = t.store();
        let a = s
            .create(
                TaskFields {
                    title: Some("Book flights".into()),
                    tags: Some(vec!["Travel".into()]),
                    ..Default::default()
                },
                "you",
            )
            .unwrap();
        s.add("Buy milk", "", "you").unwrap();
        let found = body(
            t.list_tasks(Parameters(ListArgs {
                include_done: None,
                tag: Some(" # travel".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(found.as_array().unwrap().len(), 1);
        assert_eq!(found[0]["id"], a.id.as_str());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn the_ai_saves_a_day_plan_with_time_guesses() {
        let t = tools("plan");
        let a = t.store().add("Pay rent", "", "you").unwrap();
        let key = format!("task:{}", a.id);
        let empty = body(
            t.day_plan(Parameters(DayArgs {
                day: Some("2026-11-04".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(empty["items"], serde_json::json!([]));
        assert_eq!(
            empty["work_minutes"], 465,
            "09:00 to 17:30 less 45 minutes of lunch"
        );
        let g = goals::add(&t.dir, "Ship 2.0", 1).unwrap();
        let item = PlanItemArg {
            key: key.clone(),
            minutes: Some(20),
            why: Some("Due today.".into()),
            goal: Some(g.id),
        };
        let saved = body(
            t.set_day_plan(Parameters(SetPlanArgs {
                day: Some("2026-11-04".into()),
                items: vec![item],
            }))
            .await
            .unwrap(),
        );
        assert_eq!(saved["items"][0]["minutes"], 20);
        assert_eq!(saved["items"][0]["why"], "Due today.");
        assert_eq!(saved["items"][0]["goal"], g.id);
        assert_eq!(saved["made_by"], "AIfred");
        let bad = PlanItemArg {
            key: "github:none#1".into(),
            minutes: None,
            why: None,
            goal: None,
        };
        let e = t
            .set_day_plan(Parameters(SetPlanArgs {
                day: Some("2026-11-04".into()),
                items: vec![bad],
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("github:none#1"), "{}", e.message);
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn the_ai_changes_goals_that_the_overview_shows() {
        let t = tools("goals");
        let g = body(
            t.add_goal(Parameters(GoalTextArgs {
                text: "Ship 2.0".into(),
            }))
            .await
            .unwrap(),
        );
        let id = g["id"].as_i64().unwrap();
        t.edit_goal(Parameters(GoalEditArgs {
            id,
            text: "Ship 2.0 by 30 Nov".into(),
        }))
        .await
        .unwrap();
        // The overview tool would download GitHub first. Its data comes from build.
        let o = overview::build(&t.dir, None).unwrap();
        assert_eq!(o["goals"][0]["text"], "Ship 2.0 by 30 Nov");
        let left = body(t.remove_goal(Parameters(GoalIdArgs { id })).await.unwrap());
        assert_eq!(left, serde_json::json!([]));
        assert!(t.remove_goal(Parameters(GoalIdArgs { id })).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }
}
