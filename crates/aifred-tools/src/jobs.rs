//! AIfred's own job tools, served over MCP (stdio): make, list, change, and
//! stop the jobs that AIfred runs by itself (aifred_core::jobs).
//!
//! The chat gets these tools. A job run does not: a job must not make jobs.

use aifred_core::default_data_dir;
use aifred_core::jobs::{self, Job, JobChange, NewJob, Trigger};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router, transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

#[derive(Clone)]
struct JobTools {
    dir: std::path::PathBuf,
}

#[derive(Deserialize, JsonSchema, Default)]
struct WhenArgs {
    /// One time, in the user's local time: "YYYY-MM-DDTHH:MM". Example: "2026-11-13T15:00".
    at: Option<String>,
    /// One time, this many minutes from now. Use it for "in 2 hours" (120).
    in_minutes: Option<u32>,
    /// A repeat rule in RFC 5545 RRULE form, without DTSTART. Examples:
    /// "FREQ=WEEKLY;BYDAY=MO" (every Monday), "FREQ=MONTHLY;BYMONTHDAY=3,17",
    /// "FREQ=MONTHLY;BYMONTHDAY=-1" (the last day of the month),
    /// "FREQ=WEEKLY;INTERVAL=2;BYDAY=TH" (every other Thursday),
    /// "FREQ=MONTHLY;BYDAY=-1FR" (the last Friday), "FREQ=DAILY;COUNT=5".
    /// UNTIL must be in UTC, for example "UNTIL=20261231T235959Z". At most once an hour.
    repeat: Option<String>,
    /// With `repeat`: the local time of the first run, "YYYY-MM-DDTHH:MM". Its time of day is the time of each run.
    start: Option<String>,
    /// Run after an event instead of at a time. Only "task_done": each time the user finishes a local task.
    after_event: Option<String>,
    /// With `after_event`: wait this many seconds for more events, so many events make one run (10-3600). Default 90.
    wait_seconds: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct ScheduleArgs {
    /// A short name. Example: "Timesheet reminder".
    title: String,
    /// "remind": show `message` as a notification, with no AI run.
    /// "agent": you run again at that time with read-only tools, do what `message` says, and tell the user the result.
    action: String,
    /// "remind": the text of the notification, written to the user. Example: "Submit your timesheet."
    /// "agent": the instructions for your later run. Example: "Check if PR acme/shop#123 is merged. Tell the user the state."
    message: String,
    #[serde(flatten)]
    when: WhenArgs,
    /// IANA time zone, for example "Asia/Jakarta". Leave out for the user's own time zone.
    timezone: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Also include jobs that are done or off. Default false.
    include_ended: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct UpdateArgs {
    /// Job id, from list_jobs.
    id: String,
    /// A new name. Leave out to keep it.
    title: Option<String>,
    /// A new reminder text or new instructions. Leave out to keep it.
    message: Option<String>,
    /// A new time: give at, in_minutes, repeat with start, or after_event. Leave all out to keep the time.
    #[serde(flatten)]
    when: WhenArgs,
    /// A new IANA time zone. Leave out to keep it.
    timezone: Option<String>,
    /// "paused" stops the job for now. "active" starts it again. Leave out to keep it.
    status: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    /// Job id, from list_jobs.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct RunsArgs {
    /// Job id, from list_jobs. Leave out for the runs of all jobs.
    id: Option<String>,
    /// How many runs, newest first (1-50). Default 10.
    limit: Option<u32>,
}

fn fail(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn json(v: &Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(v.to_string())]))
}

/// The trigger from the time fields, or None when all are empty.
fn trigger(w: WhenArgs, tz: &str) -> anyhow::Result<Option<Trigger>> {
    let given = [w.at.is_some(), w.in_minutes.is_some(), w.repeat.is_some(), w.after_event.is_some()]
        .iter()
        .filter(|b| **b)
        .count();
    if given > 1 {
        anyhow::bail!("give only one of at, in_minutes, repeat, or after_event");
    }
    if w.start.is_some() && w.repeat.is_none() {
        anyhow::bail!("start goes only with repeat. For one time, use at");
    }
    Ok(if let Some(at) = w.at {
        Some(Trigger::Once { at })
    } else if let Some(m) = w.in_minutes {
        if m == 0 {
            anyhow::bail!("in_minutes must be 1 or more");
        }
        // Real time, not clock time: "in 2 hours" across a summer time change is 2 hours.
        let t = aifred_core::files::now() as i64 + m as i64 * 60;
        Some(Trigger::Once { at: jobs::local_text_exact(t, tz)? })
    } else if let Some(rule) = w.repeat {
        let Some(start) = w.start else {
            anyhow::bail!("repeat needs start: the local time of the first run, YYYY-MM-DDTHH:MM");
        };
        Some(Trigger::Repeat { start, rule })
    } else {
        w.after_event.map(|event| Trigger::Event { event, wait_secs: w.wait_seconds.unwrap_or(90) })
    })
}

/// A job as the AI sees it: with its next runs in local time.
fn view(j: &Job) -> Value {
    let from = aifred_core::files::now() as i64;
    let next: Vec<String> = match j.status.as_str() {
        "active" => jobs::times_after(&j.trigger, &j.tz, from - 1, 3)
            .unwrap_or_default()
            .into_iter()
            .map(|t| jobs::show(t, &j.tz))
            .collect(),
        _ => vec![],
    };
    let mut v = serde_json::to_value(j).unwrap_or_default();
    v["next_runs"] = json!(next);
    if let Some(o) = v.as_object_mut() {
        o.remove("next_run");
        o.remove("updated");
    }
    v
}

#[tool_router]
impl JobTools {
    #[tool(
        description = "Make one of your own jobs: a reminder for the user, or an agent job that you run later with read-only tools. Use it when the user asks you to remind them, or to check or do something at a later time, again and again, or after they finish a task. Give exactly one of at, in_minutes, repeat (with start), or after_event. Times are the user's local time. The answer has `when` and `next_runs`: tell them to the user so they can check the time."
    )]
    async fn schedule_job(&self, Parameters(a): Parameters<ScheduleArgs>) -> Result<CallToolResult, ErrorData> {
        let tz = a.timezone.clone().filter(|z| !z.trim().is_empty()).unwrap_or_else(jobs::local_zone);
        if a.action != "remind" && a.action != "agent" {
            return Err(fail(anyhow::anyhow!("action must be \"remind\" or \"agent\"")));
        }
        let trigger = trigger(a.when, &tz).map_err(fail)?;
        if trigger.is_none() {
            return Err(fail(anyhow::anyhow!("give a time: at, in_minutes, repeat with start, or after_event")));
        }
        let job = jobs::create(
            &self.dir,
            NewJob {
                title: a.title,
                action: a.action,
                message: a.message,
                trigger,
                tz: Some(tz),
                created_by: "aifred".into(),
            },
        )
        .map_err(fail)?;
        json(&view(&job))
    }

    #[tool(
        description = "List your own jobs (reminders and agent jobs), the next run first, with `when` in plain words and the next run times. Not the user's to-do tasks."
    )]
    async fn list_jobs(&self, Parameters(a): Parameters<ListArgs>) -> Result<CallToolResult, ErrorData> {
        let all = jobs::list(&self.dir, a.include_ended.unwrap_or(false)).map_err(fail)?;
        json(&Value::Array(all.iter().map(view).collect()))
    }

    #[tool(
        description = "Change one of your jobs: its name, message, time, or time zone, or pause it and start it again. Only the fields you send change. Use it only when the user asks. After a new time, tell the user the new `when` and `next_runs`."
    )]
    async fn update_job(&self, Parameters(a): Parameters<UpdateArgs>) -> Result<CallToolResult, ErrorData> {
        let tz = match &a.timezone {
            Some(z) if !z.trim().is_empty() => z.clone(),
            _ => jobs::get(&self.dir, &a.id).map_err(fail)?.tz,
        };
        let trigger = trigger(a.when, &tz).map_err(fail)?;
        let job = jobs::update(
            &self.dir,
            &a.id,
            JobChange {
                title: a.title,
                message: a.message,
                trigger,
                tz: a.timezone,
                status: a.status,
            },
        )
        .map_err(fail)?;
        json(&view(&job))
    }

    #[tool(description = "Delete one of your jobs and its run history. Use it when the user asks you to stop or cancel a job.")]
    async fn cancel_job(&self, Parameters(a): Parameters<IdArgs>) -> Result<CallToolResult, ErrorData> {
        let job = jobs::get(&self.dir, &a.id).map_err(fail)?;
        jobs::delete(&self.dir, &a.id).map_err(fail)?;
        json(&json!({ "deleted": job.title }))
    }

    #[tool(
        description = "The latest runs of your jobs, newest first: when each ran, the result (ok, quiet = nothing to tell, skipped, failed), and what it told the user. Use it when the user asks what a job found or why it did not run."
    )]
    async fn job_runs(&self, Parameters(a): Parameters<RunsArgs>) -> Result<CallToolResult, ErrorData> {
        let list = jobs::runs(&self.dir, a.id.as_deref(), a.limit.unwrap_or(10).clamp(1, 50), true).map_err(fail)?;
        let titles: std::collections::HashMap<String, (String, String)> = jobs::list(&self.dir, true)
            .map_err(fail)?
            .into_iter()
            .map(|j| (j.id, (j.title, j.tz)))
            .collect();
        let out: Vec<Value> = list
            .into_iter()
            .map(|r| {
                let (title, tz) = titles.get(&r.job).cloned().unwrap_or_default();
                json!({
                    "job": r.job,
                    "title": title,
                    "planned": jobs::show(r.planned, &tz),
                    "state": r.state,
                    "late": r.late,
                    "note": r.note,
                    "output": r.output,
                })
            })
            .collect();
        json(&Value::Array(out))
    }
}

#[tool_handler]
impl ServerHandler for JobTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Serves these tools over stdio until Claude Code closes the connection.
pub async fn serve() -> anyhow::Result<()> {
    let dir = default_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let server = JobTools { dir }.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(name: &str) -> JobTools {
        let dir = std::env::temp_dir().join(format!("aifred-jobs-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        JobTools { dir }
    }

    fn body(r: CallToolResult) -> Value {
        let v = serde_json::to_value(r).unwrap();
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn schedule(title: &str, when: WhenArgs) -> ScheduleArgs {
        ScheduleArgs {
            title: title.into(),
            action: "remind".into(),
            message: "Submit the timesheet.".into(),
            when,
            timezone: Some("Asia/Jakarta".into()),
        }
    }

    #[tokio::test]
    async fn a_weekly_reminder_says_its_time_back() {
        let t = tools("weekly");
        let when = WhenArgs {
            repeat: Some("FREQ=WEEKLY;BYDAY=MO".into()),
            start: Some("2030-11-04T09:00".into()),
            ..Default::default()
        };
        let j = body(t.schedule_job(Parameters(schedule("Timesheet", when))).await.unwrap());
        assert_eq!(j["when"], "Every Monday at 09:00 (Asia/Jakarta)");
        assert_eq!(j["next_runs"][0], "Mon 4 Nov 2030 09:00");
        assert_eq!(j["next_runs"].as_array().unwrap().len(), 3);
        assert_eq!(j["created_by"], "aifred");
        let list = body(t.list_jobs(Parameters(ListArgs { include_ended: None })).await.unwrap());
        assert!(list.as_array().unwrap().iter().any(|x| x["title"] == "Timesheet"));
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn the_ai_learns_what_is_wrong() {
        let t = tools("errors");
        let two = WhenArgs { at: Some("2030-11-04T09:00".into()), in_minutes: Some(5), ..Default::default() };
        let e = t.schedule_job(Parameters(schedule("x", two))).await.unwrap_err();
        assert!(e.message.contains("only one"), "{}", e.message);
        let no_start = WhenArgs { repeat: Some("FREQ=DAILY".into()), ..Default::default() };
        let e = t.schedule_job(Parameters(schedule("x", no_start))).await.unwrap_err();
        assert!(e.message.contains("start"), "{}", e.message);
        let words = WhenArgs { at: Some("next Friday".into()), ..Default::default() };
        let e = t.schedule_job(Parameters(schedule("x", words))).await.unwrap_err();
        assert!(e.message.contains("YYYY-MM-DDTHH:MM"), "{}", e.message);
        let e = t.schedule_job(Parameters(schedule("x", WhenArgs::default()))).await.unwrap_err();
        assert!(e.message.contains("give a time"), "{}", e.message);
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn in_minutes_pause_and_cancel() {
        let t = tools("edit");
        let soon = WhenArgs { in_minutes: Some(120), ..Default::default() };
        let j = body(t.schedule_job(Parameters(schedule("Soon", soon))).await.unwrap());
        assert!(j["when"].as_str().unwrap().starts_with("Once, "));
        let id = j["id"].as_str().unwrap().to_string();
        let paused = body(
            t.update_job(Parameters(UpdateArgs {
                id: id.clone(),
                title: None,
                message: None,
                when: WhenArgs::default(),
                timezone: None,
                status: Some("paused".into()),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(paused["status"], "paused");
        assert_eq!(paused["next_runs"], json!([]));
        let gone = body(t.cancel_job(Parameters(IdArgs { id: id.clone() })).await.unwrap());
        assert_eq!(gone["deleted"], "Soon");
        assert!(t.cancel_job(Parameters(IdArgs { id })).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }
}
