//! AIfred's own jobs: the things that AIfred does by itself, at a time or
//! after an event. They are apart from the user's to-do tasks (tasks.rs).
//!
//! A job has a trigger and an action:
//! - Triggers: "once" (one local time), "repeat" (a calendar rule, RFC 5545
//!   RRULE), or "event" (for example "a task is done").
//! - Actions: "remind" (a notification, no AI call), "agent" (an AI run with
//!   read-only tools), "brief" (write the brief of today again), or "task"
//!   (add a new task for the user, for example "pay rent" on the 1st of each
//!   month). The rule of a "task" job is AIfred's (doer 'aifred'), so it is
//!   not in the user's list. Only the new tasks are.
//!
//! The desktop app runs the jobs (the scheduler loop in main.rs). It asks
//! `claim_due` for the jobs to run now. `claim_due` writes a run row and moves
//! the next run time forward in one transaction, before the job runs. So a
//! crash or a second program can never run the same time twice.
//!
//! Missed times (the computer was asleep, or AIfred was closed): a one-time
//! job runs late. A repeating job runs one time for the newest missed time,
//! if it is at most 12 hours old. The older times get a "skipped" row.
//! A "task" job adds a task for each missed time, also an old one: the user
//! must still pay the rent when the computer was off on the 1st. It adds one
//! task for each tick, oldest first, for at most 20 missed times (claim_task).
//!
//! See docs/research-agent-jobs.md for the reasons.

use std::path::Path;
use std::str::FromStr;

use anyhow::{Context, bail};
use chrono::{DateTime, NaiveDateTime, TimeZone, Utc};
use rusqlite::{Connection, OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::files::now;

/// At most this many jobs that are not finished (active or paused).
pub const MAX_JOBS: usize = 20;
/// A run that starts this much after its time is "late".
pub const LATE_SECS: i64 = 120;
/// A repeating job does not run for a missed time that is older than this.
/// The next time comes anyway.
pub const STALE_SECS: i64 = 12 * 3600;
/// An event job runs at most this long after the first event of a group.
pub const EVENT_MAX_WAIT: i64 = 600;
/// After this many failed runs in a row, a job turns off.
pub const MAX_FAILURES: u32 = 5;
/// An event job whose last run is still busy waits this long before the next check.
const BUSY_RECHECK_SECS: i64 = 60;
/// Events and runs older than this are deleted.
const KEEP_EVENTS: i64 = 7 * 86400;
const KEEP_RUNS: i64 = 60 * 86400;
/// After a long time with AIfred closed, at most this many "skipped" rows for
/// the missed times of one job.
const MAX_SKIP_ROWS: usize = 20;
/// After a long time with AIfred closed, a "task" job adds the tasks of at
/// most this many missed times (claim_task).
const MAX_CATCH_UP: usize = 20;

/// The event that a task write makes when a task is done.
pub const TASK_DONE: &str = "task_done";
/// The events that a job can wait for.
pub const EVENTS: &[&str] = &[TASK_DONE];

/// The id of the job that writes the brief again when a task is done.
pub const BRIEF_JOB: &str = "brief-on-task-done";

/// The answer of an agent run that has nothing for the user. It sends no
/// notification.
pub const NOTHING: &str = "NOTHING";
/// The start of an agent answer that ends its job (for example "the PR is
/// merged, so stop checking").
pub const DONE: &str = "DONE:";

/// When a job runs.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Trigger {
    /// One time. `at` is the local time, "YYYY-MM-DDTHH:MM".
    Once { at: String },
    /// A calendar rule. `start` is the local time of the first run,
    /// "YYYY-MM-DDTHH:MM". `rule` is the RRULE text, for example
    /// "FREQ=WEEKLY;BYDAY=MO".
    Repeat { start: String, rule: String },
    /// After an event. More events within `wait_secs` make one run.
    Event { event: String, wait_secs: u32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Job {
    pub id: String,
    /// A short name, for example "Timesheet reminder".
    pub title: String,
    /// "remind", "agent", "brief", or "task".
    pub action: String,
    /// "remind": the notification text. "agent": the instructions for the AI.
    pub message: String,
    pub trigger: Trigger,
    /// The IANA time zone of the trigger, for example "Asia/Jakarta".
    pub tz: String,
    /// "active", "paused", "done" (it has no more times), or "off" (too many failures).
    pub status: String,
    /// The next run, in Unix seconds. None: no run is planned.
    pub next_run: Option<i64>,
    /// "you" or "aifred".
    pub created_by: String,
    pub created: i64,
    pub updated: i64,
    /// Failed runs in a row.
    pub failures: u32,
    /// The trigger in plain words, for example "Every Monday at 09:00 (Asia/Jakarta)".
    pub when: String,
    /// True when "start" can work (update with status "active"): the job is
    /// not active, and it has a time to run. A job with no more times needs
    /// a new time first.
    pub can_start: bool,
}

/// One run of a job.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub id: i64,
    pub job: String,
    /// The planned time, in Unix seconds.
    pub planned: i64,
    pub started: Option<i64>,
    pub finished: Option<i64>,
    /// "running", "ok", "quiet" (nothing to tell), "skipped", or "failed".
    pub state: String,
    /// True when the run started more than 2 minutes after its time.
    pub late: bool,
    /// Why the run was skipped, or the error.
    pub note: String,
    /// What the run said to the user.
    pub output: String,
    pub cost_usd: Option<f64>,
}

/// A job that the scheduler must run now.
#[derive(Debug, Clone)]
pub struct Claim {
    pub job: Job,
    pub run: i64,
    pub planned: i64,
    pub late: bool,
    /// True when this claim tries a failed time of a "task" job again
    /// (claim_task). The scheduler then always shows the "New task"
    /// notification: the retry is not part of a catch-up.
    pub retry: bool,
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The run told the user something.
    Ok {
        output: String,
        cost_usd: Option<f64>,
    },
    /// The run had nothing to tell.
    Quiet {
        cost_usd: Option<f64>,
    },
    /// The run finished the job: the job has no more runs.
    Finished {
        output: String,
        cost_usd: Option<f64>,
    },
    Failed {
        error: String,
    },
}

/// What a new job needs.
#[derive(Debug, Clone, Default)]
pub struct NewJob {
    pub title: String,
    pub action: String,
    pub message: String,
    pub trigger: Option<Trigger>,
    /// None: the time zone of this computer.
    pub tz: Option<String>,
    pub created_by: String,
}

/// A change to a job. None keeps the value.
#[derive(Debug, Clone, Default)]
pub struct JobChange {
    pub title: Option<String>,
    pub message: Option<String>,
    pub trigger: Option<Trigger>,
    pub tz: Option<String>,
    /// "active" or "paused".
    pub status: Option<String>,
}

// ---------- Time ----------

/// The IANA time zone of this computer, or "UTC" when it is not known.
pub fn local_zone() -> String {
    iana_time_zone::get_timezone()
        .ok()
        .filter(|z| chrono_tz::Tz::from_str(z).is_ok())
        .unwrap_or_else(|| "UTC".into())
}

pub fn zone(name: &str) -> anyhow::Result<chrono_tz::Tz> {
    chrono_tz::Tz::from_str(name.trim()).map_err(|_| {
        anyhow::anyhow!(
            "\"{name}\" is not a time zone. Use an IANA name, for example \"Asia/Jakarta\"."
        )
    })
}

/// A local time "YYYY-MM-DDTHH:MM" (a space for "T" and seconds are also
/// good) in the zone `tz`. A time in the gap of a summer time change does not
/// exist and is an error. A time that is two times on the clock (the hour
/// that repeats in autumn) is the first one, unless the text ends with the
/// UTC offset, for example "2026-11-01T01:30-05:00".
pub fn parse_local(text: &str, tz: chrono_tz::Tz) -> anyhow::Result<DateTime<chrono_tz::Tz>> {
    let t = text.trim().replace(' ', "T");
    if let Ok(d) = DateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M%:z")
        .or_else(|_| DateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S%:z"))
    {
        return Ok(d.with_timezone(&tz));
    }
    let naive = NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M")
        .or_else(|_| NaiveDateTime::parse_from_str(&t, "%Y-%m-%dT%H:%M:%S"))
        .map_err(|_| anyhow::anyhow!("the time \"{text}\" is not in the form YYYY-MM-DDTHH:MM"))?;
    match tz.from_local_datetime(&naive) {
        chrono::LocalResult::Single(d) => Ok(d),
        chrono::LocalResult::Ambiguous(first, _) => Ok(first),
        chrono::LocalResult::None => bail!(
            "the time {text} does not exist in {}: the clocks skip it for summer time. Pick another time.",
            tz.name()
        ),
    }
}

/// A Unix time as local text in `tz`, for example "Mon 9 Nov 2026 09:00".
pub fn show(ts: i64, tz: &str) -> String {
    let Some(utc) = DateTime::<Utc>::from_timestamp(ts, 0) else {
        return ts.to_string();
    };
    match zone(tz) {
        Ok(z) => utc
            .with_timezone(&z)
            .format("%a %-d %b %Y %H:%M")
            .to_string(),
        Err(_) => utc.format("%a %-d %b %Y %H:%M UTC").to_string(),
    }
}

/// A Unix time as the local time "YYYY-MM-DDTHH:MM" in the zone `tz`.
pub fn local_text(ts: i64, tz: &str) -> anyhow::Result<String> {
    let utc = DateTime::<Utc>::from_timestamp(ts, 0).context("the time is out of range")?;
    Ok(utc
        .with_timezone(&zone(tz)?)
        .format("%Y-%m-%dT%H:%M")
        .to_string())
}

/// Like `local_text`, but parse_local gives the same minute back. In the hour
/// that repeats in autumn, the text gets the UTC offset ("-05:00"), so the
/// second 01:30 does not become the first one.
pub fn local_text_exact(ts: i64, tz: &str) -> anyhow::Result<String> {
    let z = zone(tz)?;
    let text = local_text(ts, tz)?;
    let minute = ts - ts.rem_euclid(60);
    if parse_local(&text, z)?.timestamp() == minute {
        return Ok(text);
    }
    let utc = DateTime::<Utc>::from_timestamp(ts, 0).context("the time is out of range")?;
    Ok(utc
        .with_timezone(&z)
        .format("%Y-%m-%dT%H:%M%:z")
        .to_string())
}

/// The RRULE text without "RRULE:" and spaces, with capital letters.
fn clean_rule(rule: &str) -> anyhow::Result<String> {
    let r = rule.trim();
    let r = r
        .strip_prefix("RRULE:")
        .unwrap_or(r)
        .trim()
        .to_ascii_uppercase();
    if r.is_empty() {
        bail!("the repeat rule is empty. Example: FREQ=WEEKLY;BYDAY=MO");
    }
    if r.contains('\n') || r.contains("DTSTART") {
        bail!(
            "give only the RRULE part, for example FREQ=WEEKLY;BYDAY=MO. Give the first time as \"start\"."
        );
    }
    // At most once an hour. With FREQ at least HOURLY and at most one BYMINUTE
    // and one BYSECOND value, two times are at least one hour apart: all times
    // have the same minute and second, so they differ by whole hours.
    for part in r.split(';') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        let too_often = match k {
            "FREQ" => matches!(v, "SECONDLY" | "MINUTELY"),
            "BYMINUTE" | "BYSECOND" => v.contains(','),
            _ => false,
        };
        if too_often {
            bail!(
                "a job can repeat at most once an hour. Use FREQ=HOURLY or slower, with at most one BYMINUTE value and one BYSECOND value."
            );
        }
    }
    Ok(r)
}

fn rule_set(start: &str, rule: &str, tz: chrono_tz::Tz) -> anyhow::Result<rrule::RRuleSet> {
    let first = parse_local(start, tz)?;
    let rtz: rrule::Tz = tz.into();
    let first = first.with_timezone(&rtz);
    rrule::RRuleSet::new(first)
        .set_from_string(&format!("RRULE:{}", clean_rule(rule)?))
        .map_err(|e| anyhow::anyhow!("the repeat rule \"{rule}\" is not valid: {e}"))
}

/// The times of `trigger` after `after` (Unix seconds, not included), at most `n`.
pub fn times_after(trigger: &Trigger, tz: &str, after: i64, n: u16) -> anyhow::Result<Vec<i64>> {
    match trigger {
        Trigger::Once { at } => {
            let t = parse_local(at, zone(tz)?)?.timestamp();
            Ok(if t > after && n > 0 { vec![t] } else { vec![] })
        }
        Trigger::Repeat { start, rule } => {
            let z = zone(tz)?;
            let rtz: rrule::Tz = z.into();
            let from = DateTime::<Utc>::from_timestamp(after + 1, 0)
                .context("the time is out of range")?
                .with_timezone(&rtz);
            Ok(rule_set(start, rule, z)?
                .after(from)
                .all(n)
                .dates
                .into_iter()
                .map(|d| d.timestamp())
                .collect())
        }
        Trigger::Event { .. } => Ok(vec![]),
    }
}

/// The times of `trigger` in (from, to]: how many there are, and the newest
/// `keep` of them, oldest first. It reads the rule in pages, so after a long
/// gap (an hourly rule that was missed for months) the count is true, and
/// the times are the newest ones.
fn times_between(
    trigger: &Trigger,
    tz: &str,
    from: i64,
    to: i64,
    keep: usize,
) -> anyhow::Result<(usize, Vec<i64>)> {
    const PAGE: u16 = 500;
    let mut count = 0;
    let mut tail = std::collections::VecDeque::with_capacity(keep + 1);
    let mut after = from;
    while after < to {
        let page = times_after(trigger, tz, after, PAGE)?;
        let full = page.len() == PAGE as usize;
        for t in page {
            if t > to {
                return Ok((count, tail.into()));
            }
            count += 1;
            tail.push_back(t);
            if tail.len() > keep {
                tail.pop_front();
            }
            after = t;
        }
        if !full {
            break;
        }
    }
    Ok((count, tail.into()))
}

/// The trigger in plain words.
pub fn describe(trigger: &Trigger, tz: &str) -> String {
    match trigger {
        Trigger::Once { at } => match zone(tz).and_then(|z| parse_local(at, z)) {
            Ok(d) => format!("Once, {} ({tz})", d.format("%a %-d %b %Y at %H:%M")),
            Err(_) => format!("Once, {at} ({tz})"),
        },
        Trigger::Repeat { start, rule } => {
            let time = start.get(11..16).unwrap_or("");
            format!("{} at {time} ({tz})", describe_rule(rule, start))
        }
        Trigger::Event { event, wait_secs } => {
            let what = match event.as_str() {
                TASK_DONE => "Each time you finish a task",
                e => return format!("After the event \"{e}\""),
            };
            format!("{what} (it waits {wait_secs} s for more)")
        }
    }
}

const DAYS: [(&str, &str); 7] = [
    ("MO", "Monday"),
    ("TU", "Tuesday"),
    ("WE", "Wednesday"),
    ("TH", "Thursday"),
    ("FR", "Friday"),
    ("SA", "Saturday"),
    ("SU", "Sunday"),
];

/// "1MO" -> "the first Monday", "-1FR" -> "the last Friday", "MO" -> "Monday".
fn describe_day(d: &str) -> String {
    let split = d.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(0);
    let (n, code) = d.split_at(split);
    let name = DAYS
        .iter()
        .find(|(c, _)| *c == code)
        .map(|(_, n)| *n)
        .unwrap_or(code);
    match n {
        "" => name.to_string(),
        "-1" => format!("the last {name}"),
        "1" | "+1" => format!("the first {name}"),
        "2" | "+2" => format!("the second {name}"),
        "3" | "+3" => format!("the third {name}"),
        "4" | "+4" => format!("the fourth {name}"),
        n => format!("{name} number {n}"),
    }
}

fn join_words(list: &[String]) -> String {
    match list {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

/// The common rules in plain words. Other rules show as they are.
fn describe_rule(rule: &str, start: &str) -> String {
    let Ok(rule) = clean_rule(rule) else {
        return format!("Repeats: {rule}");
    };
    let mut freq = "";
    let mut interval = 1u32;
    let (mut days, mut month_days, mut end) = (vec![], vec![], String::new());
    for part in rule.split(';') {
        let (k, v) = part.split_once('=').unwrap_or((part, ""));
        match k {
            "FREQ" => freq = v,
            "INTERVAL" => interval = v.parse().unwrap_or(1),
            "BYDAY" => days = v.split(',').map(describe_day).collect(),
            "BYMONTHDAY" => {
                month_days = v
                    .split(',')
                    .map(|d| {
                        if d == "-1" {
                            "the last day".to_string()
                        } else {
                            format!("day {d}")
                        }
                    })
                    .collect()
            }
            "COUNT" => end = format!(", {v} times"),
            "UNTIL" => {
                end = format!(
                    ", until {}",
                    v.get(..8)
                        .map(|d| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..]))
                        .unwrap_or_default()
                )
            }
            "WKST" => {}
            _ => return format!("Repeats from {start}: {rule}"),
        }
    }
    let every = |unit: &str| {
        if interval == 1 {
            format!("Every {unit}")
        } else {
            format!("Every {interval} {unit}s")
        }
    };
    let text = match freq {
        "HOURLY" => every("hour"),
        "DAILY" => every("day"),
        "WEEKLY" if days.is_empty() => every("week"),
        "WEEKLY" if interval == 1 => format!("Every {}", join_words(&days)),
        "WEEKLY" => format!("{} on {}", every("week"), join_words(&days)),
        "MONTHLY" if !month_days.is_empty() => {
            format!("{} on {}", every("month"), join_words(&month_days))
        }
        "MONTHLY" if !days.is_empty() => format!("{} on {}", every("month"), join_words(&days)),
        "MONTHLY" => every("month"),
        "YEARLY" => every("year"),
        _ => return format!("Repeats from {start}: {rule}"),
    };
    format!("{text}{end}")
}

// ---------- Store ----------

/// The SQL test for a job_run row that is a failed time of a "task" job and
/// waits for its retry (claim_task). A run with no start time is not a time
/// of the rule (claim_due writes it when a job cannot run): it gets no retry.
/// All queries use this one text. It has no table name: the task table has
/// no "state" or "started" column, so in a subquery on job_run the names
/// are the job_run columns. A macro, so concat!() can put it in a const.
macro_rules! waits_for_retry {
    () => {
        "state = 'failed' AND started IS NOT NULL"
    };
}

/// AIfred's tasks are the rows of the task table with doer 'aifred' (db.rs SCHEMA_19).
/// The last column is true when a failed time of a "task" job waits for its
/// retry (claim_task). The queries read the task table with no alias.
const COLUMNS: &str = concat!(
    "id, title, action, message, trigger, tz, status, next_run, added_by, created, updated, failures,
    action = 'task' AND EXISTS (SELECT 1 FROM job_run r WHERE r.job = task.id AND ",
    waits_for_retry!(),
    ")"
);

fn from_row(r: &Row) -> rusqlite::Result<Job> {
    let trigger: String = r.get(4)?;
    let trigger: Trigger = serde_json::from_str(&trigger).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    let tz: String = r.get(5)?;
    Ok(Job {
        id: r.get(0)?,
        title: r.get(1)?,
        action: r.get(2)?,
        message: r.get(3)?,
        when: describe(&trigger, &tz),
        can_start: can_start(&trigger, &tz, &status_of(r)?, r.get(12)?, now() as i64),
        trigger,
        tz,
        status: r.get(6)?,
        next_run: r.get(7)?,
        created_by: r.get(8)?,
        created: r.get(9)?,
        updated: r.get(10)?,
        failures: r.get(11)?,
    })
}

fn status_of(r: &Row) -> rusqlite::Result<String> {
    r.get(6)
}

/// True when update() accepts status "active" with no new time (see Job::can_start).
/// `retry`: a failed time of a "task" job waits for its retry. A start
/// retries it, also when the rule has no more times.
fn can_start(trigger: &Trigger, tz: &str, status: &str, retry: bool, now: i64) -> bool {
    match (status, trigger) {
        ("active", _) => false,
        _ if retry => true,
        (_, Trigger::Event { .. }) => true,
        // A paused one-time job whose time passed runs at once (update).
        ("paused", Trigger::Once { .. }) => true,
        _ => times_after(trigger, tz, now - 1, 1).is_ok_and(|t| !t.is_empty()),
    }
}

/// The job with this id, or None. The only query that reads one job.
fn find_in(c: &Connection, id: &str) -> anyhow::Result<Option<Job>> {
    Ok(c.query_row(
        &format!("SELECT {COLUMNS} FROM task WHERE id = ?1 AND doer = 'aifred'"),
        [id],
        from_row,
    )
    .optional()?)
}

fn get_in(c: &Connection, id: &str) -> anyhow::Result<Job> {
    find_in(c, id)?.with_context(|| format!("no job with id {id}"))
}

pub fn get(dir: &Path, id: &str) -> anyhow::Result<Job> {
    db::read(dir, |c| get_in(c, id))
}

/// The jobs, the next run first. `all` false: only active and paused jobs.
pub fn list(dir: &Path, all: bool) -> anyhow::Result<Vec<Job>> {
    db::read(dir, |c| list_in(c, all))
}

/// The same as list(), in an open connection. A job that cannot be read is
/// left out, with a line in the log: one bad job must not hide the other
/// jobs on the Tasks page, in the list_jobs tool, the brief, and the chat.
/// claim_due turns an active bad job off.
pub fn list_in(c: &Connection, all: bool) -> anyhow::Result<Vec<Job>> {
    let filter = if all {
        ""
    } else {
        "AND status IN ('active', 'paused')"
    };
    let mut st = c.prepare(&format!(
        "SELECT {COLUMNS} FROM task WHERE doer = 'aifred' {filter}
         ORDER BY status != 'active', next_run IS NULL, next_run, created"
    ))?;
    let rows = st.query_map([], |r| Ok((r.get::<_, String>(0)?, from_row(r))))?;
    let mut out = vec![];
    for row in rows {
        match row? {
            (_, Ok(j)) => out.push(j),
            (id, Err(e)) => eprintln!("aifred: the job {id} cannot be read: {e}"),
        }
    }
    Ok(out)
}

/// The runs of `job` (all jobs when None), the newest first, at most `limit`.
/// Skipped runs only when `skipped` is true. The runs of the brief job (action
/// "brief") only when `brief` is true: it runs after each finished task, so
/// its runs can fill the list.
pub fn runs(
    dir: &Path,
    job: Option<&str>,
    limit: u32,
    skipped: bool,
    brief: bool,
) -> anyhow::Result<Vec<Run>> {
    db::read(dir, |c| {
        let mut st = c.prepare(
            "SELECT id, job, planned, started, finished, state, late, note, output, cost_usd
             FROM job_run WHERE (?1 IS NULL OR job = ?1) AND (?2 OR state != 'skipped')
               AND (?4 OR job NOT IN (SELECT id FROM task WHERE doer = 'aifred' AND action = 'brief'))
             ORDER BY COALESCE(started, planned) DESC, id DESC LIMIT ?3",
        )?;
        let rows = st.query_map(
            rusqlite::params![job, skipped, limit.clamp(1, 200), brief],
            |r| {
                Ok(Run {
                    id: r.get(0)?,
                    job: r.get(1)?,
                    planned: r.get(2)?,
                    started: r.get(3)?,
                    finished: r.get(4)?,
                    state: r.get(5)?,
                    late: r.get(6)?,
                    note: r.get(7)?,
                    output: r.get(8)?,
                    cost_usd: r.get(9)?,
                })
            },
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

fn check_action(action: &str) -> anyhow::Result<()> {
    match action {
        "remind" | "agent" | "brief" | TASK => Ok(()),
        a => bail!("the action must be \"remind\", \"agent\", or \"task\", not \"{a}\""),
    }
}

/// A "task" job adds a task at a time. After an event, it has no time.
fn check_action_trigger(action: &str, trigger: &Trigger) -> anyhow::Result<()> {
    if action == TASK && matches!(trigger, Trigger::Event { .. }) {
        bail!("a job that adds a task needs a time or a repeat rule, not an event");
    }
    Ok(())
}

/// The action of a job that adds a new task for the user at each time.
pub const TASK: &str = "task";

/// Adds the task of a "task" job for the time `planned`, the time of the
/// claimed run `run`. The task gets the job's title, its message as the
/// notes, and the local day of `planned` as the due day.
///
/// The new task and the end of the run ("ok") are one transaction. So a
/// crash leaves both or neither: a time has at most one run (UNIQUE (job,
/// planned)), and a run that ends "ok" never runs again. A run that stopped
/// with no task is "failed", and claim_task tries it again. The finish()
/// call after it changes nothing, because the run is not "running" now.
pub fn make_task(
    dir: &Path,
    run: i64,
    job: &Job,
    planned: i64,
) -> anyhow::Result<crate::tasks::Task> {
    let tz = zone(&job.tz)?;
    let due = DateTime::from_timestamp(planned, 0)
        .context("bad run time")?
        .with_timezone(&tz)
        .format("%Y-%m-%d")
        .to_string();
    let t = now() as i64;
    db::write_jobs(dir, |tx| {
        let state: Option<String> = tx
            .query_row(
                "SELECT state FROM job_run WHERE id = ?1 AND job = ?2",
                rusqlite::params![run, job.id],
                |r| r.get(0),
            )
            .optional()?;
        if state.as_deref() != Some("running") {
            bail!("the run {run} of the job {} is not running", job.id);
        }
        let task = crate::tasks::TaskStore::create_in(
            tx,
            crate::tasks::TaskFields {
                title: Some(job.title.clone()),
                notes: Some(job.message.clone()),
                due: Some(due.clone()),
                ..Default::default()
            },
            &job.created_by,
        )?;
        tx.execute(
            "UPDATE job_run SET state = 'ok', finished = ?2, output = ?3, note = '' WHERE id = ?1",
            rusqlite::params![
                run,
                t,
                format!("Added the task \"{}\", due {due}.", task.title)
            ],
        )?;
        // A one-time job, or a job at the end of its rule, is done when it
        // has no next time and no other time waits for a retry.
        // claim_task keeps the job active until here, so the job is "done"
        // only when its last task is there.
        tx.execute(
            concat!(
                "UPDATE task SET failures = 0,
                    status = CASE WHEN status = 'active' AND next_run IS NULL AND NOT EXISTS (
                               SELECT 1 FROM job_run r WHERE r.job = task.id AND ",
                waits_for_retry!(),
                ")
                             THEN 'done' ELSE status END
             WHERE id = ?1"
            ),
            [&job.id],
        )?;
        Ok(task)
    })
}

/// The wait before a retry after `failures` failed runs in a row: 1, 2, 4,
/// then 8 minutes. 0 failures (after a start or a crash): 1 minute.
/// RETRY_WAIT_SQL is the same wait in SQL. The test
/// the_retry_wait_is_the_same_in_rust_and_sql keeps the two the same.
fn retry_wait(failures: u32) -> i64 {
    60 * (1i64 << (failures.clamp(1, MAX_FAILURES) - 1))
}

/// retry_wait() in SQL, for any_due. `task.failures` is the failure count,
/// and ?2 is MAX_FAILURES.
macro_rules! retry_wait_sql {
    () => {
        "(60 << (MIN(MAX(task.failures, 1), ?2) - 1))"
    };
}

/// Checks the trigger, and gives the next run after `now`.
fn check_trigger(trigger: &Trigger, tz: &str, now: i64) -> anyhow::Result<Option<i64>> {
    zone(tz)?;
    match trigger {
        Trigger::Once { at } => {
            let t = parse_local(at, zone(tz)?)?.timestamp();
            // A minute of slack: "now" from the AI can be a little old.
            if t < now - 60 {
                bail!(
                    "the time {at} ({tz}) is in the past. Now is {}.",
                    show(now, tz)
                );
            }
            Ok(Some(t.max(now)))
        }
        Trigger::Repeat { .. } => {
            let next = times_after(trigger, tz, now - 1, 1)?;
            match next.first() {
                Some(t) => Ok(Some(*t)),
                None => bail!("the repeat rule has no time after now"),
            }
        }
        Trigger::Event { event, wait_secs } => {
            if !EVENTS.contains(&event.as_str()) {
                bail!("the event must be one of {EVENTS:?}, not \"{event}\"");
            }
            if !(10..=3600).contains(wait_secs) {
                bail!("the wait must be from 10 to 3600 seconds");
            }
            Ok(None)
        }
    }
}

fn new_id() -> String {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("j{nanos}")
}

/// The id of the newest event, or 0.
fn last_event(tx: &Connection) -> anyhow::Result<i64> {
    Ok(
        tx.query_row("SELECT COALESCE(MAX(id), 0) FROM job_event", [], |r| {
            r.get(0)
        })?,
    )
}

pub fn create(dir: &Path, j: NewJob) -> anyhow::Result<Job> {
    let title = j.title.trim().to_string();
    if title.is_empty() {
        bail!("the job needs a title");
    }
    check_action(&j.action)?;
    if j.action != "brief" && j.action != TASK && j.message.trim().is_empty() {
        bail!("the job needs a message: the reminder text, or the instructions for the AI");
    }
    let trigger = j.trigger.context("the job needs a time or an event")?;
    check_action_trigger(&j.action, &trigger)?;
    let tz =
        j.tz.filter(|z| !z.trim().is_empty())
            .unwrap_or_else(local_zone);
    let t = now() as i64;
    let next = check_trigger(&trigger, &tz, t)?;
    db::write_jobs(dir, |tx| {
        let open: usize = tx.query_row(
            "SELECT COUNT(*) FROM task WHERE doer = 'aifred' AND status IN ('active', 'paused')",
            [],
            |r| r.get(0),
        )?;
        if open >= MAX_JOBS {
            bail!("AIfred has {open} jobs, the most it can keep. Delete or finish a job first.");
        }
        let id = new_id();
        tx.execute(
            "INSERT INTO task (id, title, action, message, trigger, tz, status, next_run,
                               added_by, created, updated, failures, cursor, doer)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?9, 0, ?10, 'aifred')",
            rusqlite::params![
                id,
                title,
                j.action,
                j.message.trim(),
                serde_json::to_string(&trigger)?,
                tz,
                next,
                j.created_by,
                t,
                // An event job waits for new events only.
                last_event(tx)?,
            ],
        )?;
        get_in(tx, &id)
    })
}

pub fn update(dir: &Path, id: &str, ch: JobChange) -> anyhow::Result<Job> {
    let t = now() as i64;
    db::write_jobs(dir, |tx| {
        let old = get_in(tx, id)?;
        let title = match ch.title {
            Some(s) if s.trim().is_empty() => bail!("the job needs a title"),
            Some(s) => s.trim().to_string(),
            None => old.title.clone(),
        };
        let message = ch
            .message
            .map(|m| m.trim().to_string())
            .unwrap_or(old.message.clone());
        let tz = ch
            .tz
            .filter(|z| !z.trim().is_empty())
            .unwrap_or(old.tz.clone());
        let new_time = ch.trigger.is_some() || tz != old.tz;
        let trigger = ch.trigger.unwrap_or(old.trigger.clone());
        check_action_trigger(&old.action, &trigger)?;
        let status = match ch.status.as_deref() {
            // A new time for a job that ended ("move that reminder to
            // Friday"): the job starts again. A paused job stays paused.
            None if new_time && matches!(old.status.as_str(), "done" | "off") => {
                "active".to_string()
            }
            None => old.status.clone(),
            Some(s @ ("active" | "paused")) => s.to_string(),
            Some(s) => bail!("the status must be \"active\" or \"paused\", not \"{s}\""),
        };
        let resumed = status == "active" && old.status != "active";
        let retry = old.action == TASK && has_retry(tx, id)?;
        if resumed && !new_time && !can_start(&trigger, &tz, &old.status, retry, t) {
            // A job with no more times comes back only with a new time.
            bail!("the job has no more times. Give it a new time.");
        }
        if resumed {
            let open: usize = tx.query_row(
                "SELECT COUNT(*) FROM task WHERE doer = 'aifred' AND status IN ('active', 'paused') AND id != ?1",
                [id],
                |r| r.get(0),
            )?;
            if old.status != "paused" && open >= MAX_JOBS {
                bail!(
                    "AIfred has {open} jobs, the most it can keep. Delete or finish a job first."
                );
            }
        }
        // A new time, or a job that starts again: the next run is from now,
        // so the missed times do not all run.
        let next = if resumed && !new_time && retry {
            // The failed time runs first (claim_task). The next run is the
            // next time of the rule after now, or none: a one-time job has
            // its run already, and a rule can be at its end.
            times_after(&trigger, &tz, t - 1, 1)?.first().copied()
        } else if let (true, false, "paused", Trigger::Once { at }) =
            (resumed, new_time, old.status.as_str(), &trigger)
        {
            // The pause held the run back. When its time passed during the
            // pause, it runs at the next tick, and the run shows it as late.
            Some(parse_local(at, zone(&tz)?)?.timestamp())
        } else if new_time || resumed {
            check_trigger(&trigger, &tz, t)?
        } else {
            old.next_run
        };
        // An event job waits for new events only: after a start, and after a
        // change from a time (or from another event) to this event.
        let same_event = matches!((&old.trigger, &trigger),
            (Trigger::Event { event: a, .. }, Trigger::Event { event: b, .. }) if a == b);
        let cursor = if matches!(trigger, Trigger::Event { .. }) && (resumed || !same_event) {
            Some(last_event(tx)?)
        } else {
            None
        };
        tx.execute(
            "UPDATE task SET title = ?2, message = ?3, trigger = ?4, tz = ?5, status = ?6,
                            next_run = ?7, updated = ?8,
                            failures = CASE WHEN ?9 THEN 0 ELSE failures END,
                            cursor = COALESCE(?10, cursor)
             WHERE id = ?1",
            rusqlite::params![
                id,
                title,
                message,
                serde_json::to_string(&trigger)?,
                tz,
                status,
                next,
                t,
                // A start or a new time is a new try: 1 minute to the next
                // retry. A failed time of a "task" job still gets its retry
                // after a start (claim_task).
                resumed || new_time,
                cursor
            ],
        )?;
        if new_time && old.action == TASK {
            // The new time replaces the old ones ("move it to Friday"), so a
            // failed old time gets no task. The run list tells why.
            tx.execute(
                concat!(
                    "UPDATE job_run SET state = 'skipped',
                        note = 'No task: the job got a new time before a retry. The error was: ' || note
                 WHERE job = ?1 AND ",
                    waits_for_retry!()
                ),
                [id],
            )?;
        }
        get_in(tx, id)
    })
}

/// True when a failed time of the job waits for its retry (claim_task).
fn has_retry(tx: &Transaction, id: &str) -> anyhow::Result<bool> {
    Ok(tx
        .query_row(
            concat!(
                "SELECT 1 FROM job_run WHERE job = ?1 AND ",
                waits_for_retry!(),
                " LIMIT 1"
            ),
            [id],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// Deletes the job and its runs. The brief job cannot be deleted, because
/// nothing can make it again. Pause it instead.
pub fn delete(dir: &Path, id: &str) -> anyhow::Result<()> {
    if id == BRIEF_JOB {
        bail!("AIfred's brief job cannot be deleted. Pause it to stop it.");
    }
    db::write_jobs(dir, |tx| {
        if tx.execute("DELETE FROM task WHERE id = ?1 AND doer = 'aifred'", [id])? == 0 {
            bail!("no job with id {id}");
        }
        Ok(())
    })
}

/// Saves an event. Event jobs run after it (claim_due). The tool servers
/// run in other programs, so the event goes through the database.
pub(crate) fn record_event(tx: &Transaction, name: &str) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO job_event (name, ts) VALUES (?1, ?2)",
        rusqlite::params![name, now() as i64],
    )?;
    Ok(())
}

/// Only the AIfred window that holds this lock runs the jobs. A second
/// window on the same data folder waits, and takes over when the first one
/// stops. The lock ends when the process stops, also after a crash.
#[derive(Debug)]
pub struct SchedulerLock {
    _file: std::fs::File,
}

impl SchedulerLock {
    /// Takes the lock in `dir`. None: another window runs the jobs.
    pub fn take(dir: &Path) -> anyhow::Result<Option<SchedulerLock>> {
        let path = dir.join("scheduler.lock");
        let file = std::fs::File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("cannot open {}", path.display()))?;
        match file.try_lock() {
            Ok(()) => Ok(Some(SchedulerLock { _file: file })),
            Err(std::fs::TryLockError::WouldBlock) => Ok(None),
            Err(std::fs::TryLockError::Error(e)) => {
                Err(e).context("cannot lock the scheduler file")
            }
        }
    }
}

/// Marks the runs that were busy when AIfred stopped as failed.
/// Call it only while you hold the SchedulerLock: then no other window has
/// a run that is busy now. Most of them do not run again: a reminder can
/// come twice, but an AI run must not. A "task" job is different: its task
/// and the end of its run are one transaction (make_task). So a "running"
/// run has no task, and claim_task tries it again after a minute.
pub fn recover(dir: &Path) -> anyhow::Result<usize> {
    let t = now() as i64;
    db::write_jobs(dir, |tx| {
        // An older AIfred made a one-time "task" job "done" at its claim
        // (claim_task now keeps it active). It must be active again, or
        // claim_due does not see the retry.
        tx.execute(
            "UPDATE task SET status = 'active'
             WHERE doer = 'aifred' AND action = 'task' AND status = 'done'
               AND id IN (SELECT job FROM job_run WHERE state = 'running')",
            [],
        )?;
        Ok(tx.execute(
            "UPDATE job_run SET state = 'failed', finished = ?1,
                    note = 'AIfred stopped during the run.'
             WHERE state = 'running'",
            [t],
        )?)
    })
}

fn add_run(
    tx: &Transaction,
    job: &str,
    planned: i64,
    state: &str,
    note: &str,
    started: Option<i64>,
    late: bool,
) -> anyhow::Result<Option<i64>> {
    let n = tx.execute(
        "INSERT OR IGNORE INTO job_run (job, planned, started, state, late, note)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![job, planned, started, state, late, note],
    )?;
    Ok((n == 1).then(|| tx.last_insert_rowid()))
}

fn busy(tx: &Transaction, job: &str) -> anyhow::Result<bool> {
    Ok(tx
        .query_row(
            "SELECT 1 FROM job_run WHERE job = ?1 AND state = 'running' LIMIT 1",
            [job],
            |_| Ok(()),
        )
        .optional()?
        .is_some())
}

/// The jobs to run now. For each one, it saves a "running" row and moves the
/// next run time forward, in the same transaction. Run each claim, then call
/// `finish` with its run id.
///
/// Most ticks have nothing to do. So it first reads, and takes the write lock
/// only when a job is due. A job that cannot be read or has a bad time turns
/// off with a failed run that tells why. The other jobs still run.
pub fn claim_due(dir: &Path, now: i64) -> anyhow::Result<Vec<Claim>> {
    let clean = cleanup_due(now);
    if !clean && !db::read(dir, |c| any_due(c, now))? {
        return Ok(vec![]);
    }
    db::write_jobs(dir, |tx| {
        if clean {
            tx.execute("DELETE FROM job_event WHERE ts < ?1", [now - KEEP_EVENTS])?;
            // A failed time of a "task" job waits for its task (claim_task),
            // also when the job is paused for a long time. It stays. When the
            // job is off or done, nothing retries it, so it goes as the
            // other runs do.
            tx.execute(
                concat!(
                    "DELETE FROM job_run WHERE COALESCE(finished, planned) < ?1 AND state != 'running'
                   AND NOT (",
                    waits_for_retry!(),
                    " AND job IN (SELECT id FROM task WHERE doer = 'aifred' AND action = 'task'
                                          AND status IN ('active', 'paused')))"
                ),
                [now - KEEP_RUNS],
            )?;
        }
        let jobs: Vec<(String, rusqlite::Result<(Job, i64)>)> = {
            let mut st = tx.prepare(&format!(
                "SELECT {COLUMNS}, cursor FROM task WHERE doer = 'aifred' AND status = 'active'"
            ))?;
            st.query_map([], |r| {
                Ok((r.get(0)?, from_row(r).and_then(|j| Ok((j, r.get(13)?)))))
            })?
            .collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (id, job) in jobs {
            // A savepoint for each job: a failure undoes only this job's writes.
            tx.execute_batch("SAVEPOINT one_job")?;
            let claim =
                job.map_err(anyhow::Error::from)
                    .and_then(|(job, cursor)| match &job.trigger {
                        Trigger::Event { event, wait_secs } => {
                            claim_event(tx, &job, cursor, event, *wait_secs, now)
                        }
                        _ => claim_time(tx, &job, now),
                    });
            match claim {
                Ok(c) => out.extend(c),
                Err(e) => {
                    tx.execute_batch("ROLLBACK TO one_job")?;
                    eprintln!("aifred: job {id} turns off: {e:#}");
                    let note = format!("AIfred cannot run this job: {e:#}");
                    // No start time: this row is not a time of the rule, so
                    // claim_task never tries it again as a task.
                    add_run(tx, &id, now, "failed", &note, None, false)?;
                    tx.execute(
                        "UPDATE task SET status = 'off', next_run = NULL WHERE id = ?1",
                        [&id],
                    )?;
                }
            }
            tx.execute_batch("RELEASE one_job")?;
        }
        Ok(out)
    })
}

/// True at most once an hour (and at the first call): time to delete old
/// events and runs.
fn cleanup_due(now: i64) -> bool {
    use std::sync::atomic::{AtomicI64, Ordering};
    static LAST: AtomicI64 = AtomicI64::new(i64::MIN);
    let last = LAST.load(Ordering::Relaxed);
    if last != i64::MIN && (0..3600).contains(&(now - last)) {
        return false;
    }
    LAST.store(now, Ordering::Relaxed);
    true
}

/// True when an active job has a run time that passed, or an event job has
/// new events. Only then does claim_due take the write lock.
fn any_due(c: &Connection, now: i64) -> anyhow::Result<bool> {
    Ok(c.query_row(
        concat!(
            "SELECT EXISTS (
           SELECT 1 FROM task WHERE doer = 'aifred' AND status = 'active' AND (
             -- A 'task' job with a failed time waits for that retry first
             -- (claim_task).
             CASE WHEN action = 'task' THEN COALESCE(
                    (SELECT COALESCE(r.finished, 0) + ",
            retry_wait_sql!(),
            " <= ?1
                     FROM job_run r
                     WHERE r.job = task.id AND ",
            waits_for_retry!(),
            "
                     ORDER BY r.planned LIMIT 1),
                    next_run IS NOT NULL AND next_run <= ?1)
                  ELSE next_run IS NOT NULL AND next_run <= ?1 END
             -- A bad trigger is due too: claim_due turns the job off.
             OR CASE WHEN json_valid(trigger) THEN
                  json_extract(trigger, '$.kind') = 'event' AND next_run IS NULL AND EXISTS (
                    SELECT 1 FROM job_event e
                    WHERE e.name = json_extract(task.trigger, '$.event') AND e.id > task.cursor)
                ELSE 1 END))"
        ),
        rusqlite::params![now, MAX_FAILURES],
        |r| r.get(0),
    )?)
}

fn claim_time(tx: &Transaction, job: &Job, now: i64) -> anyhow::Result<Option<Claim>> {
    if job.action == TASK {
        return claim_task(tx, job, now);
    }
    let Some(mut planned) = job.next_run else {
        return Ok(None);
    };
    if planned > now {
        return Ok(None);
    }
    // The times after `planned` that also passed: only the newest one runs.
    let (n, later) = times_between(&job.trigger, &job.tz, planned, now, MAX_SKIP_ROWS + 1)?;
    let next = times_after(&job.trigger, &job.tz, now, 1)?.first().copied();
    if let Some((&newest, before_newest)) = later.split_last() {
        // The missed times: `planned` and the later times without the
        // newest, n in all. `later` has only the newest of them.
        let missed: Vec<i64> = if n > later.len() {
            before_newest.to_vec()
        } else {
            std::iter::once(planned)
                .chain(before_newest.iter().copied())
                .collect()
        };
        skip_rows(tx, &job.id, &missed, n - missed.len(), |hidden| {
            if hidden > 0 {
                format!("AIfred was not running at this time, and at {hidden} earlier times.")
            } else {
                "AIfred was not running at this time.".to_string()
            }
        })?;
        planned = newest;
    }
    let repeat = matches!(job.trigger, Trigger::Repeat { .. });
    let set_next = |next: Option<i64>| set_next(tx, &job.id, next);
    if repeat && now - planned > STALE_SECS {
        add_run(
            tx,
            &job.id,
            planned,
            "skipped",
            "Too late: AIfred was not running at this time.",
            None,
            false,
        )?;
        set_next(next)?;
        return Ok(None);
    }
    if busy(tx, &job.id)? {
        add_run(
            tx,
            &job.id,
            planned,
            "skipped",
            "The last run was still busy.",
            None,
            false,
        )?;
        set_next(next)?;
        return Ok(None);
    }
    let late = now - planned > LATE_SECS;
    let run = add_run(tx, &job.id, planned, "running", "", Some(now), late)?;
    set_next(next)?;
    let job = get_in(tx, &job.id)?;
    Ok(run.map(|run| Claim {
        job,
        run,
        planned,
        late,
        retry: false,
    }))
}

/// Sets the next run of a job that runs at times. No next time: the job is done.
fn set_next(tx: &Transaction, id: &str, next: Option<i64>) -> anyhow::Result<()> {
    let status = if next.is_none() { "done" } else { "active" };
    tx.execute(
        "UPDATE task SET next_run = ?2, status = ?3 WHERE id = ?1",
        rusqlite::params![id, next, status],
    )?;
    Ok(())
}

/// Writes a "skipped" row for the missed times in `times` (oldest first).
/// `before` older times came before them. At most the newest MAX_SKIP_ROWS
/// get a row. `note` gets the number of older times with no row, for the
/// first row, and 0 for the other rows.
fn skip_rows(
    tx: &Transaction,
    job: &str,
    times: &[i64],
    before: usize,
    note: impl Fn(usize) -> String,
) -> anyhow::Result<()> {
    let cut = times.len().saturating_sub(MAX_SKIP_ROWS);
    let hidden = before + cut;
    for (i, t) in times[cut..].iter().enumerate() {
        let n = if i == 0 { hidden } else { 0 };
        add_run(tx, job, *t, "skipped", &note(n), None, false)?;
    }
    Ok(())
}

/// The claim of a "task" job. Each time of the rule must give its task, with
/// the local day of that time as the due day. So, not as the other jobs:
/// - It claims the oldest time that has no task, one time for each tick.
///   After a long miss, the next ticks add the tasks of the other missed
///   times, one after the other. Only the newest MAX_CATCH_UP missed times
///   get a task. The older ones get a "skipped" row.
/// - A busy run does not skip a time. The time waits for the next tick.
/// - A failed time (an error, or AIfred stopped during the run) tries again
///   with the same run row and the same time, so the task gets the due day of
///   the planned time. The retry waits retry_wait(), counted from the end of
///   the failed run. While it waits, no other time starts.
/// - next_run is always the next time of the rule that has no run. A failure
///   does not change it, so a retry during a catch-up still waits.
fn claim_task(tx: &Transaction, job: &Job, now: i64) -> anyhow::Result<Option<Claim>> {
    if busy(tx, &job.id)? {
        return Ok(None);
    }
    // A run with no start time is not a time of the rule (claim_due writes
    // it when the job cannot run). It gets no retry.
    let failed: Option<(i64, i64, i64)> = tx
        .query_row(
            concat!(
                "SELECT id, planned, COALESCE(finished, 0) FROM job_run
             WHERE job = ?1 AND ",
                waits_for_retry!(),
                " ORDER BY planned LIMIT 1"
            ),
            [&job.id],
            |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
        )
        .optional()?;
    if let Some((run, planned, finished)) = failed {
        if now < finished + retry_wait(job.failures) {
            return Ok(None);
        }
        tx.execute(
            "UPDATE job_run SET state = 'running', started = ?2, finished = NULL WHERE id = ?1",
            rusqlite::params![run, now],
        )?;
        let late = now - planned > LATE_SECS;
        return Ok(Some(Claim {
            job: get_in(tx, &job.id)?,
            run,
            planned,
            late,
            retry: true,
        }));
    }
    let Some(mut planned) = job.next_run else {
        return Ok(None);
    };
    if planned > now {
        return Ok(None);
    }
    // After a long miss, only the newest MAX_CATCH_UP times get a task.
    let (n, later) = times_between(
        &job.trigger,
        &job.tz,
        planned,
        now,
        MAX_CATCH_UP + MAX_SKIP_ROWS,
    )?;
    let missed = 1 + n;
    if missed > MAX_CATCH_UP {
        // The newest part of all missed times: `planned` and `later`.
        let known: Vec<i64> = if n > later.len() {
            later
        } else {
            std::iter::once(planned).chain(later).collect()
        };
        let (old, keep) = known.split_at(known.len() - MAX_CATCH_UP);
        skip_rows(tx, &job.id, old, missed - known.len(), |hidden| {
            if hidden > 0 {
                format!(
                    "AIfred was not running for a long time. No task for this time, and for {hidden} earlier times."
                )
            } else {
                "AIfred was not running for a long time. No task for this time.".to_string()
            }
        })?;
        planned = keep[0];
    }
    let late = now - planned > LATE_SECS;
    let run = add_run(tx, &job.id, planned, "running", "", Some(now), late)?;
    let next = times_after(&job.trigger, &job.tz, planned, 1)?
        .first()
        .copied();
    let Some(run) = run else {
        // This time already has a run: its task is there. So with no next
        // time, the job is done now.
        set_next(tx, &job.id, next)?;
        return Ok(None);
    };
    // Not set_next(): with no next time, the job stays active. make_task
    // makes it "done" in the same step as its last task, so the job is
    // never "done" while its last task is not there.
    tx.execute(
        "UPDATE task SET next_run = ?2 WHERE id = ?1",
        rusqlite::params![job.id, next],
    )?;
    Ok(Some(Claim {
        job: get_in(tx, &job.id)?,
        run,
        planned,
        late,
        retry: false,
    }))
}

fn claim_event(
    tx: &Transaction,
    job: &Job,
    cursor: i64,
    event: &str,
    wait: u32,
    now: i64,
) -> anyhow::Result<Option<Claim>> {
    let (first, last, top): (Option<i64>, Option<i64>, Option<i64>) = tx.query_row(
        "SELECT MIN(ts), MAX(ts), MAX(id) FROM job_event WHERE name = ?1 AND id > ?2",
        rusqlite::params![event, cursor],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let (Some(first), Some(last), Some(top)) = (first, last, top) else {
        if job.next_run.is_some() {
            tx.execute("UPDATE task SET next_run = NULL WHERE id = ?1", [&job.id])?;
        }
        return Ok(None);
    };
    // Wait for a quiet moment, but not too long after the first event.
    let due = (last + wait as i64).min(first + EVENT_MAX_WAIT);
    // Not yet: check again at the due time.
    // A run that is still busy: the new events wait for the next run. Check
    // again a minute later, not on each tick, so ticks do not take the
    // write lock again and again during a long run.
    let next = if due > now {
        Some(due)
    } else if busy(tx, &job.id)? {
        Some(now + BUSY_RECHECK_SECS)
    } else {
        None
    };
    if let Some(next) = next {
        if job.next_run != Some(next) {
            tx.execute(
                "UPDATE task SET next_run = ?2 WHERE id = ?1",
                rusqlite::params![job.id, next],
            )?;
        }
        return Ok(None);
    }
    tx.execute(
        "UPDATE task SET next_run = NULL, cursor = ?2 WHERE id = ?1",
        rusqlite::params![job.id, top],
    )?;
    let run = add_run(tx, &job.id, due, "running", "", Some(now), false)?;
    let job = get_in(tx, &job.id)?;
    Ok(run.map(|run| Claim {
        job,
        run,
        planned: due,
        late: false,
        retry: false,
    }))
}

/// Saves how a run ended, and what it means for the job:
/// - a failure retries after 1, 2, 4, then 8 minutes. After 5 failures in a
///   row the job turns off. Event jobs do not retry: the next event runs them.
/// - "Finished" ends the job.
///
/// A run that is not "running" now does not change: make_task ended it
/// already. It gives the job after the change, or None when the job is gone.
pub fn finish(dir: &Path, run: i64, outcome: Outcome) -> anyhow::Result<Option<Job>> {
    let t = now() as i64;
    db::write_jobs(dir, |tx| {
        let row: Option<(String, String)> = tx
            .query_row("SELECT job, state FROM job_run WHERE id = ?1", [run], |r| {
                Ok((r.get(0)?, r.get(1)?))
            })
            .optional()?;
        let Some((job, run_state)) = row else {
            return Ok(None);
        };
        if run_state != "running" {
            return find_in(tx, &job);
        }
        let (state, output, note, cost) = match &outcome {
            Outcome::Ok { output, cost_usd } | Outcome::Finished { output, cost_usd } => {
                ("ok", output.as_str(), "", *cost_usd)
            }
            Outcome::Quiet { cost_usd } => ("quiet", "", "", *cost_usd),
            Outcome::Failed { error } => ("failed", "", error.as_str(), None),
        };
        tx.execute(
            "UPDATE job_run SET state = ?2, finished = ?3, output = ?4, note = ?5, cost_usd = ?6 WHERE id = ?1",
            rusqlite::params![run, state, t, output, note, cost],
        )?;
        let Some(j) = find_in(tx, &job)? else {
            return Ok(None);
        };
        match outcome {
            Outcome::Failed { .. } => {
                let failures = j.failures + 1;
                if failures >= MAX_FAILURES {
                    tx.execute(
                        "UPDATE task SET failures = ?2, status = 'off', next_run = NULL WHERE id = ?1",
                        rusqlite::params![j.id, failures],
                    )?;
                } else if matches!(j.trigger, Trigger::Event { .. }) || j.status == "paused" {
                    tx.execute(
                        "UPDATE task SET failures = ?2 WHERE id = ?1",
                        rusqlite::params![j.id, failures],
                    )?;
                } else if j.action == TASK {
                    // The failed run itself waits for its retry (claim_task).
                    // next_run stays the next time of the rule. The claim
                    // keeps the job active, but an older AIfred made a
                    // one-time job "done" at its claim: it must be active
                    // for the retry.
                    tx.execute(
                        "UPDATE task SET failures = ?2, status = 'active' WHERE id = ?1",
                        rusqlite::params![j.id, failures],
                    )?;
                } else {
                    let retry = t + retry_wait(failures);
                    let next = j.next_run.map_or(retry, |n| n.min(retry));
                    tx.execute(
                        "UPDATE task SET failures = ?2, next_run = ?3, status = 'active' WHERE id = ?1",
                        rusqlite::params![j.id, failures, next],
                    )?;
                }
            }
            Outcome::Finished { .. } => {
                tx.execute(
                    "UPDATE task SET failures = 0, status = 'done', next_run = NULL WHERE id = ?1",
                    [&j.id],
                )?;
            }
            Outcome::Ok { .. } | Outcome::Quiet { .. } => {
                tx.execute("UPDATE task SET failures = 0 WHERE id = ?1", [&j.id])?;
            }
        }
        Ok(Some(get_in(tx, &j.id)?))
    })
}

/// Short answers that mean NOTHING, in lower case, with no punctuation.
const QUIET_ANSWERS: &[&str] = &[
    "nothing",
    "nothing to report",
    "nothing to tell",
    "nothing new",
];

/// The outcome of an agent answer: NOTHING is quiet, "DONE:" ends the job.
/// Capital letters and markdown marks ("**DONE:**", "Nothing.") do not matter.
pub fn agent_outcome(answer: &str, cost_usd: Option<f64>) -> Outcome {
    let a = answer.trim();
    let bare = a
        .trim_matches(|c: char| !c.is_ascii_alphanumeric())
        .to_ascii_lowercase();
    if bare.is_empty()
        || bare == NOTHING.to_ascii_lowercase()
        || QUIET_ANSWERS.contains(&bare.as_str())
    {
        return Outcome::Quiet { cost_usd };
    }
    // "DONE:", "**DONE:**", "**Done**:", "Done: ...": the marks around the word go.
    let mark = |c: char| matches!(c, '*' | '_' | '#' | '`') || c.is_whitespace();
    let word = DONE.trim_end_matches(':');
    let start = a.trim_start_matches(mark);
    if let Some(head) = start
        .get(..word.len())
        .filter(|h| h.eq_ignore_ascii_case(word))
        && let Some(rest) = start[head.len()..]
            .trim_start_matches(mark)
            .strip_prefix(':')
    {
        return Outcome::Finished {
            output: rest.trim_start_matches(mark).trim().to_string(),
            cost_usd,
        };
    }
    Outcome::Ok {
        output: a.to_string(),
        cost_usd,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-jobs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ts(local: &str, tz: &str) -> i64 {
        parse_local(local, zone(tz).unwrap()).unwrap().timestamp()
    }

    fn job(title: &str, trigger: Trigger, tz: &str) -> NewJob {
        NewJob {
            title: title.into(),
            action: "remind".into(),
            message: "Do it".into(),
            trigger: Some(trigger),
            tz: Some(tz.into()),
            created_by: "aifred".into(),
        }
    }

    fn repeat(start: &str, rule: &str) -> Trigger {
        Trigger::Repeat {
            start: start.into(),
            rule: rule.into(),
        }
    }

    #[test]
    fn rules_give_the_right_local_times() {
        let jkt = "Asia/Jakarta";
        // Every Monday at 09:00, asked on Thursday 5 Nov 2026.
        let mon = repeat("2026-11-05T09:00", "FREQ=WEEKLY;BYDAY=MO");
        let after = ts("2026-11-05T12:00", jkt);
        let got: Vec<String> = times_after(&mon, jkt, after, 3)
            .unwrap()
            .iter()
            .map(|t| show(*t, jkt))
            .collect();
        assert_eq!(
            got,
            [
                "Mon 9 Nov 2026 09:00",
                "Mon 16 Nov 2026 09:00",
                "Mon 23 Nov 2026 09:00"
            ]
        );
        // The 3rd and the 17th.
        let days = repeat("2026-11-05T09:00", "RRULE:FREQ=MONTHLY;BYMONTHDAY=3,17");
        let got: Vec<String> = times_after(&days, jkt, after, 3)
            .unwrap()
            .iter()
            .map(|t| show(*t, jkt))
            .collect();
        assert_eq!(
            got,
            [
                "Tue 17 Nov 2026 09:00",
                "Thu 3 Dec 2026 09:00",
                "Thu 17 Dec 2026 09:00"
            ]
        );
        // The last day of the month, also in February.
        let last = repeat("2027-01-01T17:00", "FREQ=MONTHLY;BYMONTHDAY=-1");
        let got: Vec<String> = times_after(&last, jkt, ts("2027-01-01T00:00", jkt), 2)
            .unwrap()
            .iter()
            .map(|t| show(*t, jkt))
            .collect();
        assert_eq!(got, ["Sun 31 Jan 2027 17:00", "Sun 28 Feb 2027 17:00"]);
        // Every other Thursday from 12 Nov.
        let other = repeat("2026-11-12T10:00", "FREQ=WEEKLY;INTERVAL=2;BYDAY=TH");
        let got: Vec<String> = times_after(&other, jkt, after, 2)
            .unwrap()
            .iter()
            .map(|t| show(*t, jkt))
            .collect();
        assert_eq!(got, ["Thu 12 Nov 2026 10:00", "Thu 26 Nov 2026 10:00"]);
        // The last Friday of the month.
        let fri = repeat("2026-11-05T16:00", "FREQ=MONTHLY;BYDAY=-1FR");
        assert_eq!(
            show(times_after(&fri, jkt, after, 1).unwrap()[0], jkt),
            "Fri 27 Nov 2026 16:00"
        );
        // A rule with an end.
        let five = repeat("2026-11-06T09:00", "FREQ=DAILY;COUNT=2");
        assert_eq!(times_after(&five, jkt, after, 10).unwrap().len(), 2);
    }

    #[test]
    fn summer_time_keeps_the_wall_time_and_rejects_missing_times() {
        let ny = "America/New_York";
        // Clocks go back on 1 Nov 2026: the daily 09:00 stays 09:00.
        let daily = repeat("2026-10-30T09:00", "FREQ=DAILY");
        let got: Vec<String> = times_after(&daily, ny, ts("2026-10-30T10:00", ny), 3)
            .unwrap()
            .iter()
            .map(|t| show(*t, ny))
            .collect();
        assert_eq!(
            got,
            [
                "Sat 31 Oct 2026 09:00",
                "Sun 1 Nov 2026 09:00",
                "Mon 2 Nov 2026 09:00"
            ]
        );
        // 02:30 on 8 Mar 2026 does not exist in New York.
        let e = parse_local("2026-03-08T02:30", zone(ny).unwrap()).unwrap_err();
        assert!(e.to_string().contains("summer time"), "{e}");
    }

    #[test]
    fn plain_words_for_the_common_rules() {
        let jkt = "Asia/Jakarta";
        assert_eq!(
            describe(&repeat("2026-11-09T09:00", "FREQ=WEEKLY;BYDAY=MO"), jkt),
            "Every Monday at 09:00 (Asia/Jakarta)"
        );
        assert_eq!(
            describe(
                &repeat("2026-11-09T09:00", "FREQ=MONTHLY;BYMONTHDAY=3,17"),
                jkt
            ),
            "Every month on day 3 and day 17 at 09:00 (Asia/Jakarta)"
        );
        assert_eq!(
            describe(
                &repeat("2026-11-12T10:00", "FREQ=WEEKLY;INTERVAL=2;BYDAY=TH"),
                jkt
            ),
            "Every 2 weeks on Thursday at 10:00 (Asia/Jakarta)"
        );
        assert_eq!(
            describe(&repeat("2026-11-12T16:00", "FREQ=MONTHLY;BYDAY=-1FR"), jkt),
            "Every month on the last Friday at 16:00 (Asia/Jakarta)"
        );
        assert_eq!(
            describe(
                &Trigger::Once {
                    at: "2026-11-13T15:00".into()
                },
                jkt
            ),
            "Once, Fri 13 Nov 2026 at 15:00 (Asia/Jakarta)"
        );
        assert!(
            describe(
                &Trigger::Event {
                    event: TASK_DONE.into(),
                    wait_secs: 90
                },
                ""
            )
            .starts_with("Each time you finish a task")
        );
    }

    #[test]
    fn create_checks_the_input_and_the_limit() {
        let d = dir("create");
        let jkt = "Asia/Jakarta";
        let past = job(
            "Old",
            Trigger::Once {
                at: "2020-01-01T09:00".into(),
            },
            jkt,
        );
        assert!(
            create(&d, past)
                .unwrap_err()
                .to_string()
                .contains("in the past")
        );
        let bad = job("Bad", repeat("2026-11-09T09:00", "FREQ=MINUTELY"), jkt);
        assert!(
            create(&d, bad)
                .unwrap_err()
                .to_string()
                .contains("once an hour")
        );
        let zone_err = job(
            "Zone",
            repeat("2030-11-09T09:00", "FREQ=DAILY"),
            "Mars/Base",
        );
        assert!(
            create(&d, zone_err)
                .unwrap_err()
                .to_string()
                .contains("IANA")
        );
        let ok = create(
            &d,
            job(
                "Timesheet",
                repeat("2030-11-04T09:00", "FREQ=WEEKLY;BYDAY=MO"),
                jkt,
            ),
        )
        .unwrap();
        assert_eq!(ok.next_run, Some(ts("2030-11-04T09:00", jkt)));
        assert_eq!(ok.status, "active");
        // The brief job is there from the start, so 19 more fit.
        for i in 0..(MAX_JOBS - 2) {
            create(
                &d,
                job(
                    &format!("J{i}"),
                    repeat("2030-11-04T09:00", "FREQ=DAILY"),
                    jkt,
                ),
            )
            .unwrap();
        }
        let e = create(
            &d,
            job(
                "One too many",
                repeat("2030-11-04T09:00", "FREQ=DAILY"),
                jkt,
            ),
        )
        .unwrap_err();
        assert!(e.to_string().contains("the most it can keep"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_task_job_adds_a_new_task_at_each_time_also_after_a_long_miss() {
        let d = dir("task-job");
        let jkt = "Asia/Jakarta";
        let mut new = job(
            "Pay rent",
            repeat("2030-12-01T08:00", "FREQ=MONTHLY;BYMONTHDAY=1"),
            jkt,
        );
        new.action = TASK.into();
        new.message = String::new();
        let rule = create(&d, new).unwrap();
        let store = crate::tasks::TaskStore::new(&d);
        assert!(
            store.load().unwrap().is_empty(),
            "the rule is not a task of the user"
        );

        let t = ts("2030-12-01T08:00", jkt);
        let c = claim_due(&d, t + 5).unwrap();
        assert_eq!(c.len(), 1);
        let made = make_task(&d, c[0].run, &c[0].job, c[0].planned).unwrap();
        assert_eq!(
            (made.title.as_str(), made.due.as_deref()),
            ("Pay rent", Some("2030-12-01"))
        );
        assert_eq!(made.added_by, "aifred");
        assert!(
            claim_due(&d, t + 10).unwrap().is_empty(),
            "one task for each time"
        );
        finish(
            &d,
            c[0].run,
            Outcome::Ok {
                output: String::new(),
                cost_usd: None,
            },
        )
        .unwrap();

        // AIfred was closed on 1 January. Nine days later, the task still comes.
        let jan = ts("2031-01-01T08:00", jkt);
        let c = claim_due(&d, jan + 9 * 86400).unwrap();
        assert_eq!(c.len(), 1, "a missed task is not skipped");
        assert_eq!(c[0].planned, jan);
        make_task(&d, c[0].run, &c[0].job, c[0].planned).unwrap();
        let dues: Vec<Option<String>> = store.load().unwrap().into_iter().map(|t| t.due).collect();
        assert_eq!(dues, [Some("2030-12-01".into()), Some("2031-01-01".into())]);
        assert_eq!(
            get(&d, &rule.id).unwrap().status,
            "active",
            "the rule stays"
        );

        // A task job cannot wait for an event.
        let e = update(
            &d,
            &rule.id,
            JobChange {
                trigger: Some(Trigger::Event {
                    event: TASK_DONE.into(),
                    wait_secs: 90,
                }),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("not an event"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Runs claim_due and make_task until no claim is left, as the scheduler
    /// does tick by tick. Gives the due days of the new tasks.
    fn run_task_ticks(d: &Path, now: i64) -> Vec<String> {
        let mut dues = vec![];
        for _ in 0..100 {
            let c = claim_due(d, now).unwrap();
            if c.is_empty() {
                break;
            }
            assert_eq!(c.len(), 1, "one task for each tick");
            let t = make_task(d, c[0].run, &c[0].job, c[0].planned).unwrap();
            dues.push(t.due.unwrap());
            finish(
                d,
                c[0].run,
                Outcome::Ok {
                    output: String::new(),
                    cost_usd: None,
                },
            )
            .unwrap();
        }
        dues
    }

    #[test]
    fn a_task_job_adds_a_task_for_each_missed_time() {
        let d = dir("task-missed");
        let utc = "UTC";
        let mut new = job(
            "Submit timesheet",
            repeat("2030-01-07T09:00", "FREQ=WEEKLY;BYDAY=MO"),
            utc,
        );
        new.action = TASK.into();
        create(&d, new).unwrap();
        // AIfred is closed for 3 weeks: 7, 14 and 21 January.
        let dues = run_task_ticks(&d, ts("2030-01-22T10:00", utc));
        assert_eq!(dues, ["2030-01-07", "2030-01-14", "2030-01-21"]);
        let skipped = runs(&d, None, 50, true, true)
            .unwrap()
            .into_iter()
            .filter(|r| r.state == "skipped")
            .count();
        assert_eq!(skipped, 0);
        std::fs::remove_dir_all(&d).unwrap();

        // After a very long miss, only the newest MAX_CATCH_UP times get a task.
        let d = dir("task-missed-long");
        let mut new = job(
            "Water the plants",
            repeat("2030-01-01T09:00", "FREQ=DAILY"),
            utc,
        );
        new.action = TASK.into();
        create(&d, new).unwrap();
        let dues = run_task_ticks(&d, ts("2030-02-10T10:00", utc));
        assert_eq!(dues.len(), MAX_CATCH_UP);
        assert_eq!(dues.last().unwrap(), "2030-02-10");
        let skipped = runs(&d, None, 200, true, true)
            .unwrap()
            .into_iter()
            .filter(|r| r.state == "skipped")
            .count();
        assert_eq!(
            skipped, MAX_SKIP_ROWS,
            "41 times, 20 tasks, at most 20 skipped rows"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_task_retry_keeps_the_day_of_the_planned_time() {
        let d = dir("task-retry");
        let utc = "UTC";
        // Days near the real clock: finish() writes the real time, and the
        // hourly cleanup deletes runs that look 60 days old.
        let day = |n: i64| {
            (Utc::now() + chrono::Duration::days(n))
                .format("%Y-%m-%d")
                .to_string()
        };
        let (day1, day2) = (day(1), day(2));
        let mut new = job(
            "Log the day",
            repeat(&format!("{day1}T23:59"), "FREQ=DAILY"),
            utc,
        );
        new.action = TASK.into();
        create(&d, new).unwrap();
        let t = ts(&format!("{day1}T23:59"), utc);
        let c = claim_due(&d, t + 5).unwrap();
        assert_eq!(c.len(), 1);
        let first_run = c[0].run;
        finish(
            &d,
            c[0].run,
            Outcome::Failed {
                error: "the database is busy".into(),
            },
        )
        .unwrap();
        // The retry comes after midnight, on the next day.
        let retry = t + 5 * 60;
        let c = claim_due(&d, retry).unwrap();
        assert_eq!(c.len(), 1, "the retry");
        assert_eq!(c[0].planned, t, "the retry keeps the planned time");
        assert_eq!(c[0].run, first_run, "the retry uses the same run row");
        let made = make_task(&d, c[0].run, &c[0].job, c[0].planned).unwrap();
        assert_eq!(made.due.as_deref(), Some(day1.as_str()));
        finish(
            &d,
            c[0].run,
            Outcome::Ok {
                output: String::new(),
                cost_usd: None,
            },
        )
        .unwrap();
        // The time of the next day still gives its own task.
        let dues = run_task_ticks(&d, ts(&format!("{day2}T23:59"), utc) + 5);
        assert_eq!(dues, [day2]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A daily "task" job whose times are the last `days` days, at the real
    /// time of now. finish() and recover() write the real time, so the
    /// retry waits must use a clock near it.
    fn daily_task_job(d: &Path, title: &str, days: i64) -> (Job, i64) {
        let utc = "UTC";
        let start = Utc::now() - chrono::Duration::days(days);
        let start_text = start.format("%Y-%m-%dT%H:%M").to_string();
        let mut new = job(title, repeat(&start_text, "FREQ=DAILY"), utc);
        new.action = TASK.into();
        // create() wants a time after now: a rule that started in the past.
        let rule = create(d, new).unwrap();
        db::write_jobs(d, |tx| {
            tx.execute(
                "UPDATE task SET next_run = ?2 WHERE id = ?1",
                rusqlite::params![rule.id, ts(&start_text, utc)],
            )?;
            Ok(())
        })
        .unwrap();
        (get(d, &rule.id).unwrap(), ts(&start_text, utc))
    }

    fn task_count(d: &Path) -> usize {
        crate::tasks::TaskStore::new(d).load().unwrap().len()
    }

    #[test]
    fn a_crash_during_a_task_run_gives_exactly_one_task() {
        let d = dir("task-crash");
        let (_, first) = daily_task_job(&d, "Log the day", 0);
        let t = now() as i64;
        // AIfred stops after the claim, before the task is saved.
        let c = claim_due(&d, t).unwrap();
        assert_eq!((c.len(), c[0].planned), (1, first));
        assert_eq!(recover(&d).unwrap(), 1);
        assert_eq!(task_count(&d), 0);
        assert!(
            claim_due(&d, t + 5).unwrap().is_empty(),
            "the retry waits a minute"
        );
        let c = claim_due(&d, t + 90).unwrap();
        assert_eq!(
            (c.len(), c[0].planned),
            (1, first),
            "the stopped time gets its retry"
        );
        // This time AIfred stops after the task is saved, before finish().
        make_task(&d, c[0].run, &c[0].job, c[0].planned).unwrap();
        assert_eq!(recover(&d).unwrap(), 0, "the run ended with the task");
        assert!(claim_due(&d, t + 600).unwrap().is_empty(), "no second task");
        assert_eq!(task_count(&d), 1);
        // A late finish() of the same run changes nothing.
        finish(
            &d,
            c[0].run,
            Outcome::Failed {
                error: "late".into(),
            },
        )
        .unwrap();
        assert_eq!(get(&d, &c[0].job.id).unwrap().failures, 0);
        assert!(claim_due(&d, t + 1200).unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_failed_task_time_retries_after_a_pause_and_waits_during_a_catch_up() {
        let d = dir("task-pause");
        // Three missed days: the catch-up adds them one tick after the other.
        let (rule, first) = daily_task_job(&d, "Water the plants", 2);
        let t = now() as i64;
        let c = claim_due(&d, t).unwrap();
        assert_eq!(c[0].planned, first);
        finish(
            &d,
            c[0].run,
            Outcome::Failed {
                error: "the database is busy".into(),
            },
        )
        .unwrap();
        // The next missed day is in the past, but the failed time waits for
        // its retry, and no other time starts meanwhile.
        assert!(get(&d, &rule.id).unwrap().next_run.unwrap() < t);
        assert!(
            claim_due(&d, t + 30).unwrap().is_empty(),
            "no retry at once"
        );
        assert!(
            !db::read(&d, |c| any_due(c, t + 30)).unwrap(),
            "no write lock each tick"
        );
        // The user pauses the job, then starts it again.
        let paused = JobChange {
            status: Some("paused".into()),
            ..Default::default()
        };
        update(&d, &rule.id, paused).unwrap();
        let start = JobChange {
            status: Some("active".into()),
            ..Default::default()
        };
        update(&d, &rule.id, start).unwrap();
        let c = claim_due(&d, t + 90).unwrap();
        assert_eq!(c.len(), 1, "the failed time still gets its task");
        assert_eq!(c[0].planned, first);
        make_task(&d, c[0].run, &c[0].job, c[0].planned).unwrap();
        assert_eq!(task_count(&d), 1);

        // A new time replaces the old ones: a failed old time gets no task,
        // and the run list says why.
        let (rent, _) = daily_task_job(&d, "Pay rent", 0);
        let c = run_until_claim(&d, t + 120);
        assert_eq!(c.job.id, rent.id);
        finish(
            &d,
            c.run,
            Outcome::Failed {
                error: "disk full".into(),
            },
        )
        .unwrap();
        let tomorrow = (Utc::now() + chrono::Duration::days(1))
            .format("%Y-%m-%dT09:00")
            .to_string();
        let change = JobChange {
            trigger: Some(repeat(&tomorrow, "FREQ=DAILY")),
            ..Default::default()
        };
        update(&d, &rent.id, change).unwrap();
        assert!(claim_due(&d, t + 600).unwrap().is_empty());
        assert_eq!(task_count(&d), 1, "no task for the old time");
        let r = runs(&d, Some(&rent.id), 10, true, true).unwrap();
        let old = r.iter().find(|r| r.id == c.run).unwrap();
        assert_eq!(old.state, "skipped");
        assert!(
            old.note.contains("new time") && old.note.contains("disk full"),
            "{}",
            old.note
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A one-time "task" job whose time was a minute ago. create() wants a
    /// time after now, so the test moves the time back.
    fn once_task_job(d: &Path, title: &str) -> Job {
        let utc = "UTC";
        let at = (Utc::now() - chrono::Duration::minutes(1))
            .format("%Y-%m-%dT%H:%M")
            .to_string();
        let mut new = job(
            title,
            Trigger::Once {
                at: (Utc::now() + chrono::Duration::days(1))
                    .format("%Y-%m-%dT%H:%M")
                    .to_string(),
            },
            utc,
        );
        new.action = TASK.into();
        let j = create(d, new).unwrap();
        db::write_jobs(d, |tx| {
            tx.execute(
                "UPDATE task SET trigger = ?2, next_run = ?3 WHERE id = ?1",
                rusqlite::params![
                    j.id,
                    serde_json::to_string(&Trigger::Once { at: at.clone() })?,
                    ts(&at, utc)
                ],
            )?;
            Ok(())
        })
        .unwrap();
        get(d, &j.id).unwrap()
    }

    fn fail(d: &Path, run: i64) -> Job {
        finish(
            d,
            run,
            Outcome::Failed {
                error: "the database is locked".into(),
            },
        )
        .unwrap()
        .unwrap()
    }

    fn set_status(d: &Path, id: &str, status: &str) -> anyhow::Result<Job> {
        update(
            d,
            id,
            JobChange {
                status: Some(status.into()),
                ..Default::default()
            },
        )
    }

    #[test]
    fn a_task_job_that_turned_off_retries_its_failed_time_after_a_start() {
        let d = dir("task-off");
        let j = once_task_job(&d, "Send the invoice");
        // A long wait after each failure, so each claim is the retry.
        let mut t = now() as i64;
        for _ in 0..MAX_FAILURES {
            t += 3600;
            let c = run_until_claim(&d, t);
            fail(&d, c.run);
        }
        let off = get(&d, &j.id).unwrap();
        assert_eq!((off.status.as_str(), off.next_run), ("off", None));
        assert!(off.can_start, "Start shows: the failed time waits");
        let on = set_status(&d, &j.id, "active").unwrap();
        assert_eq!((on.status.as_str(), on.next_run), ("active", None));
        let c = run_until_claim(&d, t + 3600);
        make_task(&d, c.run, &c.job, c.planned).unwrap();
        assert_eq!(task_count(&d), 1);
        assert_eq!(get(&d, &j.id).unwrap().status, "done");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_paused_one_time_task_job_with_a_retry_has_no_next_run_after_a_start() {
        let d = dir("task-once-pause");
        let j = once_task_job(&d, "Book the room");
        let c = run_until_claim(&d, now() as i64);
        fail(&d, c.run);
        set_status(&d, &j.id, "paused").unwrap();
        let on = set_status(&d, &j.id, "active").unwrap();
        assert_eq!(on.next_run, None, "the time has its run already");
        let c = run_until_claim(&d, now() as i64 + 3600);
        make_task(&d, c.run, &c.job, c.planned).unwrap();
        assert_eq!(get(&d, &j.id).unwrap().status, "done", "done at once");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_cleanup_keeps_a_failed_task_time_only_while_a_retry_can_come() {
        let d = dir("task-cleanup");
        let keep = once_task_job(&d, "Keep");
        let gone = once_task_job(&d, "Gone");
        for c in claim_due(&d, now() as i64).unwrap() {
            fail(&d, c.run);
        }
        set_status(&d, &keep.id, "paused").unwrap();
        db::write_jobs(&d, |tx| {
            tx.execute("UPDATE task SET status = 'off' WHERE id = ?1", [&gone.id])?;
            Ok(())
        })
        .unwrap();
        // The cleanup runs at the first claim of this process, and then once
        // an hour. A time far ahead is after the hour, and after KEEP_RUNS.
        let later = now() as i64 + KEEP_RUNS + 2 * 3600;
        claim_due(&d, later).unwrap();
        let left = |id: &str| runs(&d, Some(id), 10, true, true).unwrap().len();
        assert_eq!(left(&keep.id), 1, "the paused job can still retry it");
        assert_eq!(left(&gone.id), 0, "nothing retries it");
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn run_until_claim(d: &Path, now: i64) -> Claim {
        let mut c = claim_due(d, now).unwrap();
        assert_eq!(c.len(), 1);
        c.remove(0)
    }

    #[test]
    fn a_task_job_after_a_very_long_miss_adds_only_the_newest_times() {
        // An hourly rule missed for 30 days: 720 times, more than one page
        // of the rule (500). Only the newest 20 get a task.
        let d = dir("task-hourly");
        let utc = "UTC";
        let mut new = job(
            "Check the oven",
            repeat("2030-01-01T00:00", "FREQ=HOURLY"),
            utc,
        );
        new.action = TASK.into();
        create(&d, new).unwrap();
        let now = ts("2030-01-31T00:30", utc);
        let mut planned = vec![];
        for _ in 0..30 {
            let c = claim_due(&d, now).unwrap();
            if c.is_empty() {
                break;
            }
            planned.push(c[0].planned);
            make_task(&d, c[0].run, &c[0].job, c[0].planned).unwrap();
        }
        assert_eq!(planned.len(), MAX_CATCH_UP);
        assert_eq!(
            *planned.last().unwrap(),
            ts("2030-01-31T00:00", utc),
            "the newest time"
        );
        assert_eq!(planned[0], ts("2030-01-30T05:00", utc));
        let skipped: Vec<Run> = runs(&d, None, 200, true, true)
            .unwrap()
            .into_iter()
            .filter(|r| r.state == "skipped")
            .collect();
        assert_eq!(skipped.len(), MAX_SKIP_ROWS);
        // 721 times, 20 tasks, 20 rows: 681 times in the note of the oldest row.
        let oldest = skipped.iter().min_by_key(|r| r.planned).unwrap();
        assert!(oldest.note.contains("681 earlier times"), "{}", oldest.note);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_change_of_the_user_tasks_cannot_touch_a_job() {
        let d = dir("job-guard");
        crate::tasks::TaskStore::new(&d)
            .add("Call Sam", "", "you")
            .unwrap();
        let bulk = db::write(&d, |tx| {
            Ok(tx.execute("UPDATE task SET due = '2030-01-01'", [])?)
        });
        assert!(
            bulk.unwrap_err()
                .to_string()
                .contains("cannot change AIfred's jobs")
        );
        let gone = db::write(&d, |tx| Ok(tx.execute("DELETE FROM task", [])?));
        assert!(
            gone.unwrap_err()
                .to_string()
                .contains("cannot delete AIfred's jobs")
        );
        assert!(get(&d, BRIEF_JOB).is_ok());
        assert_eq!(task_count(&d), 1, "nothing changed");
        let mine = db::write(&d, |tx| {
            Ok(tx.execute("UPDATE task SET due = '2030-01-01' WHERE doer = 'you'", [])?)
        });
        assert_eq!(mine.unwrap(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_run_list_can_leave_out_the_brief_runs() {
        let d = dir("brief-runs");
        let j = create(
            &d,
            job("Timesheet", repeat("2030-01-07T09:00", "FREQ=DAILY"), "UTC"),
        )
        .unwrap();
        db::write(&d, |tx| {
            for i in 0..3 {
                add_run(tx, BRIEF_JOB, 100 + i, "ok", "", Some(100 + i), false)?;
            }
            add_run(tx, &j.id, 50, "ok", "", Some(50), false)?;
            Ok(())
        })
        .unwrap();
        let jobs_of = |brief: bool| -> Vec<String> {
            runs(&d, None, 2, true, brief)
                .unwrap()
                .into_iter()
                .map(|r| r.job)
                .collect()
        };
        assert_eq!(
            jobs_of(true),
            [BRIEF_JOB, BRIEF_JOB],
            "the brief runs fill the list"
        );
        assert_eq!(jobs_of(false), [j.id]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_job_runs_once_for_its_time() {
        let d = dir("once");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job("Call", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt),
        )
        .unwrap();
        let t = ts("2030-11-04T09:00", jkt);
        assert!(claim_due(&d, t - 1).unwrap().is_empty(), "not yet");
        let c = claim_due(&d, t + 5).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].planned, c[0].late), (t, false));
        assert_eq!(
            c[0].job.next_run,
            Some(t + 86400),
            "the next run moved at the claim"
        );
        assert!(
            claim_due(&d, t + 10).unwrap().is_empty(),
            "a second claim gets nothing"
        );
        let after = finish(
            &d,
            c[0].run,
            Outcome::Ok {
                output: "Done".into(),
                cost_usd: None,
            },
        )
        .unwrap()
        .unwrap();
        assert_eq!(after.status, "active");
        assert_eq!(
            runs(&d, Some(&j.id), 10, true, true).unwrap()[0].state,
            "ok"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missed_times_give_one_late_run() {
        let d = dir("missed");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job("Daily", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt),
        )
        .unwrap();
        // The computer sleeps from Mon 08:00 to Wed 10:00: 3 times pass.
        let wake = ts("2030-11-06T10:00", jkt);
        let c = claim_due(&d, wake).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(
            c[0].planned,
            ts("2030-11-06T09:00", jkt),
            "the newest time runs"
        );
        assert!(c[0].late);
        let all = runs(&d, Some(&j.id), 10, true, true).unwrap();
        assert_eq!(all.iter().filter(|r| r.state == "skipped").count(), 2);
        assert!(
            runs(&d, Some(&j.id), 10, false, true)
                .unwrap()
                .iter()
                .all(|r| r.state != "skipped")
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_repeat_that_is_too_old_waits_for_the_next_time_and_a_once_runs_late() {
        let d = dir("stale");
        let jkt = "Asia/Jakarta";
        let daily = create(
            &d,
            job(
                "Daily",
                repeat("2030-11-04T09:00", "FREQ=WEEKLY;BYDAY=MO"),
                jkt,
            ),
        )
        .unwrap();
        let once = create(
            &d,
            job(
                "Once",
                Trigger::Once {
                    at: "2030-11-04T09:00".into(),
                },
                jkt,
            ),
        )
        .unwrap();
        // Mon 09:00 + 13 hours.
        let c = claim_due(&d, ts("2030-11-04T22:00", jkt)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].job.id, once.id, "the one-time reminder still comes");
        assert_eq!(c[0].job.status, "done", "it has no more times");
        let daily = get(&d, &daily.id).unwrap();
        assert_eq!(daily.next_run, Some(ts("2030-11-11T09:00", jkt)));
        assert_eq!(
            runs(&d, Some(&daily.id), 5, true, true).unwrap()[0].note,
            "Too late: AIfred was not running at this time."
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failures_retry_then_turn_the_job_off() {
        let d = dir("fail");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job("Weekly", repeat("2030-11-04T09:00", "FREQ=WEEKLY"), jkt),
        )
        .unwrap();
        let mut t = ts("2030-11-04T09:00", jkt);
        for i in 1..=MAX_FAILURES {
            let c = claim_due(&d, t).unwrap();
            assert_eq!(c.len(), 1, "run {i}");
            let after = finish(
                &d,
                c[0].run,
                Outcome::Failed {
                    error: "no network".into(),
                },
            )
            .unwrap()
            .unwrap();
            assert_eq!(after.failures, i);
            if i < MAX_FAILURES {
                // The retry is soon, not next week. `finish` uses the real
                // clock, so the test moves to the planned retry time.
                t = after.next_run.unwrap();
                assert!(t < ts("2030-11-11T09:00", jkt));
            } else {
                assert_eq!((after.status.as_str(), after.next_run), ("off", None));
            }
        }
        // Back on: the failures count from zero, and missed times do not run.
        let on = update(
            &d,
            &j.id,
            JobChange {
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!((on.status.as_str(), on.failures), ("active", 0));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_busy_run_and_a_restart_never_run_twice() {
        let d = dir("busy");
        let jkt = "Asia/Jakarta";
        create(
            &d,
            job("Hourly", repeat("2030-11-04T09:00", "FREQ=HOURLY"), jkt),
        )
        .unwrap();
        let t = ts("2030-11-04T09:00", jkt);
        let first = claim_due(&d, t).unwrap();
        assert_eq!(first.len(), 1);
        // One hour later the first run is still busy: the time is skipped.
        assert!(claim_due(&d, t + 3600).unwrap().is_empty());
        // AIfred stops and starts again: the busy run is failed, not run again.
        assert_eq!(recover(&d).unwrap(), 1);
        let r = runs(&d, None, 10, true, true).unwrap();
        assert!(r.iter().any(|r| r.note == "The last run was still busy."));
        assert!(r.iter().any(|r| r.note == "AIfred stopped during the run."));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn many_events_make_one_run_after_a_quiet_moment() {
        let d = dir("events");
        let brief = get(&d, BRIEF_JOB).unwrap();
        assert_eq!(brief.action, "brief");
        let store = crate::tasks::TaskStore::new(&d);
        for i in 0..8 {
            let t = store.add(&format!("Task {i}"), "", "you").unwrap();
            store.set_done(&t.id, true).unwrap();
        }
        let t = now() as i64;
        assert!(
            claim_due(&d, t).unwrap().is_empty(),
            "it waits for more events"
        );
        assert!(get(&d, BRIEF_JOB).unwrap().next_run.unwrap() >= t + 89);
        let c = claim_due(&d, t + 91).unwrap();
        assert_eq!(c.len(), 1, "8 tasks give one run");
        assert!(
            claim_due(&d, t + 200).unwrap().is_empty(),
            "the events are used"
        );
        // A task that is opened again is not an event.
        let x = store.add("Again", "", "you").unwrap();
        store.set_done(&x.id, false).unwrap();
        assert!(claim_due(&d, t + 1000).unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn events_wait_at_most_ten_minutes() {
        let d = dir("maxwait");
        let t = now() as i64;
        db::write(&d, |tx| {
            for i in 0..20 {
                tx.execute(
                    "INSERT INTO job_event (name, ts) VALUES (?1, ?2)",
                    rusqlite::params![TASK_DONE, t + i * 60],
                )?;
            }
            Ok(())
        })
        .unwrap();
        // Events each minute: the run comes 10 minutes after the first one.
        assert!(claim_due(&d, t + EVENT_MAX_WAIT - 1).unwrap().is_empty());
        assert_eq!(claim_due(&d, t + EVENT_MAX_WAIT).unwrap().len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn pause_resume_and_a_new_time() {
        let d = dir("edit");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job(
                "Mon",
                repeat("2030-11-04T09:00", "FREQ=WEEKLY;BYDAY=MO"),
                jkt,
            ),
        )
        .unwrap();
        let p = update(
            &d,
            &j.id,
            JobChange {
                status: Some("paused".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(p.status, "paused");
        assert!(
            claim_due(&d, ts("2030-11-04T09:00", jkt))
                .unwrap()
                .is_empty(),
            "a paused job does not run"
        );
        let tue = update(
            &d,
            &j.id,
            JobChange {
                trigger: Some(repeat("2030-11-05T09:00", "FREQ=WEEKLY;BYDAY=TU")),
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(tue.next_run, Some(ts("2030-11-05T09:00", jkt)));
        assert_eq!(tue.when, "Every Tuesday at 09:00 (Asia/Jakarta)");
        delete(&d, &j.id).unwrap();
        assert!(get(&d, &j.id).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn agent_answers() {
        assert_eq!(
            agent_outcome("NOTHING", None),
            Outcome::Quiet { cost_usd: None }
        );
        assert_eq!(
            agent_outcome(" **NOTHING.** ", None),
            Outcome::Quiet { cost_usd: None }
        );
        assert_eq!(
            agent_outcome("Nothing.", None),
            Outcome::Quiet { cost_usd: None }
        );
        assert_eq!(
            agent_outcome("Nothing to report.", None),
            Outcome::Quiet { cost_usd: None }
        );
        let merged = Outcome::Finished {
            output: "PR #123 is merged.".into(),
            cost_usd: Some(0.1),
        };
        assert_eq!(agent_outcome("DONE: PR #123 is merged.", Some(0.1)), merged);
        assert_eq!(
            agent_outcome("**DONE:** PR #123 is merged.", Some(0.1)),
            merged
        );
        assert_eq!(
            agent_outcome("**Done**: PR #123 is merged.", Some(0.1)),
            merged
        );
        assert!(matches!(
            agent_outcome("PR #123 is still open.", None),
            Outcome::Ok { .. }
        ));
        assert!(matches!(
            agent_outcome("Nothing changed, but the PR has a new review.", None),
            Outcome::Ok { .. }
        ));
        assert!(matches!(
            agent_outcome("Done reviewing: 2 comments.", None),
            Outcome::Ok { .. }
        ));
    }

    #[test]
    fn a_rule_can_repeat_at_most_once_an_hour() {
        for bad in [
            "FREQ=MINUTELY",
            "FREQ=HOURLY;BYMINUTE=0,5,10",
            "FREQ=DAILY;BYHOUR=9;BYMINUTE=0,1",
            "FREQ=HOURLY;BYSECOND=0,30",
        ] {
            assert!(
                clean_rule(bad)
                    .unwrap_err()
                    .to_string()
                    .contains("once an hour"),
                "{bad}"
            );
        }
        for good in [
            "FREQ=HOURLY",
            "FREQ=DAILY;BYHOUR=9,17;BYMINUTE=30",
            "FREQ=HOURLY;BYMINUTE=15;BYSECOND=0",
        ] {
            clean_rule(good).unwrap();
        }
    }

    #[test]
    fn a_time_in_the_repeated_hour_keeps_its_offset() {
        let ny = "America/New_York";
        // 1 Nov 2026: 01:30 comes first in EDT (-04:00), then in EST (-05:00).
        let second = ts("2026-11-01T01:30-05:00", ny);
        let first = ts("2026-11-01T01:30", ny);
        assert_eq!(second - first, 3600);
        let text = local_text_exact(second, ny).unwrap();
        assert_eq!(text, "2026-11-01T01:30-05:00");
        assert_eq!(
            parse_local(&text, zone(ny).unwrap()).unwrap().timestamp(),
            second
        );
        // A plain time has no offset.
        assert_eq!(
            local_text_exact(ts("2026-11-02T09:00", ny), ny).unwrap(),
            "2026-11-02T09:00"
        );
    }

    #[test]
    fn a_paused_one_time_job_runs_late_after_its_time() {
        let d = dir("pausedonce");
        let jkt = "Asia/Jakarta";
        let t = now() as i64;
        let at = local_text(t + 3600, jkt).unwrap();
        let j = create(&d, job("Call Sam", Trigger::Once { at }, jkt)).unwrap();
        update(
            &d,
            &j.id,
            JobChange {
                status: Some("paused".into()),
                ..Default::default()
            },
        )
        .unwrap();
        // The time passes during the pause.
        let past = local_text(t - 3600, jkt).unwrap();
        let planned = ts(&past, jkt);
        db::write_jobs(&d, |tx| {
            let trig = serde_json::to_string(&Trigger::Once { at: past.clone() })?;
            tx.execute(
                "UPDATE task SET trigger = ?2 WHERE id = ?1",
                rusqlite::params![j.id, trig],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(get(&d, &j.id).unwrap().can_start, "Start shows");
        let back = update(
            &d,
            &j.id,
            JobChange {
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (back.status.as_str(), back.next_run),
            ("active", Some(planned))
        );
        let c = claim_due(&d, t).unwrap();
        assert_eq!(
            (c.len(), c[0].late),
            (1, true),
            "it runs at once, as a late run"
        );
        // Done now: no more times, so Start does not show and does not work.
        let done = get(&d, &j.id).unwrap();
        assert_eq!((done.status.as_str(), done.can_start), ("done", false));
        let e = update(
            &d,
            &j.id,
            JobChange {
                status: Some("active".into()),
                ..Default::default()
            },
        )
        .unwrap_err();
        assert!(e.to_string().contains("no more times"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_busy_event_job_checks_again_after_a_minute() {
        let d = dir("busyevent");
        let t = now() as i64;
        let store = crate::tasks::TaskStore::new(&d);
        let x = store.add("One", "", "you").unwrap();
        store.set_done(&x.id, true).unwrap();
        let c = claim_due(&d, t + 91).unwrap();
        assert_eq!(c.len(), 1);
        // A new event while the run is busy (after the claim at t + 91).
        db::write(&d, |tx| {
            tx.execute(
                "INSERT INTO job_event (name, ts) VALUES (?1, ?2)",
                rusqlite::params![TASK_DONE, t + 100],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            claim_due(&d, t + 200).unwrap().is_empty(),
            "the run is still busy"
        );
        let next = get(&d, BRIEF_JOB).unwrap().next_run.unwrap();
        assert_eq!(next, t + 200 + BUSY_RECHECK_SECS);
        // Not due on the next tick: that tick takes no write lock.
        assert!(!db::read(&d, |c| any_due(c, t + 230)).unwrap());
        finish(&d, c[0].run, Outcome::Quiet { cost_usd: None }).unwrap();
        assert_eq!(
            claim_due(&d, t + 260).unwrap().len(),
            1,
            "the new event runs after the busy run"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_one_window_holds_the_scheduler_lock() {
        let d = dir("schedlock");
        let first = SchedulerLock::take(&d).unwrap();
        assert!(first.is_some());
        assert!(
            SchedulerLock::take(&d).unwrap().is_none(),
            "a second window waits"
        );
        drop(first);
        assert!(
            SchedulerLock::take(&d).unwrap().is_some(),
            "it takes over when the first one stops"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_time_starts_an_ended_job_again() {
        let d = dir("revive");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job(
                "Call",
                Trigger::Once {
                    at: "2030-11-04T09:00".into(),
                },
                jkt,
            ),
        )
        .unwrap();
        let c = claim_due(&d, ts("2030-11-04T09:00", jkt)).unwrap();
        assert_eq!(c[0].job.status, "done");
        let moved = update(
            &d,
            &j.id,
            JobChange {
                trigger: Some(Trigger::Once {
                    at: "2030-11-08T10:00".into(),
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            (moved.status.as_str(), moved.next_run),
            ("active", Some(ts("2030-11-08T10:00", jkt)))
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_change_to_an_event_ignores_the_old_events() {
        let d = dir("toevent");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job("Daily", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt),
        )
        .unwrap();
        let store = crate::tasks::TaskStore::new(&d);
        let t = store.add("Old task", "", "you").unwrap();
        store.set_done(&t.id, true).unwrap();
        // Pause the brief job, so only this job can run.
        update(
            &d,
            BRIEF_JOB,
            JobChange {
                status: Some("paused".into()),
                ..Default::default()
            },
        )
        .unwrap();
        update(
            &d,
            &j.id,
            JobChange {
                trigger: Some(Trigger::Event {
                    event: TASK_DONE.into(),
                    wait_secs: 10,
                }),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            claim_due(&d, now() as i64 + 3600).unwrap().is_empty(),
            "the old task does not count"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_bad_job_turns_off_and_the_others_still_run() {
        let d = dir("bad");
        let jkt = "Asia/Jakarta";
        let good = create(
            &d,
            job("Good", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt),
        )
        .unwrap();
        let bad = create(
            &d,
            job("Bad", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt),
        )
        .unwrap();
        db::write_jobs(&d, |tx| {
            tx.execute("UPDATE task SET tz = 'Mars/Base' WHERE id = ?1", [&bad.id])?;
            Ok(())
        })
        .unwrap();
        let c = claim_due(&d, ts("2030-11-04T09:00", jkt)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].job.id, good.id);
        let bad = get(&d, &bad.id).unwrap();
        assert_eq!((bad.status.as_str(), bad.next_run), ("off", None));
        let r = &runs(&d, Some(&bad.id), 5, true, true).unwrap()[0];
        assert_eq!(r.state, "failed");
        assert!(r.note.contains("time zone"), "{}", r.note);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_long_gap_writes_at_most_twenty_skipped_rows() {
        let d = dir("gap");
        let jkt = "Asia/Jakarta";
        let j = create(
            &d,
            job("Hourly", repeat("2030-11-04T09:00", "FREQ=HOURLY"), jkt),
        )
        .unwrap();
        // Closed for 3 days: 72 hourly times pass.
        let c = claim_due(&d, ts("2030-11-07T09:30", jkt)).unwrap();
        assert_eq!(c.len(), 1);
        let skipped: Vec<Run> = runs(&d, Some(&j.id), 200, true, true)
            .unwrap()
            .into_iter()
            .filter(|r| r.state == "skipped")
            .collect();
        assert_eq!(skipped.len(), MAX_SKIP_ROWS);
        assert!(
            skipped.last().unwrap().note.contains("52 earlier times"),
            "{}",
            skipped.last().unwrap().note
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_brief_job_cannot_be_deleted() {
        let d = dir("keepbrief");
        assert!(
            delete(&d, BRIEF_JOB)
                .unwrap_err()
                .to_string()
                .contains("Pause")
        );
        assert!(get(&d, BRIEF_JOB).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_one_time_task_job_is_done_only_when_its_task_is_there() {
        let d = dir("task-done-late");
        let j = once_task_job(&d, "Send the invoice");
        let c = run_until_claim(&d, now() as i64);
        assert!(!c.retry, "the first claim is not a retry");
        let claimed = get(&d, &j.id).unwrap();
        assert_eq!(
            (claimed.status.as_str(), claimed.next_run),
            ("active", None),
            "no task yet: not done"
        );
        let c = fail(&d, c.run);
        assert_eq!(c.status, "active");
        let c = run_until_claim(&d, now() as i64 + 3600);
        assert!(c.retry, "the second claim tries the failed time again");
        make_task(&d, c.run, &c.job, c.planned).unwrap();
        assert_eq!(get(&d, &j.id).unwrap().status, "done");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_one_time_task_job_whose_time_has_a_task_is_done_after_a_start() {
        let d = dir("task-done-start");
        let j = once_task_job(&d, "Send the invoice");
        let c = run_until_claim(&d, now() as i64);
        // The user pauses the job during the run. The task comes, but the
        // job stays paused.
        set_status(&d, &j.id, "paused").unwrap();
        make_task(&d, c.run, &c.job, c.planned).unwrap();
        assert_eq!(get(&d, &j.id).unwrap().status, "paused");
        // A start gives the old time again, and that time has its task.
        set_status(&d, &j.id, "active").unwrap();
        claim_due(&d, now() as i64).unwrap();
        let after = get(&d, &j.id).unwrap();
        assert_eq!((after.status.as_str(), after.next_run), ("done", None));
        assert_eq!(task_count(&d), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_job_that_cannot_be_read_does_not_hide_the_other_jobs() {
        let d = dir("list-bad-row");
        let good = create(
            &d,
            job(
                "Good",
                Trigger::Event {
                    event: "task_done".into(),
                    wait_secs: 60,
                },
                "UTC",
            ),
        )
        .unwrap();
        let bad = create(
            &d,
            job(
                "Bad",
                Trigger::Event {
                    event: "task_done".into(),
                    wait_secs: 60,
                },
                "UTC",
            ),
        )
        .unwrap();
        db::write_jobs(&d, |tx| {
            tx.execute(
                "UPDATE task SET trigger = 'not json', status = 'paused' WHERE id = ?1",
                [&bad.id],
            )?;
            Ok(())
        })
        .unwrap();
        let ids: Vec<String> = list(&d, true).unwrap().into_iter().map(|j| j.id).collect();
        assert!(ids.contains(&good.id), "the good job is in the list");
        assert!(!ids.contains(&bad.id), "the bad job is left out");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_retry_wait_is_the_same_in_rust_and_sql() {
        let c = Connection::open_in_memory().unwrap();
        for failures in 0..=MAX_FAILURES + 2 {
            let sql: i64 = c
                .query_row(
                    concat!(
                        "SELECT ",
                        retry_wait_sql!(),
                        " FROM (SELECT ?1 AS failures) task"
                    ),
                    rusqlite::params![failures, MAX_FAILURES],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(sql, retry_wait(failures), "failures = {failures}");
        }
    }
}
