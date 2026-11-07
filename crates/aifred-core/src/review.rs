//! The reviews. They keep the list trustworthy (GTD: Getting Things Done).
//! Both need no AI call.
//!
//! The end-of-day check (day_check, finish_day): what the user finished
//! today, and what is still open of today's plan and of the local tasks due
//! today. The user picks what moves to tomorrow: those items go into the
//! plan of the next day. A local task due today also gets tomorrow as its
//! due day.
//!
//! The weekly review (week, pick, finish_week): every WEEK_DAYS days. It
//! asks if the goals are still right (goals.rs), with the number of
//! finished items that served each goal in the last 7 days. Then it lists
//! the open items that did not move in STALE_DAYS days. For each item the
//! user picks one:
//! - "do": the item goes into today's plan.
//! - "plan": the item goes into the plan of a later day. A local task
//!   also waits until that day (its start day).
//! - "delegate": someone else does it. It goes on the "waiting for" list
//!   (waiting.rs), and a local task closes.
//! - "drop": a local task closes. A GitHub or ClickUp item does not change
//!   there: AIfred only stops counting it in the brief and the reviews.
//!
//! A local task that closes at a pick is not finished work: no "task done"
//! jobs start, and AIfred keeps no real time for it (worktime.rs). An item
//! that the user gave away or dropped counts again when it changes after
//! the pick (hidden_in).
//!
//! An item that a review puts into a day with no plan makes a plan with
//! the mode plan::REVIEW, so the app still asks how to plan that day.
//!
//! Each pick has an undo. The brief and the plan chat get the picks that
//! are in force (for_ai).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, bail};
use chrono::{Datelike, NaiveDate};
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::{Value, json};

use crate::db;
use crate::plan::PlanItem;

/// An item with no change for this many days is in the weekly review.
pub const STALE_DAYS: u64 = 21;
/// The weekly review comes this many days after the last one.
pub const WEEK_DAYS: i64 = 7;
/// The weekly review shows at most this many items, oldest first.
pub const STALE_MAX: usize = 30;
/// The overview has the picks of this many days.
pub const RECENT_DAYS: u64 = 30;

pub const DO: &str = "do";
pub const PLAN: &str = "plan";
pub const DELEGATE: &str = "delegate";
pub const DROP: &str = "drop";
const PICKS: [&str; 4] = [DO, PLAN, DELEGATE, DROP];

/// An item in the end-of-day check.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayItem {
    pub key: String,
    pub title: String,
    /// Where the item is from (items::title_place), or "".
    pub place: String,
    /// The real time of a finished item, in minutes (worktime.rs).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub minutes: Option<u32>,
}

/// The end-of-day check of one day.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayCheck {
    pub day: String,
    /// What the user finished on the day.
    pub done: Vec<DayItem>,
    /// What is still open: the plan of the day first, then the local tasks due that day.
    pub left: Vec<DayItem>,
    /// True when the user finished the check of this day.
    pub finished: bool,
    /// The keys that went into the next day's plan.
    pub moved: Vec<String>,
}

/// An item that did not move in STALE_DAYS days.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StaleItem {
    pub key: String,
    pub title: String,
    pub place: String,
    /// The last change, in Unix seconds.
    pub last: u64,
    /// Whole days since the last change.
    pub days: u64,
}

/// A goal, and the finished items that served it in the last 7 days.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GoalWeek {
    pub id: i64,
    pub text: String,
    pub done: u32,
}

/// The weekly review.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WeekReview {
    /// True when the review is due today.
    pub due: bool,
    /// The day of the last review, "YYYY-MM-DD", or None.
    pub last: Option<String>,
    /// The items that did not move, oldest first. At most STALE_MAX.
    pub items: Vec<StaleItem>,
    /// The number of items that did not move and are not in `items`.
    pub more: usize,
    pub goals: Vec<GoalWeek>,
    /// True when the app must ask the user to check the goals (goals.rs).
    pub goals_stale: bool,
}

/// One pick in the weekly review.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Pick {
    pub id: i64,
    pub key: String,
    /// DO, PLAN, DELEGATE, or DROP.
    pub pick: String,
    pub title: String,
    pub place: String,
    /// PLAN: the day. DELEGATE: the person. Else "".
    pub note: String,
    pub ts: u64,
}

/// The start of a local day and of the next day, in Unix seconds.
fn day_bounds(day: &str) -> anyhow::Result<(i64, i64)> {
    let d = parse_day(day)?;
    let start = crate::calendar::local_midnight(d)?.timestamp();
    let next = d.succ_opt().context("no next day")?;
    let end = crate::calendar::local_midnight(next)?.timestamp();
    Ok((start, end))
}

fn parse_day(day: &str) -> anyhow::Result<NaiveDate> {
    crate::plan::check_day(day)?;
    NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .with_context(|| format!("the day \"{day}\" does not exist"))
}

/// The day after `day`, "YYYY-MM-DD".
pub fn next_day(day: &str) -> anyhow::Result<String> {
    Ok(parse_day(day)?
        .succ_opt()
        .context("no next day")?
        .format("%F")
        .to_string())
}

fn title_place(c: &Connection, key: &str) -> anyhow::Result<(String, String)> {
    Ok(crate::items::title_place(c, key)?.unwrap_or_else(|| (key.into(), String::new())))
}

fn day_item(c: &Connection, key: &str, minutes: Option<u32>) -> anyhow::Result<DayItem> {
    let (title, place) = title_place(c, key)?;
    Ok(DayItem {
        key: key.into(),
        title,
        place,
        minutes,
    })
}

/// The last change of an item, in Unix seconds, or None when AIfred does
/// not know it. A local task changes in AIfred. A GitHub or ClickUp item
/// changes there.
fn last_change(c: &Connection, key: &str) -> anyhow::Result<Option<u64>> {
    let (source, id) = crate::items::split(key)?;
    Ok(match source {
        "task" => c
            .query_row("SELECT updated FROM user_task WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .optional()?,
        "github" | "clickup" => c
            .query_row(
                "SELECT data FROM work_item WHERE source = ?1 AND item_id = ?2",
                [source, id],
                |r| r.get::<_, String>(0),
            )
            .optional()?
            .and_then(|data| source_updated(source, &data)),
        _ => None,
    })
}

/// The keys that the user gave away or dropped in a weekly review. They do
/// not count in the reviews or the brief. The newest pick of a key counts.
/// A key counts again when the item changed after the pick: for example,
/// the user opened the task again, or the GitHub item has new activity.
fn hidden_in(c: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut q = c.prepare(
        "SELECT p.key, p.pick, p.ts FROM review_pick p
         WHERE p.id = (SELECT MAX(id) FROM review_pick WHERE key = p.key)",
    )?;
    let rows: Vec<(String, String, u64)> = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
        .collect::<Result<_, _>>()?;
    let mut out = HashSet::new();
    for (key, pick, ts) in rows {
        if (pick == DELEGATE || pick == DROP) && last_change(c, &key)?.is_none_or(|t| t <= ts) {
            out.insert(key);
        }
    }
    Ok(out)
}

fn day_check_in(c: &Connection, day: &str) -> anyhow::Result<DayCheck> {
    let (start, end) = day_bounds(day)?;
    // A pick closes a local task that the user gives away or drops, but
    // the user did not finish it.
    let hidden = hidden_in(c)?;
    let real: HashMap<String, u32> = {
        let mut q = c.prepare("SELECT key, minutes FROM work_done")?;
        let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
        rows.collect::<Result<_, _>>()?
    };

    let mut done = Vec::new();
    let mut seen = HashSet::new();
    let mut q = c.prepare(
        "SELECT id FROM user_task WHERE done = 1 AND done_at >= ?1 AND done_at < ?2 ORDER BY done_at",
    )?;
    let ids: Vec<String> = q
        .query_map([start, end], |r| r.get(0))?
        .collect::<Result<_, _>>()?;
    for id in ids {
        let key = format!("task:{id}");
        if hidden.contains(&key) || !seen.insert(key.clone()) {
            continue;
        }
        done.push(day_item(c, &key, real.get(&key).copied())?);
    }
    // GitHub, ClickUp, and email items that the user finished with "Done".
    let mut q = c.prepare(
        "SELECT key, minutes FROM work_done WHERE finished >= ?1 AND finished < ?2 AND key NOT LIKE 'task:%' ORDER BY finished",
    )?;
    let rows: Vec<(String, u32)> = q
        .query_map([start, end], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (key, minutes) in rows {
        if seen.insert(key.clone()) {
            done.push(day_item(c, &key, Some(minutes))?);
        }
    }

    let mut left = Vec::new();
    let finished_keys: HashSet<String> = done.iter().map(|d| d.key.clone()).collect();
    let planned = crate::plan::items_in(c, day)?
        .map(|(_, items)| items)
        .unwrap_or_default();
    let mut due: Vec<String> = {
        let mut q = c.prepare(
            "SELECT id FROM user_task WHERE done = 0 AND due = ?1 AND (start IS NULL OR start <= ?1) ORDER BY created",
        )?;
        let rows = q.query_map([day], |r| r.get::<_, String>(0))?;
        rows.map(|r| r.map(|id| format!("task:{id}")))
            .collect::<Result<_, _>>()?
    };
    let mut keys: Vec<String> = planned.into_iter().map(|p| p.key).collect();
    keys.append(&mut due);
    let mut seen = HashSet::new();
    for key in keys {
        if hidden.contains(&key) || finished_keys.contains(&key) || !seen.insert(key.clone()) {
            continue;
        }
        // A finished GitHub item stays open until it closes on GitHub. "Done" took it out above.
        if crate::items::is_open(c, &key).unwrap_or(false) {
            left.push(day_item(c, &key, None)?);
        }
    }

    let row: Option<String> = c
        .query_row("SELECT moved FROM day_review WHERE day = ?1", [day], |r| {
            r.get(0)
        })
        .optional()?;
    Ok(DayCheck {
        day: day.into(),
        done,
        left,
        finished: row.is_some(),
        moved: row
            .map(|m| serde_json::from_str(&m))
            .transpose()?
            .unwrap_or_default(),
    })
}

/// The end-of-day check of `day`.
pub fn day_check(dir: &Path, day: &str) -> anyhow::Result<DayCheck> {
    db::read(dir, |c| day_check_in(c, day))
}

/// Adds `key` at the end of the plan of `day`. A day with no plan gets a
/// plan with the mode plan::REVIEW. True when the key was already in the plan.
fn add_to_plan(tx: &Transaction, day: &str, key: &str) -> anyhow::Result<bool> {
    let (mode, mut items) =
        crate::plan::items_in(tx, day)?.unwrap_or((crate::plan::REVIEW.into(), Vec::new()));
    if items.iter().any(|i| i.key == key) {
        return Ok(true);
    }
    items.push(PlanItem {
        key: key.into(),
        why: None,
        goal: None,
    });
    crate::plan::set_in(tx, day, &mode, items, false)?;
    Ok(false)
}

/// Removes `key` from the plan of `day`. A plan with the mode plan::REVIEW
/// and no items goes away, so the day has no plan again.
fn remove_from_plan(tx: &Transaction, day: &str, key: &str) -> anyhow::Result<()> {
    if let Some((mode, mut items)) = crate::plan::items_in(tx, day)? {
        items.retain(|i| i.key != key);
        if mode == crate::plan::REVIEW && items.is_empty() {
            tx.execute("DELETE FROM day_plan WHERE day = ?1", [day])?;
        } else {
            crate::plan::set_in(tx, day, &mode, items, false)?;
        }
    }
    Ok(())
}

/// Closes a local task that the user gives away or drops. It is not
/// finished work: no "task done" jobs start, and AIfred keeps no real time.
/// Its last change does not move, so a later change by the user shows
/// (hidden_in).
fn close_task(tx: &Transaction, id: &str, now: u64) -> anyhow::Result<()> {
    crate::worktime::stop_in(tx, &format!("task:{id}"), now)?;
    tx.execute(
        "UPDATE task SET done = 1, done_at = ?1 WHERE doer = 'you' AND id = ?2 AND done = 0",
        rusqlite::params![now as i64, id],
    )?;
    Ok(())
}

/// The user finished the end-of-day check of `day`. Each open item in
/// `moved` goes into the plan of the next day. A local task that is due on
/// `day` gets the next day as its due day. A second finish of the same day
/// moves the new keys too.
pub fn finish_day(dir: &Path, day: &str, moved: &[String], now: u64) -> anyhow::Result<DayCheck> {
    let next = next_day(day)?;
    db::write(dir, |tx| {
        let mut kept = Vec::new();
        for key in moved {
            let key = key.trim();
            crate::items::split(key)?;
            if !crate::items::is_open(tx, key)? {
                continue;
            }
            add_to_plan(tx, &next, key)?;
            if let Some(id) = key.strip_prefix("task:") {
                tx.execute(
                    "UPDATE task SET due = ?1, updated = ?2 WHERE doer = 'you' AND id = ?3 AND due = ?4",
                    rusqlite::params![next, now as i64, id, day],
                )?;
            }
            kept.push(key.to_string());
        }
        let old: Vec<String> = tx
            .query_row("SELECT moved FROM day_review WHERE day = ?1", [day], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|m| serde_json::from_str(&m))
            .transpose()?
            .unwrap_or_default();
        let mut all = old;
        for k in kept {
            if !all.contains(&k) {
                all.push(k);
            }
        }
        tx.execute(
            "INSERT INTO day_review (day, moved, ts) VALUES (?1, ?2, ?3)
             ON CONFLICT (day) DO UPDATE SET moved = ?2, ts = ?3",
            rusqlite::params![day, serde_json::to_string(&all)?, now as i64],
        )?;
        day_check_in(tx, day)
    })
}

/// The update time that GitHub or ClickUp gives in the saved item, in Unix
/// seconds. GitHub gives a text time, ClickUp milliseconds.
fn source_updated(source: &str, data: &str) -> Option<u64> {
    let v: Value = serde_json::from_str(data).ok()?;
    match source {
        "github" => chrono::DateTime::parse_from_rfc3339(v["updated"].as_str()?)
            .ok()
            .map(|t| t.timestamp().max(0) as u64),
        "clickup" => v["updated"].as_u64().map(|ms| ms / 1000).filter(|&t| t > 0),
        _ => None,
    }
}

/// All open items that did not move in STALE_DAYS days, oldest first. An
/// item moves when it changes (in AIfred, GitHub, or ClickUp), when the
/// user works on it (worktime.rs), and when the user picks something for
/// it in a weekly review. A local task that starts after `today` waits on
/// purpose, so it is not in the list. Emails are not in the list: AIfred
/// reads only the last 14 days of Gmail.
fn stale_in(c: &Connection, today: &str, now: u64) -> anyhow::Result<Vec<StaleItem>> {
    let mut moved: HashMap<String, u64> = HashMap::new();
    let mut bump = |key: String, t: u64| {
        let e = moved.entry(key).or_insert(0);
        *e = (*e).max(t);
    };
    let mut q = c.prepare("SELECT key, MAX(ts) FROM review_pick GROUP BY key")?;
    for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?)))? {
        let (k, t) = r?;
        bump(k, t);
    }
    // A session that runs now moves the item now.
    let mut q = c.prepare("SELECT key, MAX(COALESCE(ended, ?1)) FROM work_session GROUP BY key")?;
    for r in q.query_map([now as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, u64>(1)?))
    })? {
        let (k, t) = r?;
        bump(k, t);
    }

    let hidden = hidden_in(c)?;
    let mut out = Vec::new();
    let mut push = |c: &Connection, key: String, changed: u64| -> anyhow::Result<()> {
        if hidden.contains(&key) {
            return Ok(());
        }
        let last = changed.max(moved.get(&key).copied().unwrap_or(0));
        if now.saturating_sub(last) < STALE_DAYS * 86_400 {
            return Ok(());
        }
        let (title, place) = title_place(c, &key)?;
        out.push(StaleItem {
            key,
            title,
            place,
            last,
            days: now.saturating_sub(last) / 86_400,
        });
        Ok(())
    };

    let mut q = c.prepare(
        "SELECT id, MAX(created, updated) FROM user_task WHERE done = 0 AND (start IS NULL OR start <= ?1)",
    )?;
    let tasks: Vec<(String, u64)> = q
        .query_map([today], |r| Ok((r.get(0)?, r.get(1)?)))?
        .collect::<Result<_, _>>()?;
    for (id, changed) in tasks {
        push(c, format!("task:{id}"), changed)?;
    }
    let mut q = c.prepare(
        "SELECT source, item_id, data, first_seen FROM work_item
         WHERE in_list = 1 AND source IN ('github', 'clickup')",
    )?;
    let items: Vec<(String, String, String, u64)> = q
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?
        .collect::<Result<_, _>>()?;
    for (source, id, data, first_seen) in items {
        let changed = source_updated(&source, &data).unwrap_or(first_seen);
        push(c, format!("{source}:{id}"), changed)?;
    }
    out.sort_by(|a, b| a.last.cmp(&b.last).then_with(|| a.key.cmp(&b.key)));
    Ok(out)
}

/// For each goal, the items that the user finished in the 7 days before
/// `now` and that served it. The goal of an item comes from the briefs and
/// the plans of the last 14 days (the newest one wins).
fn goals_week_in(c: &Connection, now: u64) -> anyhow::Result<Vec<GoalWeek>> {
    let from = now.saturating_sub(7 * 86_400) as i64;
    // A task that the user gave away or dropped is closed, but not finished.
    let hidden = hidden_in(c)?;
    let mut finished: HashSet<String> = HashSet::new();
    let mut q = c.prepare("SELECT id FROM user_task WHERE done = 1 AND done_at >= ?1")?;
    for r in q.query_map([from], |r| r.get::<_, String>(0))? {
        finished.insert(format!("task:{}", r?));
    }
    let mut q = c.prepare("SELECT key FROM work_done WHERE finished >= ?1")?;
    for r in q.query_map([from], |r| r.get::<_, String>(0))? {
        finished.insert(r?);
    }

    let since = crate::gmail::local_time(now as i64 - 14 * 86_400)
        .map(|d| d.format("%F").to_string())
        .unwrap_or_default();
    let mut goal_of: HashMap<String, i64> = HashMap::new();
    let mut q = c.prepare("SELECT goal FROM brief WHERE day >= ?1 ORDER BY day")?;
    for r in q.query_map([&since], |r| r.get::<_, String>(0))? {
        let m: BTreeMap<String, i64> = serde_json::from_str(&r?).unwrap_or_default();
        goal_of.extend(m);
    }
    let mut q = c.prepare("SELECT items FROM day_plan WHERE day >= ?1 ORDER BY day")?;
    for r in q.query_map([&since], |r| r.get::<_, String>(0))? {
        let items: Vec<PlanItem> = serde_json::from_str(&r?).unwrap_or_default();
        goal_of.extend(items.into_iter().filter_map(|i| i.goal.map(|g| (i.key, g))));
    }
    let mut count: HashMap<i64, u32> = HashMap::new();
    for key in finished.difference(&hidden) {
        if let Some(g) = goal_of.get(key) {
            *count.entry(*g).or_insert(0) += 1;
        }
    }
    Ok(crate::goals::list_in(c)?
        .into_iter()
        .map(|g| GoalWeek {
            done: count.get(&g.id).copied().unwrap_or(0),
            id: g.id,
            text: g.text,
        })
        .collect())
}

/// The day of the last weekly review, or None.
fn last_week_in(c: &Connection) -> anyhow::Result<Option<String>> {
    Ok(c.query_row("SELECT MAX(day) FROM week_review", [], |r| r.get(0))?)
}

/// True when the weekly review is due on `today`: WEEK_DAYS days after the
/// last one. The first review comes on a Friday, Saturday, or Sunday.
pub fn is_due(last: Option<&str>, today: &str) -> anyhow::Result<bool> {
    let t = parse_day(today)?;
    Ok(match last {
        None => t.weekday().num_days_from_monday() >= 4,
        Some(l) => (t - parse_day(l)?).num_days() >= WEEK_DAYS,
    })
}

/// The weekly review on `today`.
pub fn week(dir: &Path, today: &str, now: u64) -> anyhow::Result<WeekReview> {
    let goals_stale = crate::goals::status(dir, now)?.stale;
    db::read(dir, |c| {
        let last = last_week_in(c)?;
        let mut items = stale_in(c, today, now)?;
        let more = items.len().saturating_sub(STALE_MAX);
        items.truncate(STALE_MAX);
        Ok(WeekReview {
            due: is_due(last.as_deref(), today)?,
            last,
            items,
            more,
            goals: goals_week_in(c, now)?,
            goals_stale,
        })
    })
}

/// The user finished the weekly review on `today`.
pub fn finish_week(dir: &Path, today: &str, now: u64) -> anyhow::Result<WeekReview> {
    parse_day(today)?;
    db::write(dir, |tx| {
        tx.execute(
            "INSERT INTO week_review (day, ts) VALUES (?1, ?2) ON CONFLICT (day) DO UPDATE SET ts = ?2",
            rusqlite::params![today, now as i64],
        )?;
        Ok(())
    })?;
    week(dir, today, now)
}

fn pick_row(r: &rusqlite::Row) -> rusqlite::Result<Pick> {
    Ok(Pick {
        id: r.get(0)?,
        key: r.get(1)?,
        pick: r.get(2)?,
        title: r.get(3)?,
        place: r.get(4)?,
        note: r.get(5)?,
        ts: r.get(6)?,
    })
}

/// What a pick changed, so the undo can put it back.
#[derive(Debug, Default, serde::Serialize, serde::Deserialize)]
struct Undo {
    /// The day of the plan that got the item, when it was not in it before.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    plan: Option<String>,
    /// True when the pick changed the start day of a local task.
    #[serde(default)]
    set_start: bool,
    /// The start day before the pick, or None for no start day.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    start: Option<String>,
    /// The "waiting for" item that the pick made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    waiting: Option<i64>,
    /// True when the pick closed a local task.
    #[serde(default)]
    closed: bool,
}

/// The user's pick for an open item in the weekly review (see the top of
/// this file). `note` is the day for PLAN (after `today`), and the person
/// for DELEGATE.
pub fn pick(
    dir: &Path,
    key: &str,
    pick: &str,
    note: &str,
    today: &str,
    now: u64,
) -> anyhow::Result<Pick> {
    if !PICKS.contains(&pick) {
        bail!("a pick is \"do\", \"plan\", \"delegate\", or \"drop\", not \"{pick}\"");
    }
    let key = key.trim();
    let (source, id) = crate::items::split(key)?;
    let note = note.split_whitespace().collect::<Vec<_>>().join(" ");
    parse_day(today)?;
    db::write(dir, |tx| {
        if !crate::items::is_open(tx, key)? {
            bail!("the item {key} is not open any more");
        }
        let (title, place) = title_place(tx, key)?;
        let mut undo = Undo::default();
        let note = match pick {
            DO => {
                if !add_to_plan(tx, today, key)? {
                    undo.plan = Some(today.into());
                }
                String::new()
            }
            PLAN => {
                let day = note.as_str();
                parse_day(day)?;
                if day <= today {
                    bail!("pick a day after today. To work on it today, pick \"do\"");
                }
                if !add_to_plan(tx, day, key)? {
                    undo.plan = Some(day.into());
                }
                if source == "task" {
                    let (start, due): (Option<String>, Option<String>) = tx.query_row(
                        "SELECT start, due FROM user_task WHERE id = ?1",
                        [id],
                        |r| Ok((r.get(0)?, r.get(1)?)),
                    )?;
                    // A start day after the due day is not allowed (tasks.rs).
                    if due.as_deref().is_none_or(|d| d >= day) {
                        tx.execute(
                            "UPDATE task SET start = ?1, updated = ?2 WHERE doer = 'you' AND id = ?3",
                            rusqlite::params![day, now as i64, id],
                        )?;
                        undo.set_start = true;
                        undo.start = start;
                    }
                }
                note
            }
            DELEGATE => {
                if note.is_empty() {
                    bail!("say who does it");
                }
                let what: String = title.chars().take(crate::waiting::MAX_CHARS).collect();
                let w = crate::waiting::add_in(
                    dir,
                    tx,
                    crate::waiting::NewWaiting {
                        what,
                        who: note.clone(),
                        since: Some(today.into()),
                        ..Default::default()
                    },
                    now,
                )?;
                undo.waiting = Some(w.id);
                if source == "task" {
                    close_task(tx, id, now)?;
                    undo.closed = true;
                }
                note
            }
            _ => {
                if source == "task" {
                    close_task(tx, id, now)?;
                    undo.closed = true;
                }
                String::new()
            }
        };
        tx.execute(
            "INSERT INTO review_pick (key, pick, title, place, note, undo, ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            rusqlite::params![key, pick, title, place, note, serde_json::to_string(&undo)?, now as i64],
        )?;
        Ok(Pick {
            id: tx.last_insert_rowid(),
            key: key.into(),
            pick: pick.into(),
            title,
            place,
            note,
            ts: now,
        })
    })
}

/// Takes back a pick: the plan, the task, and the "waiting for" list are
/// as before the pick. A start day that the user changed after the pick
/// stays.
pub fn undo_pick(dir: &Path, id: i64, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        let row: Option<(String, String, String)> = tx
            .query_row(
                "SELECT key, note, undo FROM review_pick WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let Some((key, note, undo)) = row else {
            bail!("no pick with id {id}");
        };
        let undo: Undo = serde_json::from_str(&undo)?;
        let task = key.strip_prefix("task:");
        if undo.closed
            && let Some(t) = task
        {
            // The pick did not finish the task (close_task), so there is no
            // real time and no job to take back.
            tx.execute(
                "UPDATE task SET done = 0, done_at = NULL WHERE doer = 'you' AND id = ?1 AND done = 1",
                [t],
            )?;
        }
        if let Some(day) = &undo.plan {
            remove_from_plan(tx, day, &key)?;
        }
        if undo.set_start
            && let Some(t) = task
        {
            // `note` is the start day that the pick set.
            tx.execute(
                "UPDATE task SET start = ?1, updated = ?2 WHERE doer = 'you' AND id = ?3 AND start = ?4",
                rusqlite::params![undo.start, now as i64, t, note],
            )?;
        }
        if let Some(w) = undo.waiting {
            tx.execute("DELETE FROM waiting WHERE id = ?1", [w])?;
        }
        tx.execute("DELETE FROM review_pick WHERE id = ?1", [id])?;
        Ok(())
    })
}

/// The picks for the AI, newest first. Only the newest pick of each item.
/// A "do" or "plan" pick is in the list for RECENT_DAYS days. A "delegate"
/// or "drop" pick is in the list while the reviews hide its open item
/// (hidden_in), so the brief and the reviews agree.
pub(crate) fn for_ai(c: &Connection, now: u64) -> anyhow::Result<Vec<Value>> {
    let mut q = c.prepare(
        "SELECT id, key, pick, title, place, note, ts FROM review_pick ORDER BY ts DESC, id DESC",
    )?;
    let rows: Vec<Pick> = q.query_map([], pick_row)?.collect::<Result<_, _>>()?;
    let hidden = hidden_in(c)?;
    let from = now.saturating_sub(RECENT_DAYS * 86_400);
    let mut seen = HashSet::new();
    let mut keep = Vec::new();
    for p in rows {
        if !seen.insert(p.key.clone()) {
            continue;
        }
        let in_force = if p.pick == DELEGATE || p.pick == DROP {
            hidden.contains(&p.key) && crate::items::is_open(c, &p.key).unwrap_or(false)
        } else {
            p.ts >= from
        };
        if in_force {
            keep.push(p);
        }
    }
    Ok(keep
        .into_iter()
        .map(|p| {
            let day = crate::gmail::local_time(p.ts as i64)
                .map(|d| d.format("%F").to_string())
                .unwrap_or_default();
            let mut v = json!({ "key": p.key, "title": p.title, "pick": p.pick, "day": day });
            if !p.note.is_empty() {
                v[if p.pick == PLAN {
                    "plan_day"
                } else {
                    "given_to"
                }] = p.note.into();
            }
            v
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{TaskFields, TaskStore};

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-review-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Noon of a local day, in Unix seconds.
    fn noon(day: &str) -> u64 {
        (day_bounds(day).unwrap().0 + 12 * 3600) as u64
    }

    fn task(s: &TaskStore, title: &str, due: Option<&str>) -> String {
        let t = s
            .create(
                TaskFields {
                    title: Some(title.into()),
                    due: due.map(String::from),
                    ..Default::default()
                },
                "you",
            )
            .unwrap();
        format!("task:{}", t.id)
    }

    /// Sets the times of a task, as if it was added and changed at `t`.
    fn age(d: &Path, key: &str, t: u64) {
        db::write(d, |tx| {
            tx.execute(
                "UPDATE task SET created = ?1, updated = ?1 WHERE doer = 'you' AND id = ?2",
                rusqlite::params![t as i64, key.strip_prefix("task:").unwrap()],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn keys<T>(list: &[T], key: impl Fn(&T) -> &str) -> Vec<String> {
        list.iter().map(|i| key(i).to_string()).collect()
    }

    fn add_item(d: &Path, source: &str, id: &str, data: &str, first_seen: u64) {
        db::write(d, |tx| {
            tx.execute(
                "INSERT INTO work_item (source, item_id, title, place, url, data, in_list, pos, first_seen, last_seen)
                 VALUES (?1, ?2, 'Item', 'acme/shop', '', ?3, 1, 0, ?4, ?4)",
                rusqlite::params![source, id, data, first_seen as i64],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_day_check_shows_what_is_done_and_what_is_left() {
        let d = temp("day");
        let s = TaskStore::new(&d);
        let day = crate::plan::today();
        let a = task(&s, "Planned and done", None);
        let b = task(&s, "Planned, open", None);
        let c = task(&s, "Due today", Some(&day));
        let e = task(&s, "Not today", None);
        crate::plan::set(
            &d,
            &day,
            "ai",
            [&a, &b]
                .iter()
                .map(|k| PlanItem {
                    key: k.to_string(),
                    why: None,
                    goal: None,
                })
                .collect(),
            false,
        )
        .unwrap();
        s.set_done(a.strip_prefix("task:").unwrap(), true).unwrap();
        let check = day_check(&d, &day).unwrap();
        assert_eq!(keys(&check.done, |i| &i.key), vec![a.clone()]);
        assert_eq!(keys(&check.left, |i| &i.key), [b.clone(), c.clone()]);
        assert!(!check.finished);
        assert!(!keys(&check.left, |i| &i.key).contains(&e));

        let tomorrow = next_day(&day).unwrap();
        let after = finish_day(&d, &day, &[b.clone(), c.clone(), a.clone()], 10).unwrap();
        assert!(after.finished);
        assert_eq!(
            after.moved,
            [b.clone(), c.clone()],
            "a done item does not move"
        );
        let plan = crate::plan::get(&d, &tomorrow).unwrap().unwrap();
        assert_eq!(
            plan.mode,
            crate::plan::REVIEW,
            "the app still asks how to plan"
        );
        assert_eq!(keys(&plan.items, |i| &i.key), [b.clone(), c.clone()]);
        let tasks = s.load().unwrap();
        let due_of = |k: &str| {
            tasks
                .iter()
                .find(|t| format!("task:{}", t.id) == k)
                .unwrap()
                .due
                .clone()
        };
        assert_eq!(
            due_of(&c),
            Some(tomorrow.clone()),
            "a task due today is due tomorrow"
        );
        assert_eq!(due_of(&b), None);
        // The task that moved is not due today any more, so it left the check.
        let again = day_check(&d, &day).unwrap();
        assert_eq!(keys(&again.left, |i| &i.key), vec![b.clone()]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_weekly_review_is_due_every_seven_days_and_first_on_a_friday() {
        assert!(!is_due(None, "2026-11-05").unwrap(), "a Thursday");
        assert!(is_due(None, "2026-11-06").unwrap(), "a Friday");
        assert!(is_due(None, "2026-11-08").unwrap(), "a Sunday");
        assert!(!is_due(Some("2026-11-06"), "2026-11-12").unwrap());
        assert!(is_due(Some("2026-11-06"), "2026-11-13").unwrap());
        assert!(
            is_due(Some("2026-11-02"), "2026-11-10").unwrap(),
            "late is due"
        );
        assert!(is_due(Some("bad"), "2026-11-10").is_err());
    }

    #[test]
    fn items_that_did_not_move_for_three_weeks_are_in_the_review() {
        let d = temp("stale");
        let s = TaskStore::new(&d);
        let now = noon("2026-11-06");
        let day = 86_400;
        let old = task(&s, "Old", None);
        age(&d, &old, now - 30 * day);
        let fresh = task(&s, "Fresh", None);
        age(&d, &fresh, now - 3 * day);
        let worked = task(&s, "Worked on", None);
        age(&d, &worked, now - 40 * day);
        db::write(&d, |tx| {
            tx.execute(
                "INSERT INTO work_session (key, started, ended) VALUES (?1, ?2, ?3)",
                rusqlite::params![worked, (now - 2 * day) as i64, (now - 2 * day + 600) as i64],
            )?;
            Ok(())
        })
        .unwrap();
        let later = task(&s, "Starts later", None);
        age(&d, &later, now - 60 * day);
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE task SET start = '2026-12-01' WHERE doer = 'you' AND id = ?1",
                [later.strip_prefix("task:").unwrap()],
            )?;
            Ok(())
        })
        .unwrap();
        add_item(
            &d,
            "github",
            "acme/shop#1",
            r#"{"updated":"2026-09-01T10:00:00Z"}"#,
            now - day,
        );
        add_item(
            &d,
            "github",
            "acme/shop#2",
            r#"{"updated":"2026-11-01T10:00:00Z"}"#,
            now - 90 * day,
        );
        add_item(&d, "clickup", "86a", r#"{"updated":1756000000000}"#, now);
        add_item(&d, "clickup", "86b", "{}", now - 25 * day);
        add_item(&d, "gmail", "m1", "{}", now - 60 * day);

        let w = week(&d, "2026-11-06", now).unwrap();
        assert_eq!(
            keys(&w.items, |i| &i.key),
            [
                "clickup:86a",
                "github:acme/shop#1",
                old.as_str(),
                "clickup:86b"
            ],
            "oldest first; not fresh, worked on, waiting to start, or email"
        );
        assert_eq!(w.items[2].days, 30);
        assert_eq!(w.items[1].place, "GitHub acme/shop");
        assert!(w.due && w.last.is_none());

        let w = finish_week(&d, "2026-11-06", now).unwrap();
        assert_eq!(w.last.as_deref(), Some("2026-11-06"));
        assert!(!w.due);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn picks_change_the_plan_the_task_and_the_waiting_list_and_undo_puts_them_back() {
        let d = temp("pick");
        let s = TaskStore::new(&d);
        let today = "2026-11-06";
        let now = noon(today);
        let a = task(&s, "Do it", None);
        let b = task(&s, "Plan it", Some("2026-11-20"));
        let c = task(&s, "Give it", None);
        let e = task(&s, "Drop it", None);
        add_item(&d, "github", "acme/shop#1", "{}", 0);

        let p = pick(&d, &a, DO, "", today, now).unwrap();
        assert_eq!(
            keys(&crate::plan::get(&d, today).unwrap().unwrap().items, |i| &i
                .key),
            vec![a.clone()]
        );
        undo_pick(&d, p.id, now).unwrap();
        assert!(crate::plan::get(&d, today).unwrap().is_none());

        assert!(pick(&d, &b, PLAN, today, today, now).is_err(), "not today");
        assert!(pick(&d, &b, PLAN, "someday", today, now).is_err());
        let p = pick(&d, &b, PLAN, "2026-11-10", today, now).unwrap();
        assert_eq!(p.note, "2026-11-10");
        let start = |k: &str| {
            s.load()
                .unwrap()
                .into_iter()
                .find(|t| format!("task:{}", t.id) == k)
                .unwrap()
                .start
        };
        assert_eq!(start(&b).as_deref(), Some("2026-11-10"));
        assert_eq!(
            keys(
                &crate::plan::get(&d, "2026-11-10").unwrap().unwrap().items,
                |i| &i.key
            ),
            vec![b.clone()]
        );
        undo_pick(&d, p.id, now).unwrap();
        assert_eq!(start(&b), None);

        assert!(pick(&d, &c, DELEGATE, "  ", today, now).is_err(), "who?");
        let p = pick(&d, &c, DELEGATE, " Ana ", today, now).unwrap();
        assert_eq!(p.note, "Ana");
        let w = crate::waiting::list(&d, false, now).unwrap();
        assert_eq!((w[0].what.as_str(), w[0].who.as_str()), ("Give it", "Ana"));
        assert!(pick(&d, &c, DROP, "", today, now).is_err(), "closed now");
        undo_pick(&d, p.id, now).unwrap();
        assert!(crate::waiting::list(&d, false, now).unwrap().is_empty());
        db::read(&d, |cn| {
            assert!(crate::items::is_open(cn, &c)?);
            Ok(())
        })
        .unwrap();

        pick(&d, &e, DROP, "", today, now).unwrap();
        let gh = pick(&d, "github:acme/shop#1", DROP, "", today, now).unwrap();
        db::read(&d, |cn| {
            assert!(!crate::items::is_open(cn, &e)?, "a dropped task closes");
            assert!(
                crate::items::is_open(cn, "github:acme/shop#1")?,
                "GitHub does not change"
            );
            // The closed task is not in the overview, so only GitHub is here.
            let ai = for_ai(cn, now)?;
            assert_eq!(ai.len(), 1);
            assert_eq!(
                (ai[0]["key"].as_str(), ai[0]["pick"].as_str()),
                (Some("github:acme/shop#1"), Some("drop"))
            );
            // The drop is in force after RECENT_DAYS days too.
            assert_eq!(for_ai(cn, now + 40 * 86_400)?.len(), 1);
            Ok(())
        })
        .unwrap();
        // A dropped item is not in the next review, also after three weeks.
        let w = week(&d, "2026-12-30", noon("2026-12-30")).unwrap();
        assert!(!keys(&w.items, |i| &i.key).contains(&gh.key));
        assert!(!keys(&w.items, |i| &i.key).contains(&e));
        // The dropped task is not "done today".
        assert!(
            day_check(&d, &crate::plan::today())
                .unwrap()
                .done
                .iter()
                .all(|i| i.key != e)
        );
        assert!(pick(&d, &a, "later", "", today, now).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dropped_item_counts_again_after_it_changes() {
        let d = temp("reopen");
        let s = TaskStore::new(&d);
        let today = crate::plan::today();
        let today = today.as_str();
        // The pick was an hour ago, so the changes below come after it.
        let now = crate::files::now() - 3600;
        let a = task(&s, "Drop it", None);
        age(&d, &a, now - 30 * 86_400);
        add_item(
            &d,
            "github",
            "acme/shop#1",
            r#"{"updated":"2026-09-01T10:00:00Z"}"#,
            0,
        );
        pick(&d, &a, DROP, "", today, now).unwrap();
        pick(&d, "github:acme/shop#1", DROP, "", today, now).unwrap();
        let hidden = db::read(&d, hidden_in).unwrap();
        assert!(hidden.contains(&a) && hidden.contains("github:acme/shop#1"));

        // The user opens the task again, and GitHub has new activity.
        s.set_done(a.strip_prefix("task:").unwrap(), false).unwrap();
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE work_item SET data = ?1 WHERE item_id = 'acme/shop#1'",
                [json!({ "updated": chrono::Utc::now().to_rfc3339() }).to_string()],
            )?;
            Ok(())
        })
        .unwrap();
        db::read(&d, |cn| {
            assert!(hidden_in(cn)?.is_empty());
            assert!(for_ai(cn, now)?.is_empty(), "the brief can name them again");
            Ok(())
        })
        .unwrap();
        // The task is in a later review again, and it is "done" when it is done.
        let later = crate::gmail::local_time((now + 30 * 86_400) as i64)
            .unwrap()
            .format("%F")
            .to_string();
        let w = week(&d, &later, noon(&later)).unwrap();
        assert!(keys(&w.items, |i| &i.key).contains(&a));
        s.set_done(a.strip_prefix("task:").unwrap(), true).unwrap();
        let check = day_check(&d, &crate::plan::today()).unwrap();
        assert!(keys(&check.done, |i| &i.key).contains(&a));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dropped_task_is_not_finished_work() {
        let d = temp("notdone");
        let s = TaskStore::new(&d);
        // The task first: the first open of the database can take a second,
        // and the drop must not come before the last change of the task.
        let a = task(&s, "Drop it", None);
        let now = crate::files::now();
        let today = crate::plan::today();
        let g = crate::goals::add(&d, "Ship 2.0", now).unwrap();
        crate::worktime::start(&d, &a, now - 2400).unwrap();
        crate::brief::save(
            &d,
            &crate::brief::Brief {
                day: today.clone(),
                written: now,
                text: "Hi.".into(),
                why: BTreeMap::new(),
                goal: BTreeMap::from([(a.clone(), g.id)]),
                ..Default::default()
            },
            &BTreeMap::new(),
        )
        .unwrap();
        let p = pick(&d, &a, DROP, "", &today, now).unwrap();
        db::read(&d, |cn| {
            let count = |sql: &str| cn.query_row(sql, [], |r| r.get::<_, i64>(0));
            assert_eq!(count("SELECT COUNT(*) FROM work_done")?, 0, "no real time");
            assert_eq!(
                count("SELECT COUNT(*) FROM job_event")?,
                0,
                "no \"task done\" jobs"
            );
            assert_eq!(
                count("SELECT COUNT(*) FROM work_session WHERE ended IS NULL")?,
                0,
                "the work stops"
            );
            Ok(())
        })
        .unwrap();
        assert_eq!(
            week(&d, &today, now).unwrap().goals[0].done,
            0,
            "not done for the goal"
        );
        undo_pick(&d, p.id, now).unwrap();
        db::read(&d, |cn| {
            assert!(crate::items::is_open(cn, &a)?);
            Ok(())
        })
        .unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_undo_takes_away_a_plan_that_the_pick_made() {
        let d = temp("noplan");
        let s = TaskStore::new(&d);
        let today = "2026-11-06";
        let now = noon(today);
        let a = task(&s, "Do it", None);
        let p = pick(&d, &a, DO, "", today, now).unwrap();
        assert_eq!(
            crate::plan::get(&d, today).unwrap().unwrap().mode,
            crate::plan::REVIEW
        );
        undo_pick(&d, p.id, now).unwrap();
        assert!(
            crate::plan::get(&d, today).unwrap().is_none(),
            "the day has no plan again"
        );

        // A plan that the user made stays, also with no items.
        crate::plan::set(&d, today, "self", vec![], false).unwrap();
        let p = pick(&d, &a, DO, "", today, now).unwrap();
        undo_pick(&d, p.id, now).unwrap();
        assert_eq!(crate::plan::get(&d, today).unwrap().unwrap().mode, "self");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_undo_keeps_a_start_day_that_the_user_changed_after_the_pick() {
        let d = temp("start");
        let s = TaskStore::new(&d);
        let today = "2026-11-06";
        let now = noon(today);
        let a = task(&s, "Plan it", None);
        let id = a.strip_prefix("task:").unwrap();
        let p = pick(&d, &a, PLAN, "2026-11-10", today, now).unwrap();
        s.update(
            id,
            TaskFields {
                start: Some("2026-11-12".into()),
                ..Default::default()
            },
        )
        .unwrap();
        undo_pick(&d, p.id, now).unwrap();
        let t = s.load().unwrap().into_iter().find(|t| t.id == id).unwrap();
        assert_eq!(t.start.as_deref(), Some("2026-11-12"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_review_counts_finished_items_for_each_goal() {
        let d = temp("goals");
        let s = TaskStore::new(&d);
        let now = crate::files::now();
        let g1 = crate::goals::add(&d, "Ship 2.0", now).unwrap();
        let g2 = crate::goals::add(&d, "Hire", now).unwrap();
        let a = task(&s, "A", None);
        let b = task(&s, "B", None);
        let today = crate::plan::today();
        crate::brief::save(
            &d,
            &crate::brief::Brief {
                day: today.clone(),
                written: now,
                text: "Hi.".into(),
                why: BTreeMap::new(),
                goal: BTreeMap::from([(a.clone(), g1.id), (b.clone(), g1.id)]),
                ..Default::default()
            },
            &BTreeMap::new(),
        )
        .unwrap();
        s.set_done(a.strip_prefix("task:").unwrap(), true).unwrap();
        let w = week(&d, &today, now).unwrap();
        assert_eq!(
            w.goals.iter().map(|g| (g.id, g.done)).collect::<Vec<_>>(),
            [(g1.id, 1), (g2.id, 0)]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
