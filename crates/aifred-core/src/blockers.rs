//! Blockers: an item that must wait until other items are finished, in
//! AIfred only. For example, "Deploy the fix" waits for "Review PR #12".
//! ClickUp and GitHub do not change.
//!
//! - Each row of `item_blocker` says that `key` waits for `blocker`. An item
//!   can wait for many items, and many items can wait for one item.
//! - "Blocked" is not stored. An item is blocked while one of its blockers
//!   is open. When the blocker is finished, the row stays, so a reopen or
//!   an Undo works again.
//! - `placed_by` says who set the link: "you" (in the app), "chat" (the AI
//!   says that the user asked in a chat), or "aifred" (the AI by itself).
//!   `why` is AIfred's reason. When a link is taken away, the row stays
//!   with `removed` 1, and `placed_by` says who took it away: "you" or
//!   "aifred". When the user took it away in the app, the AI never adds
//!   that link again, also not when it says that the user asked in the
//!   chat. Only the app can add it again. When the AI took it away, the AI
//!   can add it again.
//! - Only a link to an open blocker counts for the rules: the limit of
//!   [`MAX_BLOCKERS`] and the check for items that wait for each other. A
//!   link to a finished item, or to an item that is not in the list now,
//!   is asleep. So old links do not stop a new link. When a finished item
//!   opens again, its old links count again. Then two open items can wait
//!   for each other, and an item can have more than [`MAX_BLOCKERS`]
//!   blockers. The user sees both links in the panel and can take one away.
//! - An item cannot wait for itself.
//! - When an item is gone (deleted, or its source is disconnected), its
//!   rows go away (tasks.rs remove, links.rs drop_gone_in).
//! - A blocked item can still be finished. The link only informs.

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::items::{known_sql, split};

/// An item can wait for at most this many open items.
pub const MAX_BLOCKERS: usize = 10;
/// The longest reason of AIfred that is kept, in characters.
const MAX_WHY: usize = 120;

/// One item that waits for another item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocker {
    /// The item key of the item that waits.
    pub key: String,
    /// The item key of the item that must finish first.
    pub blocker: String,
    /// Who set it: "you", "chat" or "aifred".
    pub by: String,
    /// AIfred's reason. Empty when the user set it.
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Blockers {
    pub blockers: Vec<Blocker>,
}

/// One row as it was before a change, for its Undo.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub by: String,
    pub why: String,
    pub removed: bool,
}

/// How the link of `key` and `blocker` was before a change. `row` None:
/// there was no row. `at` and `gone` say how the change left the row: its
/// time, and if it was taken away. The Undo works only while the row is
/// still like that. So an old Undo does not undo a later change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Before {
    pub key: String,
    pub blocker: String,
    pub row: Option<Row>,
    #[serde(default)]
    pub at: u64,
    #[serde(default)]
    pub gone: bool,
}

/// The blockers after a change, and how things were before it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Changed {
    #[serde(flatten)]
    pub blockers: Blockers,
    pub before: Before,
}

/// All links that count: not the ones that were taken away.
pub fn get(dir: &Path) -> anyhow::Result<Blockers> {
    db::read(dir, get_in)
}

pub fn get_in(c: &Connection) -> anyhow::Result<Blockers> {
    let mut q = c.prepare(
        "SELECT key, blocker, placed_by, why FROM item_blocker WHERE removed = 0 ORDER BY key, blocker",
    )?;
    let rows = q.query_map([], |r| {
        Ok(Blocker {
            key: r.get(0)?,
            blocker: r.get(1)?,
            by: r.get(2)?,
            why: r.get(3)?,
        })
    })?;
    Ok(Blockers {
        blockers: rows.collect::<Result<_, _>>()?,
    })
}

fn row_in(c: &Connection, key: &str, blocker: &str) -> anyhow::Result<Option<(Row, u64)>> {
    Ok(c.query_row(
        "SELECT placed_by, why, removed, ts FROM item_blocker WHERE key = ?1 AND blocker = ?2",
        [key, blocker],
        |r| {
            Ok((
                Row {
                    by: r.get(0)?,
                    why: r.get(1)?,
                    removed: r.get::<_, i64>(2)? != 0,
                },
                r.get::<_, i64>(3)? as u64,
            ))
        },
    )
    .optional()?)
}

/// True when `from` waits for `to`, directly or through other items. Only
/// links to open items count: a link to a finished item is asleep.
fn waits_for_in(c: &Connection, from: &str, to: &str) -> anyhow::Result<bool> {
    let mut q = c.prepare_cached(&format!(
        "SELECT blocker FROM item_blocker WHERE key = ?1 AND removed = 0 AND {}",
        known_sql("blocker", true)
    ))?;
    let mut seen = std::collections::HashSet::new();
    let mut todo = vec![from.to_string()];
    while let Some(k) = todo.pop() {
        if k == to {
            return Ok(true);
        }
        if !seen.insert(k.clone()) {
            continue;
        }
        let next: Vec<String> = q.query_map([&k], |r| r.get(0))?.collect::<Result<_, _>>()?;
        todo.extend(next);
    }
    Ok(false)
}

/// Says that `key` waits for `blocker`. `by` is who asks:
///
/// - "you": the user, in the app.
/// - "chat": the AI says that the user asked in the chat. The link is saved
///   as "chat", so the user can see where it came from. The AI can be
///   wrong or misled, so it cannot add a link that the user took away in
///   the app.
/// - "aifred": the AI by itself. `why` is its reason. It is kept only here.
///
/// When the user set the link already ("you" or "chat"), the AI by itself
/// does not change it.
pub fn add(
    dir: &Path,
    key: &str,
    blocker: &str,
    by: &str,
    why: &str,
    now: u64,
) -> anyhow::Result<Changed> {
    let (key, blocker) = (key.trim(), blocker.trim());
    split(key)?;
    split(blocker)?;
    if !matches!(by, "you" | "chat" | "aifred") {
        bail!("who set the link must be \"you\", \"chat\" or \"aifred\", not \"{by}\"");
    }
    if key == blocker {
        bail!("an item cannot wait for itself");
    }
    db::write(dir, |tx| {
        for k in [key, blocker] {
            if crate::items::title_place(tx, k)?.is_none() {
                bail!("AIfred does not know the item {k}");
            }
        }
        let old = row_in(tx, key, blocker)?;
        let mut before = Before {
            key: key.into(),
            blocker: blocker.into(),
            row: old.as_ref().map(|(r, _)| r.clone()),
            at: now,
            gone: false,
        };
        let user_took_away = old
            .as_ref()
            .is_some_and(|(r, _)| r.removed && r.by == "you");
        if by != "you" && user_took_away {
            bail!(
                "the user took away the link \"{key} waits for {blocker}\" in the app. Do not add it again. Only the user can add it again, in the item panel"
            );
        }
        if let Some((r, ts)) = &old
            && !r.removed
            && ((by == "aifred" && r.by != "aifred") || (by == "chat" && r.by == "you"))
        {
            // No change: the AI does not relabel a link of the user. The
            // row stays as it is.
            before.at = *ts;
            return Ok(Changed {
                blockers: get_in(tx)?,
                before,
            });
        }
        if waits_for_in(tx, blocker, key)? {
            bail!("{blocker} already waits for {key}. Two items cannot wait for each other");
        }
        let active = old.as_ref().is_some_and(|(r, _)| !r.removed);
        if !active {
            check_room_in(tx, key, blocker)?;
        }
        let why = if by == "aifred" {
            crate::text::one_line(why, MAX_WHY)
        } else {
            String::new()
        };
        before.at = next_ts(now, old.as_ref().map(|(_, ts)| *ts));
        tx.execute(
            "INSERT INTO item_blocker (key, blocker, ts, placed_by, why, removed)
             VALUES (?1, ?2, ?3, ?4, ?5, 0)
             ON CONFLICT (key, blocker) DO UPDATE SET ts = excluded.ts,
               placed_by = excluded.placed_by, why = excluded.why, removed = 0",
            rusqlite::params![key, blocker, before.at as i64, by, why],
        )?;
        Ok(Changed {
            blockers: get_in(tx)?,
            before,
        })
    })
}

/// The time for a changed row: `now`, but always later than the time of
/// the row before. So two changes in one second have two times, and the
/// Undo of the first change sees the second change (restore).
fn next_ts(now: u64, old: Option<u64>) -> u64 {
    old.map_or(now, |ts| now.max(ts + 1))
}

/// Fails when `key` waits for [`MAX_BLOCKERS`] open items now. A finished
/// blocker, or one that is not in the list now, does not count. When
/// `blocker` is not open, the new link does not count either.
fn check_room_in(c: &Connection, key: &str, blocker: &str) -> anyhow::Result<()> {
    if !crate::items::is_open(c, blocker)? {
        return Ok(());
    }
    let n: i64 = c.query_row(
        &format!(
            "SELECT COUNT(*) FROM item_blocker WHERE key = ?1 AND removed = 0 AND {}",
            known_sql("blocker", true)
        ),
        [key],
        |r| r.get(0),
    )?;
    if n as usize >= MAX_BLOCKERS {
        bail!("an item can wait for at most {MAX_BLOCKERS} open items");
    }
    Ok(())
}

/// Takes away the link "`key` waits for `blocker`". `by` is who did it:
/// "you" (the user, in the app) or "aifred" (the AI). When the user did
/// it, the AI does not add the link again. When the AI did it, the AI can.
pub fn remove(dir: &Path, key: &str, blocker: &str, by: &str, now: u64) -> anyhow::Result<Changed> {
    let (key, blocker) = (key.trim(), blocker.trim());
    split(key)?;
    split(blocker)?;
    if !matches!(by, "you" | "aifred") {
        bail!("who took the link away must be \"you\" or \"aifred\", not \"{by}\"");
    }
    db::write(dir, |tx| {
        let old = row_in(tx, key, blocker)?;
        let at = next_ts(now, old.as_ref().map(|(_, ts)| *ts));
        let old = old.map(|(r, _)| r);
        if old.as_ref().is_none_or(|r| r.removed) {
            bail!("{key} does not wait for {blocker}");
        }
        tx.execute(
            "UPDATE item_blocker SET removed = 1, placed_by = ?4, ts = ?3
             WHERE key = ?1 AND blocker = ?2",
            rusqlite::params![key, blocker, at as i64, by],
        )?;
        Ok(Changed {
            blockers: get_in(tx)?,
            before: Before {
                key: key.into(),
                blocker: blocker.into(),
                row: old,
                at,
                gone: true,
            },
        })
    })
}

/// The Undo of add() or remove(): the row is as it was before. It works
/// only while the row is as the change left it. Other changes can have
/// come after the change, so the Undo checks the rules again.
pub fn restore(dir: &Path, before: &Before, now: u64) -> anyhow::Result<Blockers> {
    split(&before.key)?;
    split(&before.blocker)?;
    if let Some(r) = &before.row
        && !matches!(r.by.as_str(), "you" | "chat" | "aifred")
    {
        bail!(
            "who set the link must be \"you\", \"chat\" or \"aifred\", not \"{}\"",
            r.by
        );
    }
    db::write(dir, |tx| {
        let cur = row_in(tx, &before.key, &before.blocker)?;
        // An Undo from before `at` and `gone` were in Before has `at` 0: it
        // skips this check.
        let same = before.at == 0
            || cur
                .as_ref()
                .is_some_and(|(r, ts)| *ts == before.at && r.removed == before.gone);
        if !same {
            bail!("the link changed after this change, so the Undo cannot work now");
        }
        match &before.row {
            None => {
                // The Undo of an add: no row, as before.
                tx.execute(
                    "DELETE FROM item_blocker WHERE key = ?1 AND blocker = ?2 AND removed = 0",
                    [&before.key, &before.blocker],
                )?;
            }
            Some(r) => {
                // An item can be gone since (deleted, or its source
                // disconnected).
                for k in [&before.key, &before.blocker] {
                    if crate::items::title_place(tx, k)?.is_none() {
                        bail!("AIfred does not know the item {k} now");
                    }
                }
                if !r.removed {
                    // An Undo must not make two items wait for each other,
                    // or give an item too many blockers.
                    if waits_for_in(tx, &before.blocker, &before.key)? {
                        bail!(
                            "{} waits for {} now. Two items cannot wait for each other",
                            before.blocker,
                            before.key
                        );
                    }
                    let now_active = cur.as_ref().is_some_and(|(x, _)| !x.removed);
                    if !now_active {
                        check_room_in(tx, &before.key, &before.blocker)?;
                    }
                }
                tx.execute(
                    "INSERT INTO item_blocker (key, blocker, ts, placed_by, why, removed)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                     ON CONFLICT (key, blocker) DO UPDATE SET ts = excluded.ts,
                       placed_by = excluded.placed_by, why = excluded.why, removed = excluded.removed",
                    rusqlite::params![
                        before.key,
                        before.blocker,
                        now as i64,
                        r.by,
                        r.why,
                        r.removed as i64
                    ],
                )?;
            }
        }
        get_in(tx)
    })
}

/// The open blockers of each open item, for the AI (overview.rs). An item
/// whose blockers are all finished is not in the map.
pub fn open_blockers_in(c: &Connection) -> anyhow::Result<BTreeMap<String, Vec<String>>> {
    let sql = format!(
        "SELECT key, blocker FROM item_blocker
         WHERE removed = 0 AND {} AND {}
         ORDER BY key, blocker",
        known_sql("key", true),
        known_sql("blocker", true)
    );
    let mut q = c.prepare_cached(&sql)?;
    let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for r in rows {
        let (key, blocker) = r?;
        out.entry(key).or_default().push(blocker);
    }
    Ok(out)
}

/// Takes away all rows of an item that is gone. A deleted task uses it
/// (tasks.rs remove).
pub(crate) fn remove_item_in(tx: &Transaction, key: &str) -> anyhow::Result<()> {
    tx.execute(
        "DELETE FROM item_blocker WHERE key = ?1 OR blocker = ?1",
        [key],
    )?;
    Ok(())
}

/// Takes away the rows of each item that AIfred does not know now
/// (links.rs drop_gone_in, after a delete of work items).
pub(crate) fn drop_gone_in(tx: &Transaction) -> anyhow::Result<()> {
    let sql = format!(
        "DELETE FROM item_blocker WHERE NOT {} OR NOT {}",
        known_sql("key", false),
        known_sql("blocker", false)
    );
    tx.prepare_cached(&sql)?.execute([])?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-blockers-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(d: &Path, source: &str, id: &str) {
        db::write(d, |tx| {
            tx.execute(
                "INSERT INTO work_item (source, item_id, title, place, url, details, data, in_list, pos, first_seen, last_seen)
                 VALUES (?1, ?2, 'T', '', '', '', '{}', 1, 0, 0, 0)",
                [source, id],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn pairs(b: &Blockers) -> Vec<(&str, &str)> {
        b.blockers
            .iter()
            .map(|x| (x.key.as_str(), x.blocker.as_str()))
            .collect()
    }

    #[test]
    fn an_item_waits_for_many_items_and_undo_puts_it_back() {
        let d = dir("many");
        put(&d, "github", "a/b#1");
        put(&d, "github", "a/b#2");
        put(&d, "clickup", "c1");
        add(&d, "clickup:c1", "github:a/b#1", "you", "ignored", 1).unwrap();
        let c = add(&d, "clickup:c1", "github:a/b#2", "you", "", 2).unwrap();
        assert_eq!(
            pairs(&c.blockers),
            [
                ("clickup:c1", "github:a/b#1"),
                ("clickup:c1", "github:a/b#2")
            ]
        );
        assert!(
            c.blockers
                .blockers
                .iter()
                .all(|b| b.by == "you" && b.why.is_empty())
        );
        // Undo of the add: no row.
        let b = restore(&d, &c.before, 3).unwrap();
        assert_eq!(pairs(&b), [("clickup:c1", "github:a/b#1")]);
        // Remove, then Undo of the remove.
        let r = remove(&d, "clickup:c1", "github:a/b#1", "you", 4).unwrap();
        assert!(r.blockers.blockers.is_empty());
        assert!(remove(&d, "clickup:c1", "github:a/b#1", "you", 5).is_err());
        let b = restore(&d, &r.before, 6).unwrap();
        assert_eq!(pairs(&b), [("clickup:c1", "github:a/b#1")]);
        // An old Undo cannot make a loop.
        let r = remove(&d, "clickup:c1", "github:a/b#1", "you", 7).unwrap();
        add(&d, "github:a/b#1", "clickup:c1", "you", "", 8).unwrap();
        assert!(restore(&d, &r.before, 9).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn no_self_links_no_loops_no_unknown_items() {
        let d = dir("loop");
        put(&d, "github", "a/b#1");
        put(&d, "github", "a/b#2");
        put(&d, "github", "a/b#3");
        assert!(add(&d, "github:a/b#1", "github:a/b#1", "you", "", 1).is_err());
        assert!(add(&d, "github:a/b#1", "github:none#9", "you", "", 1).is_err());
        assert!(add(&d, "github:a/b#1", "nope", "you", "", 1).is_err());
        add(&d, "github:a/b#1", "github:a/b#2", "you", "", 1).unwrap();
        add(&d, "github:a/b#2", "github:a/b#3", "you", "", 1).unwrap();
        let e = add(&d, "github:a/b#3", "github:a/b#1", "you", "", 2).unwrap_err();
        assert!(e.to_string().contains("cannot wait for each other"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_ai_does_not_add_back_what_the_user_took_away() {
        let d = dir("ai");
        put(&d, "github", "a/b#1");
        put(&d, "github", "a/b#2");
        let c = add(
            &d,
            "github:a/b#1",
            "github:a/b#2",
            "aifred",
            "The PR needs the API change.",
            1,
        )
        .unwrap();
        assert_eq!(c.blockers.blockers[0].by, "aifred");
        assert_eq!(c.blockers.blockers[0].why, "The PR needs the API change.");
        remove(&d, "github:a/b#1", "github:a/b#2", "you", 2).unwrap();
        let e = add(&d, "github:a/b#1", "github:a/b#2", "aifred", "again", 3).unwrap_err();
        assert!(e.to_string().contains("Do not add it again"), "{e}");
        // The user can add it again.
        let c = add(&d, "github:a/b#1", "github:a/b#2", "you", "", 4).unwrap();
        assert_eq!(c.blockers.blockers[0].by, "you");
        // The AI does not change a link of the user.
        let c = add(&d, "github:a/b#1", "github:a/b#2", "aifred", "x", 5).unwrap();
        assert_eq!(c.blockers.blockers[0].by, "you");
        assert!(c.blockers.blockers[0].why.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_chat_cannot_add_back_what_the_user_took_away_in_the_app() {
        let d = dir("chat");
        put(&d, "github", "a/b#1");
        put(&d, "github", "a/b#2");
        add(&d, "github:a/b#1", "github:a/b#2", "you", "", 1).unwrap();
        remove(&d, "github:a/b#1", "github:a/b#2", "you", 2).unwrap();
        // The AI says that the user asked: it is not enough.
        let e = add(&d, "github:a/b#1", "github:a/b#2", "chat", "", 3).unwrap_err();
        assert!(e.to_string().contains("Do not add it again"), "{e}");
        // The chat does not relabel a link that the user set in the app.
        let c = add(&d, "github:a/b#1", "github:a/b#2", "you", "", 3).unwrap();
        let c2 = add(&d, "github:a/b#1", "github:a/b#2", "chat", "", 3).unwrap();
        assert_eq!(c2.blockers.blockers[0].by, "you");
        assert_eq!(c2.before.at, c.before.at);
        remove(&d, "github:a/b#1", "github:a/b#2", "you", 3).unwrap();
        // A new link from the chat shows that it came from a chat.
        put(&d, "github", "a/b#3");
        let c = add(&d, "github:a/b#1", "github:a/b#3", "chat", "x", 4).unwrap();
        assert!(
            c.blockers
                .blockers
                .iter()
                .any(|b| b.blocker == "github:a/b#3" && b.by == "chat" && b.why.is_empty())
        );
        // The AI by itself does not change it.
        let c = add(&d, "github:a/b#1", "github:a/b#3", "aifred", "y", 5).unwrap();
        assert!(c.blockers.blockers.iter().any(|b| b.by == "chat"));
        // Its Undo keeps "chat".
        let r = remove(&d, "github:a/b#1", "github:a/b#3", "you", 6).unwrap();
        let b = restore(&d, &r.before, 7).unwrap();
        assert!(b.blockers.iter().any(|b| b.by == "chat"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_removal_by_the_ai_is_no_mark_of_the_user() {
        let d = dir("ai-remove");
        put(&d, "github", "a/b#1");
        put(&d, "github", "a/b#2");
        add(&d, "github:a/b#1", "github:a/b#2", "aifred", "why", 1).unwrap();
        remove(&d, "github:a/b#1", "github:a/b#2", "aifred", 2).unwrap();
        // The AI can add it again: the user did not take it away.
        let c = add(&d, "github:a/b#1", "github:a/b#2", "aifred", "why", 3).unwrap();
        assert_eq!(c.blockers.blockers.len(), 1);
        assert!(remove(&d, "github:a/b#1", "github:a/b#2", "chat", 4).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_old_undo_keeps_the_rules() {
        let d = dir("undo");
        put(&d, "github", "a/b#1");
        put(&d, "github", "a/b#2");
        // Undo of an add, after a remove: it does nothing, and the mark
        // "taken away" stays.
        let c = add(&d, "github:a/b#1", "github:a/b#2", "you", "", 1).unwrap();
        remove(&d, "github:a/b#1", "github:a/b#2", "you", 2).unwrap();
        let e = restore(&d, &c.before, 3).unwrap_err();
        assert!(e.to_string().contains("changed after"), "{e}");
        assert!(add(&d, "github:a/b#1", "github:a/b#2", "aifred", "x", 4).is_err());
        // Add, remove, add again: the Undo of the first add does not take
        // away the new link.
        put(&d, "github", "a/b#3");
        let c = add(&d, "github:a/b#1", "github:a/b#3", "you", "", 4).unwrap();
        remove(&d, "github:a/b#1", "github:a/b#3", "you", 5).unwrap();
        add(&d, "github:a/b#1", "github:a/b#3", "you", "", 6).unwrap();
        assert!(restore(&d, &c.before, 7).is_err());
        assert!(
            get(&d)
                .unwrap()
                .blockers
                .iter()
                .any(|b| b.blocker == "github:a/b#3")
        );
        remove(&d, "github:a/b#1", "github:a/b#3", "you", 8).unwrap();
        // The same, all in one second.
        put(&d, "github", "a/b#4");
        let c = add(&d, "github:a/b#1", "github:a/b#4", "you", "", 9).unwrap();
        remove(&d, "github:a/b#1", "github:a/b#4", "you", 9).unwrap();
        add(&d, "github:a/b#1", "github:a/b#4", "you", "", 9).unwrap();
        assert!(restore(&d, &c.before, 9).is_err());
        remove(&d, "github:a/b#1", "github:a/b#4", "you", 9).unwrap();
        // Undo of a remove, after the item is gone.
        let s = TaskStore::new(&d);
        let t = s.add("Deploy", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        add(&d, "github:a/b#1", &tk, "you", "", 5).unwrap();
        let r = remove(&d, "github:a/b#1", &tk, "you", 6).unwrap();
        s.remove(&t.id).unwrap();
        assert!(restore(&d, &r.before, 7).is_err());
        assert!(get(&d).unwrap().blockers.is_empty());
        // Undo of a remove, when the item waits for the most items now.
        for n in 1..=MAX_BLOCKERS {
            put(&d, "clickup", &format!("c{n}"));
        }
        for n in 1..MAX_BLOCKERS {
            add(&d, "github:a/b#1", &format!("clickup:c{n}"), "you", "", 8).unwrap();
        }
        put(&d, "gmail", "m1");
        add(&d, "github:a/b#1", "gmail:m1", "you", "", 8).unwrap();
        let r = remove(&d, "github:a/b#1", "gmail:m1", "you", 9).unwrap();
        add(
            &d,
            "github:a/b#1",
            &format!("clickup:c{MAX_BLOCKERS}"),
            "you",
            "",
            10,
        )
        .unwrap();
        let e = restore(&d, &r.before, 11).unwrap_err();
        assert!(e.to_string().contains("at most"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_waits_for_at_most_max_blockers() {
        let d = dir("max");
        put(&d, "github", "a/b#0");
        for n in 1..=MAX_BLOCKERS {
            put(&d, "clickup", &format!("c{n}"));
            add(&d, "github:a/b#0", &format!("clickup:c{n}"), "you", "", 1).unwrap();
        }
        put(&d, "gmail", "m1");
        assert!(add(&d, "github:a/b#0", "gmail:m1", "you", "", 2).is_err());
        // The same link again is no new link.
        add(&d, "github:a/b#0", "clickup:c1", "you", "", 3).unwrap();
        // A blocker that is not in the list now does not count.
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE work_item SET in_list = 0 WHERE source = 'clickup' AND item_id = 'c1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        add(&d, "github:a/b#0", "gmail:m1", "you", "", 4).unwrap();
        // A finished task does not count, and a link to it is no new link.
        let s = TaskStore::new(&d);
        let t = s.add("Old", "", "you").unwrap();
        s.set_done(&t.id, true).unwrap();
        add(&d, "github:a/b#0", &format!("task:{}", t.id), "you", "", 5).unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn links_of_finished_items_do_not_stop_a_new_link() {
        let d = dir("asleep");
        let s = TaskStore::new(&d);
        let [a, b, c] = ["A", "B", "C"].map(|t| s.add(t, "", "you").unwrap());
        let [ak, bk, ck] = [&a, &b, &c].map(|t| format!("task:{}", t.id));
        // A waits for B. Then both are finished.
        add(&d, &ak, &bk, "you", "", 1).unwrap();
        s.set_done(&a.id, true).unwrap();
        s.set_done(&b.id, true).unwrap();
        // B opens again. C waits for A, and B waits for C. Through the old
        // link, C waits for B, but A is finished, so the link is asleep.
        s.set_done(&b.id, false).unwrap();
        add(&d, &ck, &ak, "you", "", 2).unwrap();
        add(&d, &bk, &ck, "you", "", 3).unwrap();
        // A open link still stops a loop.
        let e = add(&d, &ck, &bk, "you", "", 4).unwrap_err();
        assert!(e.to_string().contains("cannot wait for each other"), "{e}");
        // A opens again: now A, B and C wait for each other. The check for
        // the AI ends, and the user can take a link away.
        s.set_done(&a.id, false).unwrap();
        let m = db::read(&d, open_blockers_in).unwrap();
        assert_eq!(m.len(), 3);
        remove(&d, &ak, &bk, "you", 5).unwrap();
        assert_eq!(db::read(&d, open_blockers_in).unwrap().len(), 2);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_ai_sees_only_open_blockers_of_open_items() {
        let d = dir("open");
        put(&d, "github", "a/b#1");
        let s = TaskStore::new(&d);
        let t = s.add("Deploy", "", "you").unwrap();
        let u = s.add("Write notes", "", "you").unwrap();
        let (tk, uk) = (format!("task:{}", t.id), format!("task:{}", u.id));
        add(&d, &tk, "github:a/b#1", "you", "", 1).unwrap();
        add(&d, &tk, &uk, "you", "", 1).unwrap();
        let m = db::read(&d, open_blockers_in).unwrap();
        assert_eq!(m[&tk].len(), 2);
        // A finished blocker does not block. The row stays.
        s.set_done(&u.id, true).unwrap();
        let m = db::read(&d, open_blockers_in).unwrap();
        assert_eq!(m[&tk], ["github:a/b#1"]);
        assert_eq!(get(&d).unwrap().blockers.len(), 2);
        // A GitHub item that left the list does not block. The row stays.
        db::write(&d, |tx| {
            tx.execute("UPDATE work_item SET in_list = 0", [])?;
            crate::links::drop_gone_in(tx, 1)
        })
        .unwrap();
        assert!(db::read(&d, open_blockers_in).unwrap().is_empty());
        assert_eq!(get(&d).unwrap().blockers.len(), 2);
        // A finished item is not blocked.
        s.set_done(&t.id, true).unwrap();
        assert!(db::read(&d, open_blockers_in).unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn gone_items_lose_their_rows() {
        let d = dir("gone");
        put(&d, "github", "a/b#1");
        put(&d, "clickup", "c1");
        let s = TaskStore::new(&d);
        let t = s.add("Deploy", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        add(&d, &tk, "clickup:c1", "you", "", 1).unwrap();
        add(&d, "github:a/b#1", &tk, "you", "", 1).unwrap();
        // The user disconnects ClickUp.
        db::write(&d, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = 'clickup'", [])?;
            crate::links::drop_gone_in(tx, 2)
        })
        .unwrap();
        assert_eq!(pairs(&get(&d).unwrap()), [("github:a/b#1", tk.as_str())]);
        // A deleted task.
        s.remove(&t.id).unwrap();
        assert!(get(&d).unwrap().blockers.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
