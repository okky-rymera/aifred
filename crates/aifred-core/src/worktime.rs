//! Real work times. The user starts work on an item, and stops it, one item
//! at a time. An item can have more than one session. When the item is
//! finished, AIfred keeps the real time (all sessions together) next to the
//! time guess (plan.rs). The brief and the plan chat get the latest real
//! times in the overview (for_ai), so the next time guesses are better.
//!
//! A session longer than MAX_SESSION_SECS is a timer that the user forgot,
//! for example over the night. It does not count, because AIfred cannot
//! know when the work really stopped. The finish tells the user about it
//! (Done::skipped), so no time goes away silently.
//!
//! A local task that is done finishes its work here too (tasks.rs), also
//! when the AI closes it in a chat. A deleted task loses its sessions.

use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db;

/// A session longer than this does not count: the user forgot the timer.
pub const MAX_SESSION_SECS: u64 = 10 * 3600;
/// The overview has the real times of this many finished items, newest first.
pub const RECENT_MAX: usize = 20;

/// The item that the user works on now.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Running {
    pub key: String,
    pub title: String,
    /// When this session started, in Unix seconds.
    pub started: u64,
    /// The seconds of the earlier sessions of the item.
    pub before: u64,
}

/// A finished item: the real time next to the guess.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Done {
    pub key: String,
    pub title: String,
    /// The real time, in minutes. At least 1, or 0 when no session counts
    /// and `skipped` is above 0. A Done with 0 minutes is not saved.
    pub minutes: u32,
    /// The time guess when the item was finished, in minutes.
    pub guess: Option<u32>,
    /// The sessions that do not count: each one was longer than
    /// MAX_SESSION_SECS (a forgotten timer).
    pub skipped: u32,
}

/// The ended sessions of `key`: the seconds that count, and the number of
/// forgotten timers that do not count.
fn ended_secs(c: &Connection, key: &str) -> anyhow::Result<(u64, u32)> {
    let mut q =
        c.prepare("SELECT started, ended FROM work_session WHERE key = ?1 AND ended IS NOT NULL")?;
    let rows = q.query_map([key], |r| Ok((r.get::<_, u64>(0)?, r.get::<_, u64>(1)?)))?;
    let (mut total, mut skipped) = (0, 0);
    for r in rows {
        let (a, b) = r?;
        let s = b.saturating_sub(a);
        if s > MAX_SESSION_SECS {
            skipped += 1;
        } else {
            total += s;
        }
    }
    Ok((total, skipped))
}

/// The key and the start of the session that runs now.
fn running_session(c: &Connection) -> anyhow::Result<Option<(String, u64)>> {
    Ok(c.query_row(
        "SELECT key, started FROM work_session WHERE ended IS NULL",
        [],
        |r| Ok((r.get(0)?, r.get(1)?)),
    )
    .optional()?)
}

fn running_in(c: &Connection) -> anyhow::Result<Option<Running>> {
    let Some((key, started)) = running_session(c)? else {
        return Ok(None);
    };
    let title = crate::items::title_place(c, &key)?
        .map(|(t, _)| t)
        .unwrap_or_else(|| key.clone());
    let before = ended_secs(c, &key)?.0;
    Ok(Some(Running {
        key,
        title,
        started,
        before,
    }))
}

fn end_running(tx: &Transaction, now: u64) -> anyhow::Result<()> {
    // A clock that went back must not make a session end before it started.
    tx.execute(
        "UPDATE work_session SET ended = max(started, ?1) WHERE ended IS NULL",
        [now as i64],
    )?;
    Ok(())
}

/// The item that the user works on now, or None.
pub fn running(dir: &Path) -> anyhow::Result<Option<Running>> {
    db::read(dir, running_in)
}

/// Starts work on `key`. The work on another item stops. Work on the same
/// item goes on with no change.
pub fn start(dir: &Path, key: &str, now: u64) -> anyhow::Result<Running> {
    let key = key.trim();
    db::write(dir, |tx| {
        if crate::items::title_place(tx, key)?.is_none() {
            bail!("AIfred does not know the item {key}");
        }
        if let Some(r) = running_in(tx)?
            && r.key == key
        {
            return Ok(r);
        }
        end_running(tx, now)?;
        tx.execute(
            "INSERT INTO work_session (key, started) VALUES (?1, ?2)",
            rusqlite::params![key, now as i64],
        )?;
        Ok(running_in(tx)?.expect("the session was just added"))
    })
}

/// Stops the work on the item that the user works on now. The item stays open.
pub fn stop(dir: &Path, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| end_running(tx, now))
}

/// Stops the work on `key` when it runs now. AIfred does not keep a real
/// time: the item is not finished. For an item that the user drops or gives
/// away in the weekly review (review.rs).
pub(crate) fn stop_in(tx: &Transaction, key: &str, now: u64) -> anyhow::Result<()> {
    if running_session(tx)?.is_some_and(|(k, _)| k == key) {
        end_running(tx, now)?;
    }
    Ok(())
}

/// The item `key` is finished. Its work stops, and AIfred keeps the real time
/// next to the time guess. None when the item has no work time at all.
pub fn finish(dir: &Path, key: &str, now: u64) -> anyhow::Result<Option<Done>> {
    let key = key.trim();
    crate::items::split(key)?;
    db::write(dir, |tx| finish_in(tx, key, now))
}

/// finish(), in a change that is open.
pub(crate) fn finish_in(tx: &Transaction, key: &str, now: u64) -> anyhow::Result<Option<Done>> {
    if running_session(tx)?.is_some_and(|(k, _)| k == key) {
        end_running(tx, now)?;
    }
    // Unfinished and Cancelled are not finished work (status.rs): the timer
    // stops, and no real time is kept.
    if crate::status::of_in(tx, key)?.is_some_and(|s| {
        matches!(
            s.status.as_str(),
            crate::status::UNFINISHED | crate::status::CANCELLED
        )
    }) {
        return Ok(None);
    }
    let (secs, skipped) = ended_secs(tx, key)?;
    if secs == 0 && skipped == 0 {
        return Ok(None);
    }
    let (title, place) =
        crate::items::title_place(tx, key)?.unwrap_or_else(|| (key.into(), String::new()));
    let guess: Option<(u32, String)> = tx
        .query_row(
            "SELECT minutes, by FROM estimate WHERE key = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?;
    if secs == 0 {
        // Only forgotten timers: there is no real time to keep.
        return Ok(Some(Done {
            key: key.into(),
            title,
            minutes: 0,
            guess: guess.map(|g| g.0),
            skipped,
        }));
    }
    let minutes = (secs as f64 / 60.0).round().max(1.0) as u32;
    tx.execute(
            "INSERT INTO work_done (key, title, place, guess, guess_by, minutes, finished)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT (key) DO UPDATE SET title = ?2, place = ?3, guess = ?4, guess_by = ?5, minutes = ?6, finished = ?7",
            rusqlite::params![
                key,
                title,
                place,
                guess.as_ref().map(|g| g.0),
                guess.as_ref().map(|g| g.1.clone()),
                minutes,
                now as i64
            ],
        )?;
    Ok(Some(Done {
        key: key.into(),
        title,
        minutes,
        guess: guess.map(|g| g.0),
        skipped,
    }))
}

/// The real time that AIfred kept for a finished item: one work_done row.
/// status::Before carries it, so the undo of a status change puts it back.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Kept {
    pub key: String,
    pub title: String,
    pub place: String,
    pub guess: Option<u32>,
    pub guess_by: Option<String>,
    pub minutes: u32,
    pub finished: i64,
}

/// The real time that AIfred keeps for `key` now, or None.
pub(crate) fn kept_in(c: &Connection, key: &str) -> anyhow::Result<Option<Kept>> {
    Ok(c.query_row(
        "SELECT key, title, place, guess, guess_by, minutes, finished FROM work_done WHERE key = ?1",
        [key],
        |r| {
            Ok(Kept {
                key: r.get(0)?,
                title: r.get(1)?,
                place: r.get(2)?,
                guess: r.get(3)?,
                guess_by: r.get(4)?,
                minutes: r.get(5)?,
                finished: r.get(6)?,
            })
        },
    )
    .optional()?)
}

/// Puts a kept real time back, as it was (the undo of a status change).
pub(crate) fn keep_in(tx: &Transaction, k: &Kept) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO work_done (key, title, place, guess, guess_by, minutes, finished)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
         ON CONFLICT (key) DO UPDATE SET title = ?2, place = ?3, guess = ?4, guess_by = ?5, minutes = ?6, finished = ?7",
        rusqlite::params![k.key, k.title, k.place, k.guess, k.guess_by, k.minutes, k.finished],
    )?;
    Ok(())
}

/// The item is open again: its real time does not count until it is
/// finished again. Its sessions stay.
pub fn unfinish(dir: &Path, key: &str) -> anyhow::Result<()> {
    db::write(dir, |tx| unfinish_in(tx, key.trim()))
}

/// unfinish(), in a change that is open.
pub(crate) fn unfinish_in(tx: &Transaction, key: &str) -> anyhow::Result<()> {
    tx.execute("DELETE FROM work_done WHERE key = ?1", [key])?;
    Ok(())
}

/// The item is deleted: its sessions go away, also a session that runs.
/// A real time that AIfred already kept stays.
pub(crate) fn forget_in(tx: &Transaction, key: &str) -> anyhow::Result<()> {
    tx.execute("DELETE FROM work_session WHERE key = ?1", [key])?;
    Ok(())
}

/// The latest real times for the AI, or None when there are none. Each item
/// has its guess, who made the guess, and its real time in minutes.
/// `real_vs_guess` is the middle value (median) of real time / guess, only
/// for AIfred's own guesses: the AI uses it to change its own guesses, and
/// the user's guesses can be wrong in a different way. 1.5 means that the
/// work takes half as long again as AIfred's guesses say.
pub(crate) fn for_ai(c: &Connection) -> anyhow::Result<Option<Value>> {
    let mut q = c.prepare(
        "SELECT title, place, guess, guess_by, minutes FROM work_done ORDER BY finished DESC, key LIMIT ?1",
    )?;
    type Row = (String, String, Option<u32>, Option<String>, u32);
    let rows: Vec<Row> = q
        .query_map([RECENT_MAX as i64], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    if rows.is_empty() {
        return Ok(None);
    }
    let mut ratios: Vec<f64> = rows
        .iter()
        .filter(|(_, _, _, by, _)| by.as_deref() == Some("aifred"))
        .filter_map(|(_, _, g, _, m)| g.filter(|g| *g > 0).map(|g| *m as f64 / g as f64))
        .collect();
    ratios.sort_by(f64::total_cmp);
    let items: Vec<Value> = rows
        .into_iter()
        .map(|(title, place, guess, by, minutes)| {
            let mut v = json!({ "title": title, "real_minutes": minutes });
            if let Some(g) = guess {
                v["guess_minutes"] = g.into();
                if let Some(by) = by {
                    v["guess_by"] = by.into();
                }
            }
            if !place.is_empty() {
                v["place"] = place.into();
            }
            v
        })
        .collect();
    let mut out = json!({ "finished": items });
    if let Some(m) = median(&ratios) {
        out["real_vs_guess"] = ((m * 10.0).round() / 10.0).into();
    }
    Ok(Some(out))
}

fn median(sorted: &[f64]) -> Option<f64> {
    let n = sorted.len();
    match n {
        0 => None,
        _ if n % 2 == 1 => Some(sorted[n / 2]),
        _ => Some((sorted[n / 2 - 1] + sorted[n / 2]) / 2.0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-worktime-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn task(s: &TaskStore, title: &str) -> String {
        format!("task:{}", s.add(title, "", "you").unwrap().id)
    }

    #[test]
    fn one_item_at_a_time_and_all_sessions_count() {
        let d = temp("sessions");
        let s = TaskStore::new(&d);
        let (a, b) = (task(&s, "A"), task(&s, "B"));
        assert_eq!(running(&d).unwrap(), None);
        start(&d, &a, 1000).unwrap();
        // The same item again goes on.
        assert_eq!(start(&d, &a, 1500).unwrap().started, 1000);
        // Another item stops A at 1600.
        let r = start(&d, &b, 1600).unwrap();
        assert_eq!(
            (r.key.as_str(), r.title.as_str(), r.before),
            (b.as_str(), "B", 0)
        );
        stop(&d, 1700).unwrap();
        assert_eq!(running(&d).unwrap(), None);
        let r = start(&d, &a, 2000).unwrap();
        assert_eq!(r.before, 600, "A has 600 seconds from before");
        crate::plan::set_estimate(&d, &a, Some(5), "aifred").unwrap();
        let done = finish(&d, &a, 2600).unwrap().unwrap();
        assert_eq!((done.minutes, done.guess), (20, Some(5)));
        assert_eq!(running(&d).unwrap(), None);
        // B was not running: its finish keeps its time.
        assert_eq!(finish(&d, &b, 9999).unwrap().unwrap().minutes, 2);
        assert!(start(&d, "task:nope", 1).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_task_that_is_done_or_deleted_ends_its_work() {
        let d = temp("taskdone");
        let s = TaskStore::new(&d);
        let (a, b) = (task(&s, "A"), task(&s, "B"));
        let id = |k: &str| k.trim_start_matches("task:").to_string();
        // A task that the AI closes in a chat keeps its earlier sessions.
        start(&d, &a, 0).unwrap();
        stop(&d, 600).unwrap();
        s.set_done(&id(&a), true).unwrap();
        let ai = db::read(&d, for_ai).unwrap().unwrap();
        assert_eq!(ai["finished"][0]["real_minutes"], 10);
        // Open again: the real time does not count.
        s.set_done(&id(&a), false).unwrap();
        assert!(db::read(&d, for_ai).unwrap().is_none());
        // A deleted task stops its timer, with no item left behind.
        start(&d, &b, 1000).unwrap();
        s.remove(&id(&b)).unwrap();
        assert_eq!(running(&d).unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn no_work_time_is_no_record() {
        let d = temp("none");
        let s = TaskStore::new(&d);
        let a = task(&s, "A");
        assert_eq!(finish(&d, &a, 10).unwrap(), None);
        // A forgotten timer does not count, and the finish says so.
        start(&d, &a, 0).unwrap();
        let done = finish(&d, &a, MAX_SESSION_SECS + 1).unwrap().unwrap();
        assert_eq!((done.minutes, done.skipped), (0, 1));
        assert!(
            db::read(&d, for_ai).unwrap().is_none(),
            "0 minutes is not saved"
        );
        // Real work after it counts, and the forgotten timer is still reported.
        start(&d, &a, 2 * MAX_SESSION_SECS).unwrap();
        let done = finish(&d, &a, 2 * MAX_SESSION_SECS + 600).unwrap().unwrap();
        assert_eq!((done.minutes, done.skipped), (10, 1));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_that_opens_again_does_not_count() {
        let d = temp("again");
        let s = TaskStore::new(&d);
        let a = task(&s, "A");
        start(&d, &a, 0).unwrap();
        finish(&d, &a, 600).unwrap().unwrap();
        unfinish(&d, &a).unwrap();
        assert!(db::read(&d, for_ai).unwrap().is_none());
        // More work, then finished again: both sessions count.
        start(&d, &a, 1000).unwrap();
        assert_eq!(finish(&d, &a, 1600).unwrap().unwrap().minutes, 20);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_ai_gets_the_real_times_and_the_middle_ratio() {
        let d = temp("ai");
        let s = TaskStore::new(&d);
        let keys: Vec<String> = ["A", "B", "C", "D"].iter().map(|t| task(&s, t)).collect();
        // AIfred's guesses. A: guess 10, real 20 (2.0). B: guess 30, real 30 (1.0).
        // C: no guess, real 5. D: the user's guess 5, real 30 (6.0): it does not
        // change AIfred's ratio.
        for (i, (k, guess, by, secs)) in [
            (0, Some(10), "aifred", 1200),
            (1, Some(30), "aifred", 1800),
            (2, None, "", 300),
            (3, Some(5), "you", 1800),
        ]
        .into_iter()
        .map(|(i, g, by, s)| (i, (&keys[i], g, by, s)))
        {
            if guess.is_some() {
                crate::plan::set_estimate(&d, k, guess, by).unwrap();
            }
            let t0 = 10_000 * (i as u64 + 1);
            start(&d, k, t0).unwrap();
            finish(&d, k, t0 + secs).unwrap();
        }
        let v = db::read(&d, for_ai).unwrap().unwrap();
        assert_eq!(v["real_vs_guess"], 1.5);
        let f = v["finished"].as_array().unwrap();
        assert_eq!(f[0]["title"], "D", "newest first");
        assert_eq!(f[0]["guess_by"], "you");
        assert!(f[1].get("guess_minutes").is_none());
        assert_eq!(
            (
                f[3]["guess_minutes"].as_u64(),
                f[3]["real_minutes"].as_u64(),
                f[3]["guess_by"].as_str()
            ),
            (Some(10), Some(20), Some("aifred"))
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
