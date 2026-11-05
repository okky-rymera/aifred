//! AIfred's own jobs: the things that AIfred does by itself, at a time or
//! after an event. They are apart from the user's to-do tasks (tasks.rs).
//!
//! A job has a trigger and an action:
//! - Triggers: "once" (one local time), "repeat" (a calendar rule, RFC 5545
//!   RRULE), or "event" (for example "a task is done").
//! - Actions: "remind" (a notification, no AI call), "agent" (an AI run with
//!   read-only tools), or "brief" (write the brief of today again).
//!
//! The desktop app runs the jobs (the scheduler loop in main.rs). It asks
//! `claim_due` for the jobs to run now. `claim_due` writes a run row and moves
//! the next run time forward in one transaction, before the job runs. So a
//! crash or a second program can never run the same time twice.
//!
//! Missed times (the computer was asleep, or AIfred was closed): a one-time
//! job runs late. A repeating job runs one time for the newest missed time,
//! if it is at most 12 hours old. The older times get a "skipped" row.
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
    /// "remind", "agent", or "brief".
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
}

/// How a run ended.
#[derive(Debug, Clone, PartialEq)]
pub enum Outcome {
    /// The run told the user something.
    Ok { output: String, cost_usd: Option<f64> },
    /// The run had nothing to tell.
    Quiet { cost_usd: Option<f64> },
    /// The run finished the job: the job has no more runs.
    Finished { output: String, cost_usd: Option<f64> },
    Failed { error: String },
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
        anyhow::anyhow!("\"{name}\" is not a time zone. Use an IANA name, for example \"Asia/Jakarta\".")
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
        Ok(z) => utc.with_timezone(&z).format("%a %-d %b %Y %H:%M").to_string(),
        Err(_) => utc.format("%a %-d %b %Y %H:%M UTC").to_string(),
    }
}

/// A Unix time as the local time "YYYY-MM-DDTHH:MM" in the zone `tz`.
pub fn local_text(ts: i64, tz: &str) -> anyhow::Result<String> {
    let utc = DateTime::<Utc>::from_timestamp(ts, 0).context("the time is out of range")?;
    Ok(utc.with_timezone(&zone(tz)?).format("%Y-%m-%dT%H:%M").to_string())
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
    Ok(utc.with_timezone(&z).format("%Y-%m-%dT%H:%M%:z").to_string())
}

/// The RRULE text without "RRULE:" and spaces, with capital letters.
fn clean_rule(rule: &str) -> anyhow::Result<String> {
    let r = rule.trim();
    let r = r.strip_prefix("RRULE:").unwrap_or(r).trim().to_ascii_uppercase();
    if r.is_empty() {
        bail!("the repeat rule is empty. Example: FREQ=WEEKLY;BYDAY=MO");
    }
    if r.contains('\n') || r.contains("DTSTART") {
        bail!("give only the RRULE part, for example FREQ=WEEKLY;BYDAY=MO. Give the first time as \"start\".");
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
            bail!("a job can repeat at most once an hour. Use FREQ=HOURLY or slower, with at most one BYMINUTE value and one BYSECOND value.");
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

/// The times of `trigger` in (from, to], at most 500.
fn times_between(trigger: &Trigger, tz: &str, from: i64, to: i64) -> anyhow::Result<Vec<i64>> {
    if to <= from {
        return Ok(vec![]);
    }
    let mut out = times_after(trigger, tz, from, 500)?;
    out.retain(|t| *t <= to);
    Ok(out)
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
                    .map(|d| if d == "-1" { "the last day".to_string() } else { format!("day {d}") })
                    .collect()
            }
            "COUNT" => end = format!(", {v} times"),
            "UNTIL" => end = format!(", until {}", v.get(..8).map(|d| format!("{}-{}-{}", &d[..4], &d[4..6], &d[6..])).unwrap_or_default()),
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
        "MONTHLY" if !month_days.is_empty() => format!("{} on {}", every("month"), join_words(&month_days)),
        "MONTHLY" if !days.is_empty() => format!("{} on {}", every("month"), join_words(&days)),
        "MONTHLY" => every("month"),
        "YEARLY" => every("year"),
        _ => return format!("Repeats from {start}: {rule}"),
    };
    format!("{text}{end}")
}

// ---------- Store ----------

const COLUMNS: &str =
    "id, title, action, message, trigger, tz, status, next_run, created_by, created, updated, failures";

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
        can_start: can_start(&trigger, &tz, &status_of(r)?, now() as i64),
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
fn can_start(trigger: &Trigger, tz: &str, status: &str, now: i64) -> bool {
    match (status, trigger) {
        ("active", _) => false,
        (_, Trigger::Event { .. }) => true,
        // A paused one-time job whose time passed runs at once (update).
        ("paused", Trigger::Once { .. }) => true,
        _ => times_after(trigger, tz, now - 1, 1).is_ok_and(|t| !t.is_empty()),
    }
}

fn get_in(c: &Connection, id: &str) -> anyhow::Result<Job> {
    c.query_row(&format!("SELECT {COLUMNS} FROM job WHERE id = ?1"), [id], from_row)
        .optional()?
        .with_context(|| format!("no job with id {id}"))
}

pub fn get(dir: &Path, id: &str) -> anyhow::Result<Job> {
    db::read(dir, |c| get_in(c, id))
}

/// The jobs, the next run first. `all` false: only active and paused jobs.
pub fn list(dir: &Path, all: bool) -> anyhow::Result<Vec<Job>> {
    db::read(dir, |c| {
        let filter = if all { "" } else { "WHERE status IN ('active', 'paused')" };
        let mut st = c.prepare(&format!(
            "SELECT {COLUMNS} FROM job {filter}
             ORDER BY status != 'active', next_run IS NULL, next_run, created"
        ))?;
        Ok(st.query_map([], from_row)?.collect::<Result<_, _>>()?)
    })
}

/// The runs of `job` (all jobs when None), the newest first, at most `limit`.
/// Skipped runs only when `skipped` is true.
pub fn runs(dir: &Path, job: Option<&str>, limit: u32, skipped: bool) -> anyhow::Result<Vec<Run>> {
    db::read(dir, |c| {
        let mut st = c.prepare(
            "SELECT id, job, planned, started, finished, state, late, note, output, cost_usd
             FROM job_run WHERE (?1 IS NULL OR job = ?1) AND (?2 OR state != 'skipped')
             ORDER BY COALESCE(started, planned) DESC, id DESC LIMIT ?3",
        )?;
        let rows = st.query_map(rusqlite::params![job, skipped, limit.clamp(1, 200)], |r| {
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
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

fn check_action(action: &str) -> anyhow::Result<()> {
    match action {
        "remind" | "agent" | "brief" => Ok(()),
        a => bail!("the action must be \"remind\" or \"agent\", not \"{a}\""),
    }
}

/// Checks the trigger, and gives the next run after `now`.
fn check_trigger(trigger: &Trigger, tz: &str, now: i64) -> anyhow::Result<Option<i64>> {
    zone(tz)?;
    match trigger {
        Trigger::Once { at } => {
            let t = parse_local(at, zone(tz)?)?.timestamp();
            // A minute of slack: "now" from the AI can be a little old.
            if t < now - 60 {
                bail!("the time {at} ({tz}) is in the past. Now is {}.", show(now, tz));
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
    Ok(tx.query_row("SELECT COALESCE(MAX(id), 0) FROM job_event", [], |r| r.get(0))?)
}

pub fn create(dir: &Path, j: NewJob) -> anyhow::Result<Job> {
    let title = j.title.trim().to_string();
    if title.is_empty() {
        bail!("the job needs a title");
    }
    check_action(&j.action)?;
    if j.action != "brief" && j.message.trim().is_empty() {
        bail!("the job needs a message: the reminder text, or the instructions for the AI");
    }
    let trigger = j.trigger.context("the job needs a time or an event")?;
    let tz = j.tz.filter(|z| !z.trim().is_empty()).unwrap_or_else(local_zone);
    let t = now() as i64;
    let next = check_trigger(&trigger, &tz, t)?;
    db::write(dir, |tx| {
        let open: usize = tx.query_row(
            "SELECT COUNT(*) FROM job WHERE status IN ('active', 'paused')",
            [],
            |r| r.get(0),
        )?;
        if open >= MAX_JOBS {
            bail!("AIfred has {open} jobs, the most it can keep. Delete or finish a job first.");
        }
        let id = new_id();
        tx.execute(
            "INSERT INTO job (id, title, action, message, trigger, tz, status, next_run,
                              created_by, created, updated, failures, cursor)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'active', ?7, ?8, ?9, ?9, 0, ?10)",
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
    db::write(dir, |tx| {
        let old = get_in(tx, id)?;
        let title = match ch.title {
            Some(s) if s.trim().is_empty() => bail!("the job needs a title"),
            Some(s) => s.trim().to_string(),
            None => old.title.clone(),
        };
        let message = ch.message.map(|m| m.trim().to_string()).unwrap_or(old.message.clone());
        let tz = ch.tz.filter(|z| !z.trim().is_empty()).unwrap_or(old.tz.clone());
        let new_time = ch.trigger.is_some() || tz != old.tz;
        let trigger = ch.trigger.unwrap_or(old.trigger.clone());
        let status = match ch.status.as_deref() {
            // A new time for a job that ended ("move that reminder to
            // Friday"): the job starts again. A paused job stays paused.
            None if new_time && matches!(old.status.as_str(), "done" | "off") => "active".to_string(),
            None => old.status.clone(),
            Some(s @ ("active" | "paused")) => s.to_string(),
            Some(s) => bail!("the status must be \"active\" or \"paused\", not \"{s}\""),
        };
        let resumed = status == "active" && old.status != "active";
        if resumed && !new_time && !can_start(&trigger, &tz, &old.status, t) {
            // A job with no more times comes back only with a new time.
            bail!("the job has no more times. Give it a new time.");
        }
        if resumed {
            let open: usize = tx.query_row(
                "SELECT COUNT(*) FROM job WHERE status IN ('active', 'paused') AND id != ?1",
                [id],
                |r| r.get(0),
            )?;
            if old.status != "paused" && open >= MAX_JOBS {
                bail!("AIfred has {open} jobs, the most it can keep. Delete or finish a job first.");
            }
        }
        // A new time, or a job that starts again: the next run is from now,
        // so the missed times do not all run.
        let next = if let (true, false, "paused", Trigger::Once { at }) = (resumed, new_time, old.status.as_str(), &trigger) {
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
            "UPDATE job SET title = ?2, message = ?3, trigger = ?4, tz = ?5, status = ?6,
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
                resumed,
                cursor
            ],
        )?;
        get_in(tx, id)
    })
}

/// Deletes the job and its runs. The brief job cannot be deleted, because
/// nothing can make it again. Pause it instead.
pub fn delete(dir: &Path, id: &str) -> anyhow::Result<()> {
    if id == BRIEF_JOB {
        bail!("AIfred's brief job cannot be deleted. Pause it to stop it.");
    }
    db::write(dir, |tx| {
        if tx.execute("DELETE FROM job WHERE id = ?1", [id])? == 0 {
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
            Err(std::fs::TryLockError::Error(e)) => Err(e).context("cannot lock the scheduler file"),
        }
    }
}

/// Marks the runs that were busy when AIfred stopped as failed.
/// Call it only while you hold the SchedulerLock: then no other window has
/// a run that is busy now. They do not
/// run again: a reminder can come twice, but an AI run must not.
pub fn recover(dir: &Path) -> anyhow::Result<usize> {
    let t = now() as i64;
    db::write(dir, |tx| {
        Ok(tx.execute(
            "UPDATE job_run SET state = 'failed', finished = ?1,
                    note = 'AIfred stopped during the run.'
             WHERE state = 'running'",
            [t],
        )?)
    })
}

fn add_run(tx: &Transaction, job: &str, planned: i64, state: &str, note: &str, started: Option<i64>, late: bool) -> anyhow::Result<Option<i64>> {
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
    db::write(dir, |tx| {
        if clean {
            tx.execute("DELETE FROM job_event WHERE ts < ?1", [now - KEEP_EVENTS])?;
            tx.execute(
                "DELETE FROM job_run WHERE COALESCE(finished, planned) < ?1 AND state != 'running'",
                [now - KEEP_RUNS],
            )?;
        }
        let jobs: Vec<(String, rusqlite::Result<(Job, i64)>)> = {
            let mut st = tx.prepare(&format!(
                "SELECT {COLUMNS}, cursor FROM job WHERE status = 'active'"
            ))?;
            st.query_map([], |r| Ok((r.get(0)?, from_row(r).and_then(|j| Ok((j, r.get(12)?))))))?
                .collect::<Result<_, _>>()?
        };
        let mut out = vec![];
        for (id, job) in jobs {
            // A savepoint for each job: a failure undoes only this job's writes.
            tx.execute_batch("SAVEPOINT one_job")?;
            let claim = job.map_err(anyhow::Error::from).and_then(|(job, cursor)| match &job.trigger {
                Trigger::Event { event, wait_secs } => claim_event(tx, &job, cursor, event, *wait_secs, now),
                _ => claim_time(tx, &job, now),
            });
            match claim {
                Ok(c) => out.extend(c),
                Err(e) => {
                    tx.execute_batch("ROLLBACK TO one_job")?;
                    eprintln!("aifred: job {id} turns off: {e:#}");
                    let note = format!("AIfred cannot run this job: {e:#}");
                    add_run(tx, &id, now, "failed", &note, Some(now), false)?;
                    tx.execute(
                        "UPDATE job SET status = 'off', next_run = NULL WHERE id = ?1",
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
        "SELECT EXISTS (
           SELECT 1 FROM job WHERE status = 'active' AND (
             (next_run IS NOT NULL AND next_run <= ?1)
             -- A bad trigger is due too: claim_due turns the job off.
             OR CASE WHEN json_valid(trigger) THEN
                  json_extract(trigger, '$.kind') = 'event' AND next_run IS NULL AND EXISTS (
                    SELECT 1 FROM job_event e
                    WHERE e.name = json_extract(job.trigger, '$.event') AND e.id > job.cursor)
                ELSE 1 END))",
        [now],
        |r| r.get(0),
    )?)
}

fn claim_time(tx: &Transaction, job: &Job, now: i64) -> anyhow::Result<Option<Claim>> {
    let Some(mut planned) = job.next_run else {
        return Ok(None);
    };
    if planned > now {
        return Ok(None);
    }
    // The times after `planned` that also passed: only the newest one runs.
    let later = times_between(&job.trigger, &job.tz, planned, now)?;
    let next = times_after(&job.trigger, &job.tz, now, 1)?.first().copied();
    let skip = |t: i64, note: &str| add_run(tx, &job.id, t, "skipped", note, None, false);
    if let Some(&newest) = later.last() {
        // The missed times, without the newest: at most MAX_SKIP_ROWS rows.
        // The oldest row tells how many times have no row.
        let missed: Vec<i64> = std::iter::once(planned).chain(later[..later.len() - 1].iter().copied()).collect();
        let cut = missed.len().saturating_sub(MAX_SKIP_ROWS);
        for (i, t) in missed[cut..].iter().enumerate() {
            if i == 0 && cut > 0 {
                skip(*t, &format!("AIfred was not running at this time, and at {cut} earlier times."))?;
            } else {
                skip(*t, "AIfred was not running at this time.")?;
            }
        }
        planned = newest;
    }
    let repeat = matches!(job.trigger, Trigger::Repeat { .. });
    let set_next = |next: Option<i64>| -> anyhow::Result<()> {
        let status = if next.is_none() { "done" } else { "active" };
        tx.execute(
            "UPDATE job SET next_run = ?2, status = ?3 WHERE id = ?1",
            rusqlite::params![job.id, next, status],
        )?;
        Ok(())
    };
    if repeat && now - planned > STALE_SECS {
        add_run(tx, &job.id, planned, "skipped", "Too late: AIfred was not running at this time.", None, false)?;
        set_next(next)?;
        return Ok(None);
    }
    if busy(tx, &job.id)? {
        add_run(tx, &job.id, planned, "skipped", "The last run was still busy.", None, false)?;
        set_next(next)?;
        return Ok(None);
    }
    let late = now - planned > LATE_SECS;
    let run = add_run(tx, &job.id, planned, "running", "", Some(now), late)?;
    set_next(next)?;
    let job = get_in(tx, &job.id)?;
    Ok(run.map(|run| Claim { job, run, planned, late }))
}

fn claim_event(tx: &Transaction, job: &Job, cursor: i64, event: &str, wait: u32, now: i64) -> anyhow::Result<Option<Claim>> {
    let (first, last, top): (Option<i64>, Option<i64>, Option<i64>) = tx.query_row(
        "SELECT MIN(ts), MAX(ts), MAX(id) FROM job_event WHERE name = ?1 AND id > ?2",
        rusqlite::params![event, cursor],
        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
    )?;
    let (Some(first), Some(last), Some(top)) = (first, last, top) else {
        if job.next_run.is_some() {
            tx.execute("UPDATE job SET next_run = NULL WHERE id = ?1", [&job.id])?;
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
                "UPDATE job SET next_run = ?2 WHERE id = ?1",
                rusqlite::params![job.id, next],
            )?;
        }
        return Ok(None);
    }
    tx.execute(
        "UPDATE job SET next_run = NULL, cursor = ?2 WHERE id = ?1",
        rusqlite::params![job.id, top],
    )?;
    let run = add_run(tx, &job.id, due, "running", "", Some(now), false)?;
    let job = get_in(tx, &job.id)?;
    Ok(run.map(|run| Claim { job, run, planned: due, late: false }))
}

/// Saves how a run ended, and what it means for the job:
/// - a failure retries after 1, 2, 4, then 8 minutes. After 5 failures in a
///   row the job turns off. Event jobs do not retry: the next event runs them.
/// - "Finished" ends the job.
///
/// It gives the job after the change, or None when the job is gone.
pub fn finish(dir: &Path, run: i64, outcome: Outcome) -> anyhow::Result<Option<Job>> {
    let t = now() as i64;
    db::write(dir, |tx| {
        let job: Option<String> = tx
            .query_row("SELECT job FROM job_run WHERE id = ?1", [run], |r| r.get(0))
            .optional()?;
        let Some(job) = job else {
            return Ok(None);
        };
        let (state, output, note, cost) = match &outcome {
            Outcome::Ok { output, cost_usd } | Outcome::Finished { output, cost_usd } => ("ok", output.as_str(), "", *cost_usd),
            Outcome::Quiet { cost_usd } => ("quiet", "", "", *cost_usd),
            Outcome::Failed { error } => ("failed", "", error.as_str(), None),
        };
        tx.execute(
            "UPDATE job_run SET state = ?2, finished = ?3, output = ?4, note = ?5, cost_usd = ?6 WHERE id = ?1",
            rusqlite::params![run, state, t, output, note, cost],
        )?;
        let Some(j) = tx
            .query_row(&format!("SELECT {COLUMNS} FROM job WHERE id = ?1"), [&job], from_row)
            .optional()?
        else {
            return Ok(None);
        };
        match outcome {
            Outcome::Failed { .. } => {
                let failures = j.failures + 1;
                if failures >= MAX_FAILURES {
                    tx.execute(
                        "UPDATE job SET failures = ?2, status = 'off', next_run = NULL WHERE id = ?1",
                        rusqlite::params![j.id, failures],
                    )?;
                } else if matches!(j.trigger, Trigger::Event { .. }) || j.status == "paused" {
                    tx.execute("UPDATE job SET failures = ?2 WHERE id = ?1", rusqlite::params![j.id, failures])?;
                } else {
                    let retry = t + 60 * (1i64 << (failures - 1));
                    let next = j.next_run.map_or(retry, |n| n.min(retry));
                    tx.execute(
                        "UPDATE job SET failures = ?2, next_run = ?3, status = 'active' WHERE id = ?1",
                        rusqlite::params![j.id, failures, next],
                    )?;
                }
            }
            Outcome::Finished { .. } => {
                tx.execute(
                    "UPDATE job SET failures = 0, status = 'done', next_run = NULL WHERE id = ?1",
                    [&j.id],
                )?;
            }
            Outcome::Ok { .. } | Outcome::Quiet { .. } => {
                tx.execute("UPDATE job SET failures = 0 WHERE id = ?1", [&j.id])?;
            }
        }
        Ok(Some(get_in(tx, &j.id)?))
    })
}

/// Short answers that mean NOTHING, in lower case, with no punctuation.
const QUIET_ANSWERS: &[&str] = &["nothing", "nothing to report", "nothing to tell", "nothing new"];

/// The outcome of an agent answer: NOTHING is quiet, "DONE:" ends the job.
/// Capital letters and markdown marks ("**DONE:**", "Nothing.") do not matter.
pub fn agent_outcome(answer: &str, cost_usd: Option<f64>) -> Outcome {
    let a = answer.trim();
    let bare = a.trim_matches(|c: char| !c.is_ascii_alphanumeric()).to_ascii_lowercase();
    if bare.is_empty() || bare == NOTHING.to_ascii_lowercase() || QUIET_ANSWERS.contains(&bare.as_str()) {
        return Outcome::Quiet { cost_usd };
    }
    // "DONE:", "**DONE:**", "**Done**:", "Done: ...": the marks around the word go.
    let mark = |c: char| matches!(c, '*' | '_' | '#' | '`') || c.is_whitespace();
    let word = DONE.trim_end_matches(':');
    let start = a.trim_start_matches(mark);
    if let Some(head) = start.get(..word.len()).filter(|h| h.eq_ignore_ascii_case(word))
        && let Some(rest) = start[head.len()..].trim_start_matches(mark).strip_prefix(':')
    {
        return Outcome::Finished { output: rest.trim_start_matches(mark).trim().to_string(), cost_usd };
    }
    Outcome::Ok { output: a.to_string(), cost_usd }
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
        Trigger::Repeat { start: start.into(), rule: rule.into() }
    }

    #[test]
    fn rules_give_the_right_local_times() {
        let jkt = "Asia/Jakarta";
        // Every Monday at 09:00, asked on Thursday 5 Nov 2026.
        let mon = repeat("2026-11-05T09:00", "FREQ=WEEKLY;BYDAY=MO");
        let after = ts("2026-11-05T12:00", jkt);
        let got: Vec<String> = times_after(&mon, jkt, after, 3).unwrap().iter().map(|t| show(*t, jkt)).collect();
        assert_eq!(got, ["Mon 9 Nov 2026 09:00", "Mon 16 Nov 2026 09:00", "Mon 23 Nov 2026 09:00"]);
        // The 3rd and the 17th.
        let days = repeat("2026-11-05T09:00", "RRULE:FREQ=MONTHLY;BYMONTHDAY=3,17");
        let got: Vec<String> = times_after(&days, jkt, after, 3).unwrap().iter().map(|t| show(*t, jkt)).collect();
        assert_eq!(got, ["Tue 17 Nov 2026 09:00", "Thu 3 Dec 2026 09:00", "Thu 17 Dec 2026 09:00"]);
        // The last day of the month, also in February.
        let last = repeat("2027-01-01T17:00", "FREQ=MONTHLY;BYMONTHDAY=-1");
        let got: Vec<String> = times_after(&last, jkt, ts("2027-01-01T00:00", jkt), 2).unwrap().iter().map(|t| show(*t, jkt)).collect();
        assert_eq!(got, ["Sun 31 Jan 2027 17:00", "Sun 28 Feb 2027 17:00"]);
        // Every other Thursday from 12 Nov.
        let other = repeat("2026-11-12T10:00", "FREQ=WEEKLY;INTERVAL=2;BYDAY=TH");
        let got: Vec<String> = times_after(&other, jkt, after, 2).unwrap().iter().map(|t| show(*t, jkt)).collect();
        assert_eq!(got, ["Thu 12 Nov 2026 10:00", "Thu 26 Nov 2026 10:00"]);
        // The last Friday of the month.
        let fri = repeat("2026-11-05T16:00", "FREQ=MONTHLY;BYDAY=-1FR");
        assert_eq!(show(times_after(&fri, jkt, after, 1).unwrap()[0], jkt), "Fri 27 Nov 2026 16:00");
        // A rule with an end.
        let five = repeat("2026-11-06T09:00", "FREQ=DAILY;COUNT=2");
        assert_eq!(times_after(&five, jkt, after, 10).unwrap().len(), 2);
    }

    #[test]
    fn summer_time_keeps_the_wall_time_and_rejects_missing_times() {
        let ny = "America/New_York";
        // Clocks go back on 1 Nov 2026: the daily 09:00 stays 09:00.
        let daily = repeat("2026-10-30T09:00", "FREQ=DAILY");
        let got: Vec<String> = times_after(&daily, ny, ts("2026-10-30T10:00", ny), 3).unwrap().iter().map(|t| show(*t, ny)).collect();
        assert_eq!(got, ["Sat 31 Oct 2026 09:00", "Sun 1 Nov 2026 09:00", "Mon 2 Nov 2026 09:00"]);
        // 02:30 on 8 Mar 2026 does not exist in New York.
        let e = parse_local("2026-03-08T02:30", zone(ny).unwrap()).unwrap_err();
        assert!(e.to_string().contains("summer time"), "{e}");
    }

    #[test]
    fn plain_words_for_the_common_rules() {
        let jkt = "Asia/Jakarta";
        assert_eq!(describe(&repeat("2026-11-09T09:00", "FREQ=WEEKLY;BYDAY=MO"), jkt), "Every Monday at 09:00 (Asia/Jakarta)");
        assert_eq!(describe(&repeat("2026-11-09T09:00", "FREQ=MONTHLY;BYMONTHDAY=3,17"), jkt), "Every month on day 3 and day 17 at 09:00 (Asia/Jakarta)");
        assert_eq!(describe(&repeat("2026-11-12T10:00", "FREQ=WEEKLY;INTERVAL=2;BYDAY=TH"), jkt), "Every 2 weeks on Thursday at 10:00 (Asia/Jakarta)");
        assert_eq!(describe(&repeat("2026-11-12T16:00", "FREQ=MONTHLY;BYDAY=-1FR"), jkt), "Every month on the last Friday at 16:00 (Asia/Jakarta)");
        assert_eq!(describe(&Trigger::Once { at: "2026-11-13T15:00".into() }, jkt), "Once, Fri 13 Nov 2026 at 15:00 (Asia/Jakarta)");
        assert!(describe(&Trigger::Event { event: TASK_DONE.into(), wait_secs: 90 }, "").starts_with("Each time you finish a task"));
    }

    #[test]
    fn create_checks_the_input_and_the_limit() {
        let d = dir("create");
        let jkt = "Asia/Jakarta";
        let past = job("Old", Trigger::Once { at: "2020-01-01T09:00".into() }, jkt);
        assert!(create(&d, past).unwrap_err().to_string().contains("in the past"));
        let bad = job("Bad", repeat("2026-11-09T09:00", "FREQ=MINUTELY"), jkt);
        assert!(create(&d, bad).unwrap_err().to_string().contains("once an hour"));
        let zone_err = job("Zone", repeat("2030-11-09T09:00", "FREQ=DAILY"), "Mars/Base");
        assert!(create(&d, zone_err).unwrap_err().to_string().contains("IANA"));
        let ok = create(&d, job("Timesheet", repeat("2030-11-04T09:00", "FREQ=WEEKLY;BYDAY=MO"), jkt)).unwrap();
        assert_eq!(ok.next_run, Some(ts("2030-11-04T09:00", jkt)));
        assert_eq!(ok.status, "active");
        // The brief job is there from the start, so 19 more fit.
        for i in 0..(MAX_JOBS - 2) {
            create(&d, job(&format!("J{i}"), repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap();
        }
        let e = create(&d, job("One too many", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap_err();
        assert!(e.to_string().contains("the most it can keep"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_job_runs_once_for_its_time() {
        let d = dir("once");
        let jkt = "Asia/Jakarta";
        let j = create(&d, job("Call", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap();
        let t = ts("2030-11-04T09:00", jkt);
        assert!(claim_due(&d, t - 1).unwrap().is_empty(), "not yet");
        let c = claim_due(&d, t + 5).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!((c[0].planned, c[0].late), (t, false));
        assert_eq!(c[0].job.next_run, Some(t + 86400), "the next run moved at the claim");
        assert!(claim_due(&d, t + 10).unwrap().is_empty(), "a second claim gets nothing");
        let after = finish(&d, c[0].run, Outcome::Ok { output: "Done".into(), cost_usd: None }).unwrap().unwrap();
        assert_eq!(after.status, "active");
        assert_eq!(runs(&d, Some(&j.id), 10, true).unwrap()[0].state, "ok");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn missed_times_give_one_late_run() {
        let d = dir("missed");
        let jkt = "Asia/Jakarta";
        let j = create(&d, job("Daily", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap();
        // The computer sleeps from Mon 08:00 to Wed 10:00: 3 times pass.
        let wake = ts("2030-11-06T10:00", jkt);
        let c = claim_due(&d, wake).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].planned, ts("2030-11-06T09:00", jkt), "the newest time runs");
        assert!(c[0].late);
        let all = runs(&d, Some(&j.id), 10, true).unwrap();
        assert_eq!(all.iter().filter(|r| r.state == "skipped").count(), 2);
        assert!(runs(&d, Some(&j.id), 10, false).unwrap().iter().all(|r| r.state != "skipped"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_repeat_that_is_too_old_waits_for_the_next_time_and_a_once_runs_late() {
        let d = dir("stale");
        let jkt = "Asia/Jakarta";
        let daily = create(&d, job("Daily", repeat("2030-11-04T09:00", "FREQ=WEEKLY;BYDAY=MO"), jkt)).unwrap();
        let once = create(&d, job("Once", Trigger::Once { at: "2030-11-04T09:00".into() }, jkt)).unwrap();
        // Mon 09:00 + 13 hours.
        let c = claim_due(&d, ts("2030-11-04T22:00", jkt)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].job.id, once.id, "the one-time reminder still comes");
        assert_eq!(c[0].job.status, "done", "it has no more times");
        let daily = get(&d, &daily.id).unwrap();
        assert_eq!(daily.next_run, Some(ts("2030-11-11T09:00", jkt)));
        assert_eq!(runs(&d, Some(&daily.id), 5, true).unwrap()[0].note, "Too late: AIfred was not running at this time.");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn failures_retry_then_turn_the_job_off() {
        let d = dir("fail");
        let jkt = "Asia/Jakarta";
        let j = create(&d, job("Weekly", repeat("2030-11-04T09:00", "FREQ=WEEKLY"), jkt)).unwrap();
        let mut t = ts("2030-11-04T09:00", jkt);
        for i in 1..=MAX_FAILURES {
            let c = claim_due(&d, t).unwrap();
            assert_eq!(c.len(), 1, "run {i}");
            let after = finish(&d, c[0].run, Outcome::Failed { error: "no network".into() }).unwrap().unwrap();
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
        let on = update(&d, &j.id, JobChange { status: Some("active".into()), ..Default::default() }).unwrap();
        assert_eq!((on.status.as_str(), on.failures), ("active", 0));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_busy_run_and_a_restart_never_run_twice() {
        let d = dir("busy");
        let jkt = "Asia/Jakarta";
        create(&d, job("Hourly", repeat("2030-11-04T09:00", "FREQ=HOURLY"), jkt)).unwrap();
        let t = ts("2030-11-04T09:00", jkt);
        let first = claim_due(&d, t).unwrap();
        assert_eq!(first.len(), 1);
        // One hour later the first run is still busy: the time is skipped.
        assert!(claim_due(&d, t + 3600).unwrap().is_empty());
        // AIfred stops and starts again: the busy run is failed, not run again.
        assert_eq!(recover(&d).unwrap(), 1);
        let r = runs(&d, None, 10, true).unwrap();
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
        assert!(claim_due(&d, t).unwrap().is_empty(), "it waits for more events");
        assert!(get(&d, BRIEF_JOB).unwrap().next_run.unwrap() >= t + 89);
        let c = claim_due(&d, t + 91).unwrap();
        assert_eq!(c.len(), 1, "8 tasks give one run");
        assert!(claim_due(&d, t + 200).unwrap().is_empty(), "the events are used");
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
        let j = create(&d, job("Mon", repeat("2030-11-04T09:00", "FREQ=WEEKLY;BYDAY=MO"), jkt)).unwrap();
        let p = update(&d, &j.id, JobChange { status: Some("paused".into()), ..Default::default() }).unwrap();
        assert_eq!(p.status, "paused");
        assert!(claim_due(&d, ts("2030-11-04T09:00", jkt)).unwrap().is_empty(), "a paused job does not run");
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
        assert_eq!(agent_outcome("NOTHING", None), Outcome::Quiet { cost_usd: None });
        assert_eq!(agent_outcome(" **NOTHING.** ", None), Outcome::Quiet { cost_usd: None });
        assert_eq!(agent_outcome("Nothing.", None), Outcome::Quiet { cost_usd: None });
        assert_eq!(agent_outcome("Nothing to report.", None), Outcome::Quiet { cost_usd: None });
        let merged = Outcome::Finished { output: "PR #123 is merged.".into(), cost_usd: Some(0.1) };
        assert_eq!(agent_outcome("DONE: PR #123 is merged.", Some(0.1)), merged);
        assert_eq!(agent_outcome("**DONE:** PR #123 is merged.", Some(0.1)), merged);
        assert_eq!(agent_outcome("**Done**: PR #123 is merged.", Some(0.1)), merged);
        assert!(matches!(agent_outcome("PR #123 is still open.", None), Outcome::Ok { .. }));
        assert!(matches!(agent_outcome("Nothing changed, but the PR has a new review.", None), Outcome::Ok { .. }));
        assert!(matches!(agent_outcome("Done reviewing: 2 comments.", None), Outcome::Ok { .. }));
    }

    #[test]
    fn a_rule_can_repeat_at_most_once_an_hour() {
        for bad in [
            "FREQ=MINUTELY",
            "FREQ=HOURLY;BYMINUTE=0,5,10",
            "FREQ=DAILY;BYHOUR=9;BYMINUTE=0,1",
            "FREQ=HOURLY;BYSECOND=0,30",
        ] {
            assert!(clean_rule(bad).unwrap_err().to_string().contains("once an hour"), "{bad}");
        }
        for good in ["FREQ=HOURLY", "FREQ=DAILY;BYHOUR=9,17;BYMINUTE=30", "FREQ=HOURLY;BYMINUTE=15;BYSECOND=0"] {
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
        assert_eq!(parse_local(&text, zone(ny).unwrap()).unwrap().timestamp(), second);
        // A plain time has no offset.
        assert_eq!(local_text_exact(ts("2026-11-02T09:00", ny), ny).unwrap(), "2026-11-02T09:00");
    }

    #[test]
    fn a_paused_one_time_job_runs_late_after_its_time() {
        let d = dir("pausedonce");
        let jkt = "Asia/Jakarta";
        let t = now() as i64;
        let at = local_text(t + 3600, jkt).unwrap();
        let j = create(&d, job("Call Sam", Trigger::Once { at }, jkt)).unwrap();
        update(&d, &j.id, JobChange { status: Some("paused".into()), ..Default::default() }).unwrap();
        // The time passes during the pause.
        let past = local_text(t - 3600, jkt).unwrap();
        let planned = ts(&past, jkt);
        db::write(&d, |tx| {
            let trig = serde_json::to_string(&Trigger::Once { at: past.clone() })?;
            tx.execute("UPDATE job SET trigger = ?2 WHERE id = ?1", rusqlite::params![j.id, trig])?;
            Ok(())
        })
        .unwrap();
        assert!(get(&d, &j.id).unwrap().can_start, "Start shows");
        let back = update(&d, &j.id, JobChange { status: Some("active".into()), ..Default::default() }).unwrap();
        assert_eq!((back.status.as_str(), back.next_run), ("active", Some(planned)));
        let c = claim_due(&d, t).unwrap();
        assert_eq!((c.len(), c[0].late), (1, true), "it runs at once, as a late run");
        // Done now: no more times, so Start does not show and does not work.
        let done = get(&d, &j.id).unwrap();
        assert_eq!((done.status.as_str(), done.can_start), ("done", false));
        let e = update(&d, &j.id, JobChange { status: Some("active".into()), ..Default::default() }).unwrap_err();
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
            tx.execute("INSERT INTO job_event (name, ts) VALUES (?1, ?2)", rusqlite::params![TASK_DONE, t + 100])?;
            Ok(())
        })
        .unwrap();
        assert!(claim_due(&d, t + 200).unwrap().is_empty(), "the run is still busy");
        let next = get(&d, BRIEF_JOB).unwrap().next_run.unwrap();
        assert_eq!(next, t + 200 + BUSY_RECHECK_SECS);
        // Not due on the next tick: that tick takes no write lock.
        assert!(!db::read(&d, |c| any_due(c, t + 230)).unwrap());
        finish(&d, c[0].run, Outcome::Quiet { cost_usd: None }).unwrap();
        assert_eq!(claim_due(&d, t + 260).unwrap().len(), 1, "the new event runs after the busy run");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_one_window_holds_the_scheduler_lock() {
        let d = dir("schedlock");
        let first = SchedulerLock::take(&d).unwrap();
        assert!(first.is_some());
        assert!(SchedulerLock::take(&d).unwrap().is_none(), "a second window waits");
        drop(first);
        assert!(SchedulerLock::take(&d).unwrap().is_some(), "it takes over when the first one stops");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_time_starts_an_ended_job_again() {
        let d = dir("revive");
        let jkt = "Asia/Jakarta";
        let j = create(&d, job("Call", Trigger::Once { at: "2030-11-04T09:00".into() }, jkt)).unwrap();
        let c = claim_due(&d, ts("2030-11-04T09:00", jkt)).unwrap();
        assert_eq!(c[0].job.status, "done");
        let moved = update(
            &d,
            &j.id,
            JobChange { trigger: Some(Trigger::Once { at: "2030-11-08T10:00".into() }), ..Default::default() },
        )
        .unwrap();
        assert_eq!((moved.status.as_str(), moved.next_run), ("active", Some(ts("2030-11-08T10:00", jkt))));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_change_to_an_event_ignores_the_old_events() {
        let d = dir("toevent");
        let jkt = "Asia/Jakarta";
        let j = create(&d, job("Daily", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap();
        let store = crate::tasks::TaskStore::new(&d);
        let t = store.add("Old task", "", "you").unwrap();
        store.set_done(&t.id, true).unwrap();
        // Pause the brief job, so only this job can run.
        update(&d, BRIEF_JOB, JobChange { status: Some("paused".into()), ..Default::default() }).unwrap();
        update(
            &d,
            &j.id,
            JobChange { trigger: Some(Trigger::Event { event: TASK_DONE.into(), wait_secs: 10 }), ..Default::default() },
        )
        .unwrap();
        assert!(claim_due(&d, now() as i64 + 3600).unwrap().is_empty(), "the old task does not count");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_bad_job_turns_off_and_the_others_still_run() {
        let d = dir("bad");
        let jkt = "Asia/Jakarta";
        let good = create(&d, job("Good", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap();
        let bad = create(&d, job("Bad", repeat("2030-11-04T09:00", "FREQ=DAILY"), jkt)).unwrap();
        db::write(&d, |tx| {
            tx.execute("UPDATE job SET tz = 'Mars/Base' WHERE id = ?1", [&bad.id])?;
            Ok(())
        })
        .unwrap();
        let c = claim_due(&d, ts("2030-11-04T09:00", jkt)).unwrap();
        assert_eq!(c.len(), 1);
        assert_eq!(c[0].job.id, good.id);
        let bad = get(&d, &bad.id).unwrap();
        assert_eq!((bad.status.as_str(), bad.next_run), ("off", None));
        let r = &runs(&d, Some(&bad.id), 5, true).unwrap()[0];
        assert_eq!(r.state, "failed");
        assert!(r.note.contains("time zone"), "{}", r.note);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_long_gap_writes_at_most_twenty_skipped_rows() {
        let d = dir("gap");
        let jkt = "Asia/Jakarta";
        let j = create(&d, job("Hourly", repeat("2030-11-04T09:00", "FREQ=HOURLY"), jkt)).unwrap();
        // Closed for 3 days: 72 hourly times pass.
        let c = claim_due(&d, ts("2030-11-07T09:30", jkt)).unwrap();
        assert_eq!(c.len(), 1);
        let skipped: Vec<Run> = runs(&d, Some(&j.id), 200, true).unwrap().into_iter().filter(|r| r.state == "skipped").collect();
        assert_eq!(skipped.len(), MAX_SKIP_ROWS);
        assert!(skipped.last().unwrap().note.contains("52 earlier times"), "{}", skipped.last().unwrap().note);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_brief_job_cannot_be_deleted() {
        let d = dir("keepbrief");
        assert!(delete(&d, BRIEF_JOB).unwrap_err().to_string().contains("Pause"));
        assert!(get(&d, BRIEF_JOB).is_ok());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
