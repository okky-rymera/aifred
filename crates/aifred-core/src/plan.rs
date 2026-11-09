//! The day plan: the items that the user wants to do on one day, in order,
//! and a time guess for each item. The window places the items on the
//! work day, around lunch (settings.rs). The user makes the plan by hand, or
//! AIfred makes it in the plan chat with the day_plan tools (aifred-tools).

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::{Context, bail};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::{db, items};

/// One item in the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// The item key (items.rs).
    pub key: String,
    /// Why the item is at this place, when AIfred placed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
    /// The id of the goal that the item serves (goals.rs), when AIfred knows it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub goal: Option<i64>,
    /// A fixed start time, in clock minutes after midnight (0..1440). The
    /// user sets it when they drag a block to a time, or the AI sets it.
    /// Without it, the window places the item in the next free time.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayPlan {
    /// "YYYY-MM-DD".
    pub day: String,
    /// "ai": AIfred made the plan. "self": the user made it. REVIEW: a
    /// review put items in a day with no plan (review.rs). The user did not
    /// pick yet how to plan the day, so the app still asks.
    pub mode: String,
    /// The first item first.
    pub items: Vec<PlanItem>,
    /// When the plan changed last, in Unix seconds.
    pub updated: u64,
}

/// A time guess for an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub minutes: u32,
    /// "you" or "aifred".
    pub by: String,
}

/// The plan mode of a day that has only items from the reviews (DayPlan::mode).
pub const REVIEW: &str = "review";

/// The shortest and the longest time guess, in minutes.
pub const MIN_MINUTES: u32 = 5;
pub const MAX_MINUTES: u32 = 600;

/// Today in the user's own time zone, "YYYY-MM-DD".
pub fn today() -> String {
    chrono::Local::now().format("%F").to_string()
}

/// The days that the app shows: today and the next PLAN_DAYS - 1 days.
pub const PLAN_DAYS: i64 = 7;

/// An error when `day` is not today or one of the next PLAN_DAYS - 1 days.
/// The app shows only these days, so an item on another day goes off the calendar.
pub fn check_shown_day(day: &str, today: &str) -> anyhow::Result<()> {
    check_day(day)?;
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .with_context(|| format!("bad day {day}"))?;
    let t = chrono::NaiveDate::parse_from_str(today, "%Y-%m-%d")
        .with_context(|| format!("bad day {today}"))?;
    let n = (d - t).num_days();
    if !(0..PLAN_DAYS).contains(&n) {
        bail!(
            "{day} is not in the plans that the user sees: use a day from {today} to {}",
            t + chrono::Days::new(PLAN_DAYS as u64 - 1)
        );
    }
    Ok(())
}

pub(crate) fn check_day(day: &str) -> anyhow::Result<()> {
    let ok = day.len() == 10
        && day.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !ok {
        bail!("the day \"{day}\" is not in the form YYYY-MM-DD");
    }
    Ok(())
}

/// The minutes after midnight of a clock time "H:MM" or "HH:MM".
pub fn parse_time(t: &str) -> anyhow::Result<u32> {
    let bad = || anyhow::anyhow!("the time \"{t}\" is not in the form HH:MM");
    let (h, m) = t.trim().split_once(':').ok_or_else(bad)?;
    // Only digits: "+9:30" and "9:5" are errors, so the AI can fix its call.
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    if !(1..=2).contains(&h.len()) || m.len() != 2 || !digits(h) || !digits(m) {
        return Err(bad());
    }
    let h: u32 = h.parse().map_err(|_| bad())?;
    let m: u32 = m.parse().map_err(|_| bad())?;
    if h > 23 || m > 59 {
        return Err(bad());
    }
    Ok(h * 60 + m)
}

/// "HH:MM" of minutes after midnight.
pub fn time_text(m: u32) -> String {
    format!("{:02}:{:02}", m / 60, m % 60)
}

/// The plan of `day`, or None when the day has no plan.
pub fn get(dir: &Path, day: &str) -> anyhow::Result<Option<DayPlan>> {
    check_day(day)?;
    db::read(dir, |c| {
        let row: Option<(String, String, u64)> = c
            .query_row(
                "SELECT mode, items, updated FROM day_plan WHERE day = ?1",
                [day],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((mode, items, updated)) => Some(DayPlan {
                day: day.into(),
                mode,
                items: serde_json::from_str(&items)?,
                updated,
            }),
            None => None,
        })
    })
}

/// Saves the plan of `day`. An item key that is twice in the list is kept
/// only at its first place.
///
/// A key that is already in the saved plan of `day` stays, also when its
/// item is now done: a finished item keeps its place on the day. A new key
/// must be of an open item.
///
/// `strict`: a new key of an item that is not open is an error, with the bad
/// keys in the text, so the AI can fix its call. Without `strict` (the
/// window), such keys are dropped: an item can close while the plan is open.
/// A goal id that does not exist is an error with `strict`. Without
/// `strict`, it is dropped: the user can remove a goal while the plan is open.
pub fn set(
    dir: &Path,
    day: &str,
    mode: &str,
    items: Vec<PlanItem>,
    strict: bool,
) -> anyhow::Result<DayPlan> {
    check_day(day)?;
    if mode != "ai" && mode != "self" && mode != REVIEW {
        bail!("the plan mode must be \"ai\" or \"self\", not \"{mode}\"");
    }
    db::write(dir, |tx| set_in(tx, day, mode, items, strict))
}

/// set() for the AI (strict). An item with no `at` keeps its old fixed
/// time, except the items in `clear_at`: they lose their fixed time.
pub fn set_ai(
    dir: &Path,
    day: &str,
    items: Vec<PlanItem>,
    clear_at: &HashSet<String>,
) -> anyhow::Result<DayPlan> {
    check_day(day)?;
    db::write(dir, |tx| {
        set_with(tx, day, "ai", items, true, Some(clear_at))
    })
}

/// set(), in a change that is open. Also for the reviews (review.rs).
pub(crate) fn set_in(
    tx: &rusqlite::Transaction,
    day: &str,
    mode: &str,
    items: Vec<PlanItem>,
    strict: bool,
) -> anyhow::Result<DayPlan> {
    set_with(tx, day, mode, items, strict, None)
}

/// set_in(). `keep_at`: Some for the AI (set_ai). Then an item with no `at`
/// keeps its old fixed time, except the items in the set: they lose it.
/// None: each item has the time that it has in `items`.
fn set_with(
    tx: &rusqlite::Transaction,
    day: &str,
    mode: &str,
    items: Vec<PlanItem>,
    strict: bool,
    keep_at: Option<&HashSet<String>>,
) -> anyhow::Result<DayPlan> {
    {
        let old_items: Vec<PlanItem> = tx
            .query_row("SELECT items FROM day_plan WHERE day = ?1", [day], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|j| serde_json::from_str::<Vec<PlanItem>>(&j))
            .transpose()?
            .unwrap_or_default();
        // The fixed times of the saved plan. The AI sends the full plan
        // again for each change. When it leaves out a time, the item keeps
        // it, so a block that the user dragged to a time does not move.
        let old_at: HashMap<String, u32> = old_items
            .iter()
            .filter_map(|i| i.at.map(|a| (i.key.clone(), a)))
            .collect();
        let old: HashSet<String> = old_items.into_iter().map(|i| i.key).collect();
        let mut seen = HashSet::new();
        let mut keep = Vec::new();
        let mut bad = Vec::new();
        let mut bad_goals = Vec::new();
        for mut i in items {
            i.key = i.key.trim().to_string();
            i.why = i
                .why
                .map(|w| w.trim().to_string())
                .filter(|w| !w.is_empty());
            if !seen.insert(i.key.clone()) {
                continue;
            }
            if let Some(a) = i.at
                && a >= 1440
            {
                if strict {
                    bail!("the time of {} is not a time of the day", i.key);
                }
                i.at = None;
            }
            if let Some(clear_at) = keep_at
                && i.at.is_none()
                && !clear_at.contains(&i.key)
            {
                i.at = old_at.get(&i.key).copied();
            }
            if let Some(g) = i.goal
                && !crate::goals::exists(tx, g)?
            {
                bad_goals.push(g.to_string());
                i.goal = None;
            }
            if old.contains(&i.key) || items::is_open(tx, &i.key).unwrap_or(false) {
                keep.push(i);
            } else {
                bad.push(i.key);
            }
        }
        if strict && !bad.is_empty() {
            bail!(
                "these keys are not open items: {}. Take the keys from the overview tool",
                bad.join(", ")
            );
        }
        if strict && !bad_goals.is_empty() {
            bail!(
                "these goal ids do not exist: {}. Take the goal ids from the overview tool",
                bad_goals.join(", ")
            );
        }
        let plan = DayPlan {
            day: day.into(),
            mode: mode.into(),
            items: keep,
            updated: crate::files::now(),
        };
        tx.execute(
            "INSERT INTO day_plan (day, mode, items, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (day) DO UPDATE SET mode = ?2, items = ?3, updated = ?4",
            rusqlite::params![
                plan.day,
                plan.mode,
                serde_json::to_string(&plan.items)?,
                plan.updated as i64
            ],
        )?;
        Ok(plan)
    }
}

/// The mode and the items of the plan of `day`, or None when the day has no plan.
pub(crate) fn items_in(
    c: &rusqlite::Connection,
    day: &str,
) -> anyhow::Result<Option<(String, Vec<PlanItem>)>> {
    c.query_row(
        "SELECT mode, items FROM day_plan WHERE day = ?1",
        [day],
        |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)),
    )
    .optional()?
    .map(|(mode, items)| Ok((mode, serde_json::from_str(&items)?)))
    .transpose()
}

/// The plans of the days from `from` to `to`, both included, first day first.
/// A day with no plan is not in the list.
pub fn range(dir: &Path, from: &str, to: &str) -> anyhow::Result<Vec<DayPlan>> {
    check_day(from)?;
    check_day(to)?;
    db::read(dir, |c| {
        let mut q = c.prepare(
            "SELECT day, mode, items, updated FROM day_plan WHERE day >= ?1 AND day <= ?2 ORDER BY day",
        )?;
        let rows = q.query_map([from, to], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, u64>(3)?,
            ))
        })?;
        let mut out = Vec::new();
        for r in rows {
            let (day, mode, items, updated) = r?;
            out.push(DayPlan {
                day,
                mode,
                items: serde_json::from_str(&items)?,
                updated,
            });
        }
        Ok(out)
    })
}

/// Moves the item `key` from the plan of `from` to the plan of `to`, in
/// one change, so the item cannot get lost between two saves. `at` is its
/// new fixed time, or None to place it in the next free time. `index` is its
/// place in the new day (None: at the end). It keeps its reason and goal.
/// A day with no plan, or a plan from the reviews, becomes the user's own plan.
/// Returns the two plans after the move. The first is None when the old day
/// has no plan now.
pub fn move_item(
    dir: &Path,
    from: &str,
    to: &str,
    key: &str,
    at: Option<u32>,
    index: Option<usize>,
) -> anyhow::Result<(Option<DayPlan>, DayPlan)> {
    check_day(from)?;
    check_day(to)?;
    if from == to {
        bail!("the item is already on {to}");
    }
    if at.is_some_and(|a| a >= 1440) {
        bail!("the time is not a time of the day");
    }
    db::write(dir, |tx| {
        let (from_mode, mut from_items) = items_in(tx, from)?.unwrap_or_default();
        let Some(n) = from_items.iter().position(|i| i.key == key) else {
            bail!("{key} is not in the plan of {from}");
        };
        // A local task waits until its start day, so it cannot go on a day before it.
        if let Some(id) = key.strip_prefix("task:") {
            let start: Option<String> = tx
                .query_row("SELECT start FROM user_task WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?
                .flatten();
            if let Some(start) = start.filter(|s| !s.is_empty() && s.as_str() > to) {
                bail!("{key} starts on {start}, so it cannot move to {to}");
            }
        }
        // An item that closed can stay on its own day, but it does not move.
        // Check it here: the save below keeps a key that is already in the
        // plan of `to`, also when its item is closed.
        if !items::is_open(tx, key).unwrap_or(false) {
            bail!("{key} is not open, so it cannot move to {to}");
        }
        let mut item = from_items.remove(n);
        item.at = at;
        let (to_mode, mut to_items) = items_in(tx, to)?.unwrap_or_default();
        to_items.retain(|i| i.key != key);
        let k = index.unwrap_or(to_items.len()).min(to_items.len());
        to_items.insert(k, item);
        let to_mode = if to_mode == "ai" { "ai" } else { "self" };
        let moved = set_in(tx, to, to_mode, to_items, false)?;
        // An empty plan from the reviews goes away, as in review.rs
        // remove_from_plan, so the day has no plan again.
        let left = if from_mode == REVIEW && from_items.is_empty() {
            tx.execute("DELETE FROM day_plan WHERE day = ?1", [from])?;
            None
        } else {
            Some(set_in(tx, from, &from_mode, from_items, false)?)
        };
        Ok((left, moved))
    })
}

/// Removes the plan of `day`. The plan chat of the day stays.
pub fn clear(dir: &Path, day: &str) -> anyhow::Result<()> {
    check_day(day)?;
    db::write(dir, |tx| {
        tx.execute("DELETE FROM day_plan WHERE day = ?1", [day])?;
        Ok(())
    })
}

/// All time guesses, by item key.
pub fn estimates(dir: &Path) -> anyhow::Result<BTreeMap<String, Estimate>> {
    db::read(dir, |c| {
        let mut q = c.prepare("SELECT key, minutes, by FROM estimate")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Estimate {
                    minutes: r.get(1)?,
                    by: r.get(2)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// Saves a time guess. None removes it. A guess from AIfred does not replace
/// a guess from the user, so AIfred cannot undo the user's own change.
/// Returns false when it kept the user's guess.
pub fn set_estimate(dir: &Path, key: &str, minutes: Option<u32>, by: &str) -> anyhow::Result<bool> {
    items::split(key)?;
    db::write(dir, |tx| set_estimate_in(tx, key, minutes, by))
}

pub(crate) fn set_estimate_in(
    tx: &rusqlite::Transaction,
    key: &str,
    minutes: Option<u32>,
    by: &str,
) -> anyhow::Result<bool> {
    if by != "you" && by != "aifred" {
        bail!("a time guess is by \"you\" or \"aifred\"");
    }
    if by == "aifred" {
        let theirs: Option<String> = tx
            .query_row("SELECT by FROM estimate WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        if theirs.as_deref() == Some("you") {
            return Ok(false);
        }
    }
    match minutes {
        Some(m) => {
            let m = m.clamp(MIN_MINUTES, MAX_MINUTES);
            tx.execute(
                "INSERT INTO estimate (key, minutes, by) VALUES (?1, ?2, ?3) ON CONFLICT (key) DO UPDATE SET minutes = ?2, by = ?3",
                rusqlite::params![key, m, by],
            )?;
        }
        None => {
            tx.execute("DELETE FROM estimate WHERE key = ?1", [key])?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-plan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn item(key: &str) -> PlanItem {
        PlanItem {
            key: key.into(),
            why: None,
            goal: None,
            at: None,
        }
    }

    #[test]
    fn the_ai_keeps_a_fixed_time_that_it_leaves_out() {
        let d = temp("at");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let b = format!("task:{}", s.add("B", "", "you").unwrap().id);
        let at = |key: &str, m| PlanItem {
            at: Some(m),
            ..item(key)
        };
        set(&d, "2026-11-04", "self", vec![at(&a, 900), item(&b)], false).unwrap();
        // The AI sends the plan again with no times: A keeps 15:00.
        let none = HashSet::new();
        let p = set_ai(&d, "2026-11-04", vec![item(&b), item(&a)], &none).unwrap();
        assert_eq!(p.items[1].at, Some(900));
        // Another strict save that is not the AI does not keep the time.
        set(&d, "2026-11-04", "self", vec![at(&a, 900)], false).unwrap();
        let p = set(&d, "2026-11-04", "ai", vec![item(&a)], true).unwrap();
        assert_eq!(p.items[0].at, None);
        // The AI can move it.
        let p = set_ai(&d, "2026-11-04", vec![at(&a, 600)], &none).unwrap();
        assert_eq!(p.items[0].at, Some(600));
        // The window sends the full plan: no time takes the time away.
        let p = set(&d, "2026-11-04", "self", vec![item(&a)], false).unwrap();
        assert_eq!(p.items[0].at, None);
        // The AI can take the fixed time away.
        set(&d, "2026-11-04", "self", vec![at(&a, 900)], false).unwrap();
        let clear: HashSet<String> = [a.clone()].into();
        let p = set_ai(&d, "2026-11-04", vec![item(&a)], &clear).unwrap();
        assert_eq!(p.items[0].at, None);
        // A time past the day.
        assert!(set(&d, "2026-11-04", "ai", vec![at(&a, 1440)], true).is_err());
        let p = set(&d, "2026-11-04", "self", vec![at(&a, 1440)], false).unwrap();
        assert_eq!(p.items[0].at, None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_moves_to_another_day_in_one_change() {
        let d = temp("move");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let b = format!("task:{}", s.add("B", "", "you").unwrap().id);
        let mut why = item(&a);
        why.why = Some("Due soon".into());
        set(&d, "2026-11-04", "ai", vec![why, item(&b)], false).unwrap();
        let (left, to) = move_item(&d, "2026-11-04", "2026-11-05", &a, Some(600), None).unwrap();
        let left = left.unwrap();
        assert_eq!(left.items.len(), 1);
        assert_eq!(left.mode, "ai");
        assert_eq!(to.mode, "self", "a new day is the user's own plan");
        assert_eq!(to.items[0].at, Some(600));
        assert_eq!(to.items[0].why.as_deref(), Some("Due soon"));
        // Back again, at its old place.
        let (_, back) = move_item(&d, "2026-11-05", "2026-11-04", &a, None, Some(0)).unwrap();
        assert_eq!(back.items[0].key, a);
        assert_eq!(back.items[0].at, None);
        assert!(move_item(&d, "2026-11-04", "2026-11-05", "task:nope", None, None).is_err());
        // A task cannot move to a day before its start day.
        let id = b.strip_prefix("task:").unwrap();
        s.update(
            id,
            crate::tasks::TaskFields {
                start: Some("2026-11-05".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let (_, _) = move_item(&d, "2026-11-04", "2026-11-05", &b, None, None).unwrap();
        let e = move_item(&d, "2026-11-05", "2026-11-03", &b, None, None).unwrap_err();
        assert!(e.to_string().contains("starts on 2026-11-05"), "{e}");
        let days: Vec<String> = range(&d, "2026-11-01", "2026-11-07")
            .unwrap()
            .into_iter()
            .map(|p| p.day)
            .collect();
        assert_eq!(days, ["2026-11-04", "2026-11-05"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_closed_item_does_not_move_also_when_the_new_day_has_it() {
        let d = temp("move-closed");
        let s = TaskStore::new(&d);
        let t = s.add("A", "", "you").unwrap();
        let a = format!("task:{}", t.id);
        set(&d, "2026-11-04", "self", vec![item(&a)], false).unwrap();
        set(&d, "2026-11-05", "self", vec![item(&a)], false).unwrap();
        s.set_done(&t.id, true).unwrap();
        let e = move_item(&d, "2026-11-04", "2026-11-05", &a, None, None).unwrap_err();
        assert!(e.to_string().contains("is not open"), "{e}");
        // Nothing changed: the item stays on both days.
        assert_eq!(get(&d, "2026-11-04").unwrap().unwrap().items.len(), 1);
        assert_eq!(get(&d, "2026-11-05").unwrap().unwrap().items.len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_empty_plan_from_a_review_goes_away_after_a_move() {
        let d = temp("move-review");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        set(&d, "2026-11-04", REVIEW, vec![item(&a)], false).unwrap();
        let (left, _) = move_item(&d, "2026-11-04", "2026-11-05", &a, None, None).unwrap();
        assert_eq!(left, None);
        assert_eq!(
            get(&d, "2026-11-04").unwrap(),
            None,
            "the day has no plan again"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_clock_time_is_read() {
        assert_eq!(parse_time("9:30").unwrap(), 570);
        assert_eq!(parse_time("15:00").unwrap(), 900);
        assert!(parse_time("24:00").is_err());
        assert!(parse_time("9.30").is_err());
        assert!(parse_time("9:5").is_err(), "the minutes need two digits");
        assert!(parse_time("+9:30").is_err());
        assert!(parse_time("009:30").is_err());
        assert_eq!(parse_time(" 09:05 ").unwrap(), 545);
        assert_eq!(time_text(570), "09:30");
    }

    #[test]
    fn a_plan_item_keeps_a_goal_that_exists() {
        let d = temp("goal");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let g = crate::goals::add(&d, "Ship 2.0", 1).unwrap();
        let with = |goal| PlanItem {
            goal: Some(goal),
            ..item(&a)
        };
        let p = set(&d, "2026-11-04", "ai", vec![with(g.id)], true).unwrap();
        assert_eq!(p.items[0].goal, Some(g.id));
        let e = set(&d, "2026-11-04", "ai", vec![with(g.id + 1)], true).unwrap_err();
        assert!(format!("{e}").contains("goal ids"), "{e}");
        let p = set(&d, "2026-11-04", "self", vec![with(g.id + 1)], false).unwrap();
        assert_eq!(p.items[0].goal, None, "the window drops a removed goal");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_plan_is_saved_and_read_again() {
        let d = temp("save");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let b = format!("task:{}", s.add("B", "", "you").unwrap().id);
        assert_eq!(get(&d, "2026-11-04").unwrap(), None);
        let p = set(
            &d,
            "2026-11-04",
            "self",
            vec![item(&b), item(&a), item(&b)],
            true,
        )
        .unwrap();
        assert_eq!(
            p.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
            [b.as_str(), a.as_str()],
            "a key twice stays at its first place"
        );
        assert_eq!(get(&d, "2026-11-04").unwrap().unwrap().items, p.items);
        clear(&d, "2026-11-04").unwrap();
        assert_eq!(get(&d, "2026-11-04").unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_closed_item_is_an_error_for_the_ai_and_dropped_for_the_window() {
        let d = temp("closed");
        let s = TaskStore::new(&d);
        let t = s.add("A", "", "you").unwrap();
        let key = format!("task:{}", t.id);
        s.set_done(&t.id, true).unwrap();
        let e = set(&d, "2026-11-04", "ai", vec![item(&key)], true).unwrap_err();
        assert!(format!("{e}").contains(&key), "{e}");
        assert!(
            set(&d, "2026-11-04", "self", vec![item(&key)], false)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            set(&d, "2026-11-04", "other", vec![], false).is_err(),
            "an unknown mode"
        );
        assert!(
            set(&d, "4 Nov", "self", vec![], false).is_err(),
            "a bad day"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_finished_item_keeps_its_place_when_the_plan_changes() {
        let d = temp("finished");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let b = s.add("B", "", "you").unwrap();
        let c = format!("task:{}", s.add("C", "", "you").unwrap().id);
        let bk = format!("task:{}", b.id);
        set(&d, "2026-11-04", "self", vec![item(&a), item(&bk)], false).unwrap();
        s.set_done(&b.id, true).unwrap();
        // The window moves C in; the AI copies the plan back. Both keep B.
        let p = set(
            &d,
            "2026-11-04",
            "self",
            vec![item(&c), item(&a), item(&bk)],
            false,
        )
        .unwrap();
        assert_eq!(
            p.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
            [c.as_str(), a.as_str(), bk.as_str()]
        );
        set(&d, "2026-11-04", "ai", vec![item(&bk), item(&a)], true).unwrap();
        // A done item that was not in the plan of that day is still new.
        assert!(set(&d, "2026-11-05", "ai", vec![item(&bk)], true).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn aifred_does_not_replace_the_users_time_guess() {
        let d = temp("estimate");
        assert!(set_estimate(&d, "github:a#1", Some(30), "aifred").unwrap());
        assert!(set_estimate(&d, "github:a#1", Some(45), "you").unwrap());
        assert!(!set_estimate(&d, "github:a#1", Some(90), "aifred").unwrap());
        assert_eq!(
            estimates(&d).unwrap()["github:a#1"],
            Estimate {
                minutes: 45,
                by: "you".into()
            }
        );
        set_estimate(&d, "github:a#1", Some(9999), "you").unwrap();
        assert_eq!(estimates(&d).unwrap()["github:a#1"].minutes, MAX_MINUTES);
        set_estimate(&d, "github:a#1", None, "you").unwrap();
        assert!(estimates(&d).unwrap().is_empty());
        assert!(set_estimate(&d, "nokey", Some(5), "you").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn today_is_a_date() {
        check_day(&today()).unwrap();
    }

    #[test]
    fn shown_days_are_today_and_the_next_six() {
        let t = "2026-11-09";
        check_shown_day("2026-11-09", t).unwrap();
        check_shown_day("2026-11-15", t).unwrap();
        assert!(check_shown_day("2026-11-08", t).is_err());
        assert!(check_shown_day("2026-11-16", t).is_err());
        assert!(check_shown_day("9 Nov", t).is_err());
    }
}
