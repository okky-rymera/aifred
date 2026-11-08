//! What AIfred learns from the user's corrections of the brief.
//!
//! The user can tell AIfred that the brief is wrong about an item:
//! - "not today" (NOT_TODAY): the item does not need the user today.
//! - "wrong priority" (WRONG_PRIORITY): the brief ranked the item too high.
//!
//! The brief of that day stops showing a reason for the item at once
//! (brief::get). The next briefs and the plan chat get the recent
//! corrections in the overview (for_ai), so they rank the item lower.
//!
//! When the user says "wrong priority" for LEARN_AFTER different items from
//! one place (a GitHub repository, a ClickUp list, the first person of an
//! email, or the first tag of a local task) in LEARN_DAYS days, AIfred saves a
//! memory fact about that place. The fact is core, so the brief and every
//! chat get it. The user can change or forget the fact in memory. AIfred
//! learns each place only once: a forgotten fact does not come back.

use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::Serialize;
use serde_json::{Value, json};

use crate::db;
use crate::memory::Fact;

pub const NOT_TODAY: &str = "not_today";
pub const WRONG_PRIORITY: &str = "wrong_priority";

/// This many "wrong priority" corrections of different items from one place
/// make a memory fact.
pub const LEARN_AFTER: usize = 3;
/// Only the corrections of this many days count for a new fact.
pub const LEARN_DAYS: u64 = 60;
/// The overview has the corrections of this many days...
pub const RECENT_DAYS: u64 = 30;
/// ...but at most this many, newest first.
pub const RECENT_MAX: usize = 30;

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Correction {
    pub id: i64,
    pub key: String,
    /// NOT_TODAY or WRONG_PRIORITY.
    pub kind: String,
    /// The day of the brief, "YYYY-MM-DD".
    pub day: String,
    pub title: String,
    /// Where the item is from (items::title_place), or "".
    pub place: String,
    pub ts: u64,
}

/// A saved correction, and the memory fact that it made, if any.
#[derive(Debug, Clone, Serialize)]
pub struct Corrected {
    pub correction: Correction,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub learned: Option<Fact>,
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Correction> {
    Ok(Correction {
        id: r.get(0)?,
        key: r.get(1)?,
        kind: r.get(2)?,
        day: r.get(3)?,
        title: r.get(4)?,
        place: r.get(5)?,
        ts: r.get(6)?,
    })
}

const COLUMNS: &str = "id, key, kind, day, title, place, ts";

/// Saves a correction of the brief of `day`. The same correction of the same
/// item on the same day is saved once: a second click gives the first one.
pub fn add(dir: &Path, key: &str, kind: &str, day: &str, now: u64) -> anyhow::Result<Corrected> {
    db::write(dir, |tx| add_in(tx, key, kind, day, now))
}

/// add(), in a write that is open.
pub(crate) fn add_in(
    tx: &Transaction,
    key: &str,
    kind: &str,
    day: &str,
    now: u64,
) -> anyhow::Result<Corrected> {
    if kind != NOT_TODAY && kind != WRONG_PRIORITY {
        bail!("a correction is \"{NOT_TODAY}\" or \"{WRONG_PRIORITY}\", not \"{kind}\"");
    }
    crate::plan::check_day(day)?;
    let key = key.trim();
    let Some((title, place)) = crate::items::title_place(tx, key)? else {
        bail!("AIfred does not know the item {key}");
    };
    let old = tx
        .query_row(
            &format!("SELECT {COLUMNS} FROM correction WHERE key = ?1 AND kind = ?2 AND day = ?3"),
            [key, kind, day],
            row,
        )
        .optional()?;
    if let Some(correction) = old {
        return Ok(Corrected {
            correction,
            learned: None,
        });
    }
    tx.execute(
        "INSERT INTO correction (key, kind, day, title, place, ts) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
        rusqlite::params![key, kind, day, title, place, now as i64],
    )?;
    let correction = Correction {
        id: tx.last_insert_rowid(),
        key: key.into(),
        kind: kind.into(),
        day: day.into(),
        title,
        place,
        ts: now,
    };
    let learned = learn(tx, &correction, now)?;
    Ok(Corrected {
        correction,
        learned,
    })
}

/// Saves a memory fact when the user said "wrong priority" for enough
/// different items of the correction's place. None when there is nothing
/// to learn, or when AIfred already learned the place.
fn learn(tx: &Transaction, c: &Correction, now: u64) -> anyhow::Result<Option<Fact>> {
    if c.kind != WRONG_PRIORITY || c.place.is_empty() {
        return Ok(None);
    }
    let known: Option<i64> = tx
        .query_row("SELECT 1 FROM learned WHERE place = ?1", [&c.place], |r| {
            r.get(0)
        })
        .optional()?;
    if known.is_some() {
        return Ok(None);
    }
    let items: i64 = tx.query_row(
        "SELECT count(DISTINCT key) FROM correction WHERE place = ?1 AND kind = ?2 AND ts >= ?3",
        rusqlite::params![
            c.place,
            WRONG_PRIORITY,
            now.saturating_sub(LEARN_DAYS * 86_400) as i64
        ],
        |r| r.get(0),
    )?;
    if (items as usize) < LEARN_AFTER {
        return Ok(None);
    }
    let text = fact_text(&c.place, items);
    let fact = crate::memory::add_in(tx, &text, "aifred", true)?;
    tx.execute(
        "INSERT INTO learned (place, fact, correction, ts) VALUES (?1, ?2, ?3, ?4)",
        rusqlite::params![c.place, fact.id, c.id, now as i64],
    )?;
    Ok(Some(fact))
}

/// The text of a learned fact: the place, FACT_HEAD, the number of
/// corrections, FACT_TAIL.
const FACT_HEAD: &str = ": these items are less important to the user than they look. Rank them lower. (AIfred learned this from ";
const FACT_TAIL: &str = " \"wrong priority\" corrections in the brief.)";

/// The text of the memory fact that AIfred learns about `place` from
/// `items` corrections.
fn fact_text(place: &str, items: i64) -> String {
    format!("{place}{FACT_HEAD}{items}{FACT_TAIL}")
}

/// True when `text` is a text that fact_text() makes for `place`: the user
/// did not change the fact.
fn is_fact_text(place: &str, text: &str) -> bool {
    text.strip_prefix(place)
        .and_then(|r| r.strip_prefix(FACT_HEAD))
        .and_then(|r| r.strip_suffix(FACT_TAIL))
        .is_some_and(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

/// Removes a correction: the click was a mistake. When the place of the
/// correction has a learned memory fact, and the fact now has fewer than
/// LEARN_AFTER corrections, the fact is forgotten too, and AIfred can learn
/// the place again later. A fact that the user changed or forgot stays as
/// the user left it: it is the user's fact now.
pub fn undo(dir: &Path, id: i64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        let Some((kind, place)): Option<(String, String)> = tx
            .query_row(
                "SELECT kind, place FROM correction WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?
        else {
            bail!("no correction with id {id}");
        };
        tx.execute("DELETE FROM correction WHERE id = ?1", [id])?;
        if kind != WRONG_PRIORITY || place.is_empty() {
            return Ok(());
        }
        let learned: Option<(i64, u64)> = tx
            .query_row(
                "SELECT fact, ts FROM learned WHERE place = ?1",
                [&place],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((fact, learned_ts)) = learned else {
            return Ok(());
        };
        // The corrections that counted when AIfred learned the fact.
        let items: i64 = tx.query_row(
            "SELECT count(DISTINCT key) FROM correction WHERE place = ?1 AND kind = ?2 AND ts >= ?3",
            rusqlite::params![
                place,
                WRONG_PRIORITY,
                learned_ts.saturating_sub(LEARN_DAYS * 86_400) as i64
            ],
            |r| r.get(0),
        )?;
        if items as usize >= LEARN_AFTER {
            return Ok(());
        }
        // The text tells if the user changed the fact. The times cannot:
        // they have one-second steps, and a change of "core" sets them too.
        let row: Option<(String, bool)> = tx
            .query_row(
                "SELECT text, forgotten IS NULL FROM memory WHERE id = ?1",
                [fact],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let untouched = row.is_some_and(|(text, live)| live && is_fact_text(&place, &text));
        if untouched {
            crate::memory::forget_in(tx, fact)?;
            tx.execute("DELETE FROM learned WHERE place = ?1", [&place])?;
        }
        Ok(())
    })
}

/// The keys of the items that the user corrected in the brief of `day`.
/// The brief of that day shows no reason for them.
pub(crate) fn corrected_on(c: &Connection, day: &str) -> anyhow::Result<HashSet<String>> {
    let mut q = c.prepare("SELECT key FROM correction WHERE day = ?1")?;
    let rows = q.query_map([day], |r| r.get::<_, String>(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The recent corrections for the AI, newest first.
pub(crate) fn for_ai(c: &Connection, now: u64) -> anyhow::Result<Vec<Value>> {
    let mut q = c.prepare(&format!(
        "SELECT {COLUMNS} FROM correction WHERE ts >= ?1 ORDER BY ts DESC, id DESC LIMIT ?2"
    ))?;
    let rows = q.query_map(
        rusqlite::params![
            now.saturating_sub(RECENT_DAYS * 86_400) as i64,
            RECENT_MAX as i64
        ],
        row,
    )?;
    rows.map(|r| {
        let r = r?;
        let mut v = json!({ "key": r.key, "title": r.title, "kind": r.kind, "day": r.day });
        if !r.place.is_empty() {
            v["place"] = r.place.into();
        }
        Ok(v)
    })
    .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::memory::MemoryStore;
    use crate::tasks::{TaskFields, TaskStore};

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-learn-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A local task with the tag `tag`, as an item key.
    fn task(s: &TaskStore, title: &str, tag: &str) -> String {
        let t = s
            .create(
                TaskFields {
                    title: Some(title.into()),
                    tags: Some(vec![tag.into()]),
                    ..Default::default()
                },
                "you",
            )
            .unwrap();
        format!("task:{}", t.id)
    }

    #[test]
    fn a_correction_is_saved_once_and_can_be_undone() {
        let d = temp("once");
        let s = TaskStore::new(&d);
        let k = task(&s, "Fix docs", "docs");
        let a = add(&d, &k, NOT_TODAY, "2026-11-06", 100).unwrap();
        assert_eq!(a.correction.title, "Fix docs");
        assert_eq!(a.correction.place, "Your tasks with #docs");
        let b = add(&d, &k, NOT_TODAY, "2026-11-06", 200).unwrap();
        assert_eq!(
            a.correction, b.correction,
            "a second click gives the first one"
        );
        let on = db::read(&d, |c| corrected_on(c, "2026-11-06")).unwrap();
        assert!(on.contains(&k));
        undo(&d, a.correction.id).unwrap();
        assert!(
            db::read(&d, |c| corrected_on(c, "2026-11-06"))
                .unwrap()
                .is_empty()
        );
        assert!(undo(&d, a.correction.id).is_err());
        assert!(
            add(&d, &k, "later", "2026-11-06", 1).is_err(),
            "an unknown kind"
        );
        assert!(
            add(&d, "task:nope", NOT_TODAY, "2026-11-06", 1).is_err(),
            "an unknown item"
        );
        assert!(add(&d, &k, NOT_TODAY, "6 Nov", 1).is_err(), "a bad day");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn three_items_from_one_place_make_one_fact() {
        let d = temp("learn");
        let s = TaskStore::new(&d);
        let day = 86_400;
        let a = task(&s, "Fix typo", "docs");
        let b = task(&s, "Update screenshots", "docs");
        let c = task(&s, "New FAQ", "docs");
        let other = task(&s, "Pay rent", "home");
        // The same item two times counts once. "Not today" does not count.
        assert!(
            add(&d, &a, WRONG_PRIORITY, "2026-11-01", day)
                .unwrap()
                .learned
                .is_none()
        );
        assert!(
            add(&d, &a, WRONG_PRIORITY, "2026-11-02", 2 * day)
                .unwrap()
                .learned
                .is_none()
        );
        assert!(
            add(&d, &b, NOT_TODAY, "2026-11-02", 2 * day)
                .unwrap()
                .learned
                .is_none()
        );
        assert!(
            add(&d, &other, WRONG_PRIORITY, "2026-11-02", 2 * day)
                .unwrap()
                .learned
                .is_none()
        );
        assert!(
            add(&d, &b, WRONG_PRIORITY, "2026-11-03", 3 * day)
                .unwrap()
                .learned
                .is_none()
        );
        let third = add(&d, &c, WRONG_PRIORITY, "2026-11-04", 4 * day).unwrap();
        let fact = third.learned.expect("the third item makes a fact");
        assert!(
            fact.text.starts_with("Your tasks with #docs: these items"),
            "{}",
            fact.text
        );
        assert!(fact.core, "the brief and every chat get it");
        assert_eq!(fact.added_by, "aifred");

        // Undo of the correction that made the fact forgets the fact.
        undo(&d, third.correction.id).unwrap();
        let m = MemoryStore::new(&d);
        assert!(m.core_text().unwrap().is_empty());
        // The same click again learns the place again.
        let again = add(&d, &c, WRONG_PRIORITY, "2026-11-04", 4 * day).unwrap();
        let fact = again.learned.expect("learned again after the undo");
        // A fact that the user forgets does not come back.
        m.forget(fact.id).unwrap();
        let d4 = task(&s, "Fix links", "docs");
        assert!(
            add(&d, &d4, WRONG_PRIORITY, "2026-11-05", 5 * day)
                .unwrap()
                .learned
                .is_none()
        );
        assert!(m.core_text().unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn undo_of_any_counted_correction_forgets_an_untouched_fact() {
        let d = temp("undoany");
        let s = TaskStore::new(&d);
        let m = MemoryStore::new(&d);
        let keys: Vec<String> = (0..4)
            .map(|i| task(&s, &format!("Docs {i}"), "docs"))
            .collect();
        let first = add(&d, &keys[0], WRONG_PRIORITY, "2026-11-01", 10).unwrap();
        add(&d, &keys[1], WRONG_PRIORITY, "2026-11-01", 20).unwrap();
        add(&d, &keys[2], WRONG_PRIORITY, "2026-11-01", 30)
            .unwrap()
            .learned
            .expect("learned");
        // A fourth item: an undo of the first one leaves 3, so the fact stays.
        let fourth = add(&d, &keys[3], WRONG_PRIORITY, "2026-11-01", 40).unwrap();
        undo(&d, first.correction.id).unwrap();
        assert!(!m.core_text().unwrap().is_empty(), "3 corrections are left");
        // One more undo leaves 2: the fact goes, also if it was not the last click.
        undo(&d, fourth.correction.id).unwrap();
        assert!(
            m.core_text().unwrap().is_empty(),
            "only 2 corrections are left"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn undo_keeps_a_fact_that_the_user_changed() {
        let d = temp("undoedit");
        let s = TaskStore::new(&d);
        let m = MemoryStore::new(&d);
        let keys: Vec<String> = (0..3)
            .map(|i| task(&s, &format!("Docs {i}"), "docs"))
            .collect();
        add(&d, &keys[0], WRONG_PRIORITY, "2026-11-01", 10).unwrap();
        add(&d, &keys[1], WRONG_PRIORITY, "2026-11-01", 20).unwrap();
        let third = add(&d, &keys[2], WRONG_PRIORITY, "2026-11-01", 30).unwrap();
        let fact = third.learned.expect("learned");
        // The user changes the fact in the same second that AIfred made it.
        m.edit(fact.id, "Docs tasks can wait until Friday.")
            .unwrap();
        undo(&d, third.correction.id).unwrap();
        assert!(
            m.core_text()
                .unwrap()
                .contains("Docs tasks can wait until Friday.")
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn undo_forgets_a_fact_when_the_user_only_changed_core() {
        let d = temp("undocore");
        let s = TaskStore::new(&d);
        let m = MemoryStore::new(&d);
        let keys: Vec<String> = (0..3)
            .map(|i| task(&s, &format!("Docs {i}"), "docs"))
            .collect();
        add(&d, &keys[0], WRONG_PRIORITY, "2026-11-01", 10).unwrap();
        add(&d, &keys[1], WRONG_PRIORITY, "2026-11-01", 20).unwrap();
        let third = add(&d, &keys[2], WRONG_PRIORITY, "2026-11-01", 30).unwrap();
        let fact = third.learned.expect("learned");
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE memory SET updated = updated + 60 WHERE id = ?1",
                [fact.id],
            )?;
            Ok(())
        })
        .unwrap();
        m.set_core(fact.id, false).unwrap();
        undo(&d, third.correction.id).unwrap();
        assert!(m.all_text().unwrap().is_empty(), "the text did not change");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_place_with_digits_still_matches_its_fact_text() {
        let text = fact_text("GitHub acme/v0", 3);
        assert!(is_fact_text("GitHub acme/v0", &text));
        assert!(!is_fact_text("GitHub acme/v1", &text));
        assert!(!is_fact_text(
            "GitHub acme/v0",
            &format!("{text} Also on Fridays.")
        ));
    }

    #[test]
    fn old_corrections_do_not_count() {
        let d = temp("old");
        let s = TaskStore::new(&d);
        let day = 86_400;
        let keys: Vec<String> = (0..3)
            .map(|i| task(&s, &format!("Docs {i}"), "docs"))
            .collect();
        add(&d, &keys[0], WRONG_PRIORITY, "2026-01-01", day).unwrap();
        add(&d, &keys[1], WRONG_PRIORITY, "2026-03-01", 70 * day).unwrap();
        let last = add(&d, &keys[2], WRONG_PRIORITY, "2026-03-02", 71 * day).unwrap();
        assert!(
            last.learned.is_none(),
            "the first one is older than {LEARN_DAYS} days"
        );
        let ai = db::read(&d, |c| for_ai(c, 71 * day)).unwrap();
        assert_eq!(ai.len(), 2, "only the last {RECENT_DAYS} days");
        assert_eq!(ai[0]["key"], keys[2].as_str(), "newest first");
        assert_eq!(ai[0]["kind"], WRONG_PRIORITY);
        assert_eq!(ai[0]["place"], "Your tasks with #docs");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_task_with_no_tag_has_no_place_and_teaches_nothing() {
        let d = temp("noplace");
        let s = TaskStore::new(&d);
        for i in 0..4 {
            let k = format!("task:{}", s.add(&format!("T{i}"), "", "you").unwrap().id);
            let r = add(&d, &k, WRONG_PRIORITY, "2026-11-06", 10).unwrap();
            assert!(r.learned.is_none());
        }
        let ai = db::read(&d, |c| for_ai(c, 10)).unwrap();
        assert!(ai[0].get("place").is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
