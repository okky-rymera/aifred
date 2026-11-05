//! The user's current goals: 3 to 5 short lines, for example "Ship StoreAgent
//! 2.0 this month". Dates make an item urgent. Goals make it important: the
//! brief and the day plan rank items by the goal that each item serves.
//!
//! Goals get old. When the user has not changed or checked the goals for
//! CHECK_AFTER_DAYS days, the app asks the user to check them.

use std::path::Path;

use anyhow::{bail, ensure};
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::Serialize;

use crate::db;

/// The user keeps at most this many goals. More goals do not help to rank.
pub const MAX: usize = 5;
/// The longest goal, in characters.
pub const MAX_CHARS: usize = 200;
/// After this many days with no change and no check, the app asks the user to check the goals.
pub const CHECK_AFTER_DAYS: u64 = 30;

/// The meta key of the last time the user changed or checked the goals.
const CHECKED: &str = "goals_checked";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Goal {
    pub id: i64,
    pub text: String,
    pub created: u64,
    pub updated: u64,
}

/// The goals and their age.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Goals {
    pub goals: Vec<Goal>,
    /// The last time the user changed or checked the goals, in Unix seconds. 0: never.
    pub checked: u64,
    /// True when there are goals and the user has not checked them for CHECK_AFTER_DAYS days.
    pub stale: bool,
}

fn clean(text: &str) -> anyhow::Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    ensure!(!text.is_empty(), "the goal is empty");
    ensure!(
        text.chars().count() <= MAX_CHARS,
        "the goal is longer than {MAX_CHARS} characters. Write it in one short line"
    );
    Ok(text)
}

pub(crate) fn list_in(c: &Connection) -> anyhow::Result<Vec<Goal>> {
    let mut q = c.prepare("SELECT id, text, created, updated FROM goal ORDER BY id")?;
    let rows = q.query_map([], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<Goal> {
    Ok(Goal {
        id: r.get(0)?,
        text: r.get(1)?,
        created: r.get(2)?,
        updated: r.get(3)?,
    })
}

fn get(c: &Connection, id: i64) -> anyhow::Result<Goal> {
    match c
        .query_row(
            "SELECT id, text, created, updated FROM goal WHERE id = ?1",
            [id],
            row,
        )
        .optional()?
    {
        Some(g) => Ok(g),
        None => bail!("no goal with id {id}"),
    }
}

fn mark_checked(tx: &Transaction, now: u64) -> anyhow::Result<()> {
    db::set_meta(tx, CHECKED, &now.to_string())
}

pub fn list(dir: &Path) -> anyhow::Result<Vec<Goal>> {
    db::read(dir, list_in)
}

/// The goals, and whether the app must ask the user to check them.
pub fn status(dir: &Path, now: u64) -> anyhow::Result<Goals> {
    db::read(dir, |c| {
        let goals = list_in(c)?;
        let checked = db::meta(c, CHECKED)?
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(0);
        let stale = !goals.is_empty() && now.saturating_sub(checked) > CHECK_AFTER_DAYS * 86_400;
        Ok(Goals {
            goals,
            checked,
            stale,
        })
    })
}

/// Adds a goal at the end of the list. A change counts as a check.
pub fn add(dir: &Path, text: &str, now: u64) -> anyhow::Result<Goal> {
    let text = clean(text)?;
    db::write(dir, |tx| {
        let n: i64 = tx.query_row("SELECT count(*) FROM goal", [], |r| r.get(0))?;
        if n as usize >= MAX {
            bail!(
                "the user has {MAX} goals, the most that AIfred keeps. Change or remove a goal first"
            );
        }
        tx.execute(
            "INSERT INTO goal (text, created, updated) VALUES (?1, ?2, ?2)",
            rusqlite::params![text, now as i64],
        )?;
        mark_checked(tx, now)?;
        get(tx, tx.last_insert_rowid())
    })
}

pub fn edit(dir: &Path, id: i64, text: &str, now: u64) -> anyhow::Result<Goal> {
    let text = clean(text)?;
    db::write(dir, |tx| {
        let n = tx.execute(
            "UPDATE goal SET text = ?2, updated = ?3 WHERE id = ?1",
            rusqlite::params![id, text, now as i64],
        )?;
        if n == 0 {
            bail!("no goal with id {id}");
        }
        mark_checked(tx, now)?;
        get(tx, id)
    })
}

/// Removes a goal. The goal is deleted: an old goal does not help to rank.
pub fn remove(dir: &Path, id: i64, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        if tx.execute("DELETE FROM goal WHERE id = ?1", [id])? == 0 {
            bail!("no goal with id {id}");
        }
        mark_checked(tx, now)
    })
}

/// The user says that the goals are still right.
pub fn confirm(dir: &Path, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| mark_checked(tx, now))
}

/// True when a goal with this id exists.
pub(crate) fn exists(c: &Connection, id: i64) -> anyhow::Result<bool> {
    Ok(
        c.query_row("SELECT 1 FROM goal WHERE id = ?1", [id], |_| Ok(()))
            .optional()?
            .is_some(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-goals-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn goals_are_added_changed_and_removed() {
        let d = temp("crud");
        let a = add(&d, "  Ship StoreAgent\n2.0  ", 100).unwrap();
        assert_eq!(a.text, "Ship StoreAgent 2.0", "one line, no extra spaces");
        let b = add(&d, "Hire a designer", 100).unwrap();
        assert_eq!(
            list(&d).unwrap().iter().map(|g| g.id).collect::<Vec<_>>(),
            [a.id, b.id]
        );
        let a2 = edit(&d, a.id, "Ship StoreAgent 2.0 by 30 Nov", 200).unwrap();
        assert_eq!((a2.created, a2.updated), (100, 200));
        remove(&d, b.id, 300).unwrap();
        assert_eq!(list(&d).unwrap(), vec![a2]);
        assert!(edit(&d, b.id, "x", 400).is_err());
        assert!(remove(&d, b.id, 400).is_err());
        assert!(add(&d, "   ", 400).is_err());
        assert!(add(&d, &"x".repeat(MAX_CHARS + 1), 400).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_goal_never_gets_the_id_of_a_removed_goal() {
        let d = temp("reuse");
        add(&d, "Ship StoreAgent 2.0", 1).unwrap();
        let last = add(&d, "Write the docs", 1).unwrap();
        remove(&d, last.id, 2).unwrap();
        let new = add(&d, "Hire a designer", 3).unwrap();
        assert!(new.id > last.id, "the brief can still point at {}", last.id);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn at_most_five_goals() {
        let d = temp("max");
        for i in 0..MAX {
            add(&d, &format!("Goal {i}"), 1).unwrap();
        }
        let e = add(&d, "One more", 1).unwrap_err().to_string();
        assert!(e.contains("Change or remove"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn old_goals_ask_for_a_check() {
        let d = temp("stale");
        let day = 86_400;
        assert!(
            !status(&d, 1000 * day).unwrap().stale,
            "no goals: nothing to check"
        );
        add(&d, "Ship 2.0", 0).unwrap();
        assert!(!status(&d, 30 * day).unwrap().stale);
        assert!(status(&d, 31 * day).unwrap().stale);
        confirm(&d, 31 * day).unwrap();
        let s = status(&d, 32 * day).unwrap();
        assert_eq!((s.stale, s.checked), (false, 31 * day));
        assert!(status(&d, 62 * day).unwrap().stale);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
