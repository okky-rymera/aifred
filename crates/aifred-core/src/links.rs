//! Subtasks: items under other items, in AIfred only. A
//! ClickUp task, its GitHub issue, and the pull request of the issue are one
//! tree: the issue under the ClickUp task, the pull request under the issue.
//! ClickUp and GitHub do not change.
//!
//! - Each row of `item_link` puts `key` under its parent `main`. An item
//!   has at most one parent. A tree can have many levels.
//! - The top item of a tree is its root. The home list shows one row for
//!   each tree, with the open subtasks under it.
//! - `placed_by` says who placed the item: "you" or "aifred". `why` is AIfred's
//!   reason. The AI never moves an item that the user placed (sort.rs).
//!   A move by the user, and "Move out", mark the item as placed by the user.
//! - An item cannot go under itself or under one of its own subtasks.
//! - A move takes the subtasks of the item with it. So does "move out".
//!   When an item is gone (deleted, or its source is disconnected), its
//!   subtasks move up to its parent.
//!
//! GitHub links between an issue and its pull request stay in github_list.rs.

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::items::split;

/// At most this many items in one tree.
pub const MAX_GROUP: usize = 10;
/// A walk up the tree stops after this many levels. A tree is never this
/// deep: the cap only stops a loop in a broken database.
const MAX_DEPTH: usize = MAX_GROUP;

/// One item under another item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The item key of the subtask.
    pub key: String,
    /// The item key of its parent.
    pub main: String,
    /// Who placed it: "you" or "aifred".
    pub by: String,
    /// AIfred's reason. Empty when the user placed it.
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Links {
    pub links: Vec<Link>,
}

/// The links.
pub fn get(dir: &Path) -> anyhow::Result<Links> {
    db::read(dir, get_in)
}

pub fn get_in(c: &Connection) -> anyhow::Result<Links> {
    Ok(Links { links: all(c)? })
}

fn all(c: &Connection) -> anyhow::Result<Vec<Link>> {
    let mut q = c.prepare("SELECT key, main, placed_by, why FROM item_link ORDER BY main, key")?;
    let rows = q.query_map([], |r| {
        Ok(Link {
            key: r.get(0)?,
            main: r.get(1)?,
            by: r.get(2)?,
            why: r.get(3)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The parent of `key`. None when it is a top item.
pub(crate) fn parent_in(c: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    Ok(
        c.query_row("SELECT main FROM item_link WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?,
    )
}

/// The root of `key`'s tree. An item with no parent is its own root.
pub(crate) fn head_in(c: &Connection, key: &str) -> anyhow::Result<String> {
    let mut at = key.to_string();
    for _ in 0..MAX_DEPTH {
        match parent_in(c, &at)? {
            Some(p) => at = p,
            None => break,
        }
    }
    Ok(at)
}

/// The items above `key`: its parent first, then the parent of the parent,
/// up to the root. Empty for a top item.
pub(crate) fn above_in(c: &Connection, key: &str) -> anyhow::Result<Vec<String>> {
    let mut out = Vec::new();
    let mut at = key.to_string();
    for _ in 0..MAX_DEPTH {
        match parent_in(c, &at)? {
            Some(p) if p != key && !out.contains(&p) => {
                out.push(p.clone());
                at = p;
            }
            _ => break,
        }
    }
    Ok(out)
}

/// The items directly under `key`, in key order, with who placed each one.
pub(crate) fn children_in(c: &Connection, key: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut q =
        c.prepare_cached("SELECT key, placed_by FROM item_link WHERE main = ?1 ORDER BY key")?;
    let rows = q.query_map([key], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// All items of `key`'s tree, the root first, then each item before its
/// subtasks. An item with no link is a tree of one. The AI builds trees, so
/// the items of one tree are not always the same piece of work: to find the
/// work of one item, use above_in() and children_in().
pub fn group(dir: &Path, key: &str) -> anyhow::Result<Vec<String>> {
    db::read(dir, |c| group_in(c, key))
}

/// group(), with a connection that is open.
pub(crate) fn group_in(c: &Connection, key: &str) -> anyhow::Result<Vec<String>> {
    members_in(c, &head_in(c, key)?)
}

/// `top` and all items under it, each item before its subtasks. The
/// subtasks of one item are in key order.
fn members_in(c: &Connection, top: &str) -> anyhow::Result<Vec<String>> {
    let mut q = c.prepare_cached("SELECT key FROM item_link WHERE main = ?1 ORDER BY key")?;
    let mut out = Vec::new();
    let mut todo = vec![top.to_string()];
    while let Some(k) = todo.pop() {
        if out.contains(&k) || out.len() > MAX_GROUP * 2 {
            // A loop in a broken database.
            break;
        }
        let kids: Vec<String> = q
            .query_map([&k], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?;
        out.push(k);
        todo.extend(kids.into_iter().rev());
    }
    Ok(out)
}

/// The mark of one item for the AI sort (`item_sort`), before a change.
/// None: the sort had not seen the item. true: the user placed it, so the
/// AI does not move it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mark {
    pub key: String,
    pub fixed: Option<bool>,
}

/// How things were before a move by the user, so that its Undo puts back
/// all of it: the old link of the item, and the sort marks of the item and
/// of the two parents.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Before {
    /// The item that moved.
    pub key: String,
    /// Its link before the move. None: it was a top item.
    pub link: Option<Link>,
    pub marks: Vec<Mark>,
}

/// The links after a move by the user, and how things were before it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Changed {
    #[serde(flatten)]
    pub links: Links,
    pub before: Before,
}

/// Reads how things are now for a move of `key`. `parents` are the new
/// parent of the move, if any. The old parent is added here.
fn before_in(c: &Connection, key: &str, parents: &[&str]) -> anyhow::Result<Before> {
    let link = all(c)?.into_iter().find(|l| l.key == key);
    let mut keys = vec![key.to_string()];
    for k in parents
        .iter()
        .copied()
        .chain(link.as_ref().map(|l| l.main.as_str()))
    {
        if !keys.iter().any(|x| x == k) {
            keys.push(k.to_string());
        }
    }
    let mut marks = Vec::new();
    for k in keys {
        let fixed: Option<bool> = c
            .query_row("SELECT fixed FROM item_sort WHERE key = ?1", [&k], |r| {
                r.get::<_, i64>(0).map(|f| f != 0)
            })
            .optional()?;
        marks.push(Mark { key: k, fixed });
    }
    Ok(Before {
        key: key.to_string(),
        link,
        marks,
    })
}

/// Puts `key` under `main`, with its own subtasks. The user placed it.
pub fn link(dir: &Path, main: &str, key: &str, now: u64) -> anyhow::Result<Links> {
    Ok(link_undoable(dir, main, key, now)?.links)
}

/// link(), and how things were before it, for its Undo (restore()).
pub fn link_undoable(dir: &Path, main: &str, key: &str, now: u64) -> anyhow::Result<Changed> {
    db::write(dir, |tx| {
        let before = before_in(tx, key, &[main])?;
        link_in(tx, main, key, "you", "", now)?;
        Ok(Changed {
            links: get_in(tx)?,
            before,
        })
    })
}

/// The Undo of a move by the user (link_undoable() or unlink_undoable()).
/// The item goes back to its old place, with who placed it and AIfred's
/// reason. The sort marks of the item and its parents are as before the
/// move. So an item that the sort had not seen is new again, and an item
/// that the AI placed can move again.
pub fn restore(dir: &Path, before: &Before, now: u64) -> anyhow::Result<Links> {
    split(&before.key)?;
    db::write(dir, |tx| {
        match &before.link {
            Some(l) => {
                let (by, why) = if l.by == "aifred" {
                    ("aifred", l.why.as_str())
                } else {
                    ("you", "")
                };
                link_in(tx, &l.main, &before.key, by, why, now)?;
            }
            None => {
                tx.execute("DELETE FROM item_link WHERE key = ?1", [&before.key])?;
            }
        }
        for m in &before.marks {
            match m.fixed {
                None => tx.execute("DELETE FROM item_sort WHERE key = ?1", [&m.key])?,
                Some(f) => tx.execute(
                    "INSERT INTO item_sort (key, ts, fixed) VALUES (?1, ?2, ?3)
                     ON CONFLICT (key) DO UPDATE SET fixed = excluded.fixed",
                    rusqlite::params![m.key, now as i64, f as i64],
                )?,
            };
        }
        get_in(tx)
    })
}

/// link(), in a change that is open. `by` is "you" or "aifred".
///
/// It reads each item of the two trees with one query, so about 20
/// queries for two full trees. A loop over many moves is slow: keep it small.
pub(crate) fn link_in(
    tx: &Transaction,
    main: &str,
    key: &str,
    by: &str,
    why: &str,
    now: u64,
) -> anyhow::Result<()> {
    // A link to an item that is gone must not count toward MAX_GROUP.
    drop_gone_in(tx, now)?;
    place_in(tx, main, key, by, why, now)?;
    // The user built this part of the tree: the AI sort must move neither
    // the item nor its new parent.
    if by == "you" {
        fix_in(tx, key, now)?;
        fix_in(tx, main, now)?;
    }
    Ok(())
}

/// link_in() with no clean-up of the items that are gone first, and with
/// no sort marks: the caller sets them (fix_in()). A loop of many moves
/// calls drop_gone_in() one time, then this for each move (sort.rs):
/// drop_gone_in() reads each linked item.
pub(crate) fn place_in(
    tx: &Transaction,
    main: &str,
    key: &str,
    by: &str,
    why: &str,
    now: u64,
) -> anyhow::Result<()> {
    split(main)?;
    split(key)?;
    if main == key {
        bail!("an item cannot be a subtask of itself");
    }
    for k in [main, key] {
        if crate::items::title_place(tx, k)?.is_none() {
            bail!("AIfred does not know the item {k}");
        }
    }
    let moving = members_in(tx, key)?;
    if moving.iter().any(|k| k == main) {
        bail!("{main} is a subtask of {key}. An item cannot go under its own subtask");
    }
    let tree = group_in(tx, main)?;
    let new: BTreeSet<&String> = tree.iter().chain(moving.iter()).collect();
    if new.len() > MAX_GROUP {
        bail!("one tree can have at most {MAX_GROUP} items");
    }
    tx.execute(
        "INSERT INTO item_link (key, main, ts, placed_by, why) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (key) DO UPDATE SET main = excluded.main, ts = excluded.ts,
           placed_by = excluded.placed_by, why = excluded.why",
        rusqlite::params![key, main, now as i64, by, why],
    )?;
    Ok(())
}

/// Marks `key` as placed by the user: the AI sort never moves it (sort.rs).
pub(crate) fn fix_in(tx: &Transaction, key: &str, now: u64) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO item_sort (key, ts, fixed) VALUES (?1, ?2, 1)
         ON CONFLICT (key) DO UPDATE SET fixed = 1, ts = excluded.ts",
        rusqlite::params![key, now as i64],
    )?;
    Ok(())
}

/// Moves `key` out from under its parent, with its own subtasks. It is a
/// top item then. A top item does not change. The user placed it there, so
/// the AI sort does not put it back.
pub fn unlink(dir: &Path, key: &str, now: u64) -> anyhow::Result<Links> {
    Ok(unlink_undoable(dir, key, now)?.links)
}

/// unlink(), and how things were before it, for its Undo (restore()).
pub fn unlink_undoable(dir: &Path, key: &str, now: u64) -> anyhow::Result<Changed> {
    split(key)?;
    db::write(dir, |tx| {
        let before = before_in(tx, key, &[])?;
        unlink_in(tx, key, now)?;
        Ok(Changed {
            links: get_in(tx)?,
            before,
        })
    })
}

/// unlink(), in a change that is open.
pub fn unlink_in(tx: &Transaction, key: &str, now: u64) -> anyhow::Result<()> {
    tx.execute("DELETE FROM item_link WHERE key = ?1", [key])?;
    fix_in(tx, key, now)
}

/// Takes an item that is gone out of its tree. Its subtasks move up to its
/// parent, or are top items when it had none. A deleted task uses it
/// (tasks.rs), and so does drop_gone_in().
pub fn remove_in(tx: &Transaction, key: &str, now: u64) -> anyhow::Result<()> {
    match parent_in(tx, key)? {
        Some(p) => tx.execute(
            "UPDATE item_link SET main = ?2, ts = ?3 WHERE main = ?1",
            rusqlite::params![key, p, now as i64],
        )?,
        None => tx.execute("DELETE FROM item_link WHERE main = ?1", [key])?,
    };
    tx.execute("DELETE FROM item_link WHERE key = ?1", [key])?;
    Ok(())
}

/// Takes each item that AIfred does not know now out of its tree. Use it
/// after a delete of work items, for example when the user disconnects
/// ClickUp or Gmail (clickup_list.rs, gmail.rs).
pub fn drop_gone_in(tx: &Transaction, now: u64) -> anyhow::Result<()> {
    let mut keys = BTreeSet::new();
    for l in all(tx)? {
        keys.insert(l.key);
        keys.insert(l.main);
    }
    for k in keys {
        if crate::items::title_place(tx, &k)?.is_none() {
            remove_in(tx, &k, now)?;
        }
    }
    Ok(())
}

/// The parent of each open subtask, for the AI (overview.rs). When the
/// parent is closed, the nearest open item above it takes its place. A
/// subtask with no open item above it has no parent here.
pub fn parents_for_ai(c: &Connection) -> anyhow::Result<HashMap<String, String>> {
    let links = all(c)?;
    let parent: HashMap<&str, &str> = links
        .iter()
        .map(|l| (l.key.as_str(), l.main.as_str()))
        .collect();
    let mut out = HashMap::new();
    for l in &links {
        if !crate::items::is_open(c, &l.key)? {
            continue;
        }
        let mut at = l.main.as_str();
        for _ in 0..MAX_DEPTH {
            if crate::items::is_open(c, at)? {
                out.insert(l.key.clone(), at.to_string());
                break;
            }
            match parent.get(at) {
                Some(p) => at = p,
                None => break,
            }
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(d: &Path, source: &str, id: &str, title: &str) {
        db::write(d, |tx| {
            tx.execute(
                "INSERT INTO work_item (source, item_id, title, place, url, details, data, in_list, pos, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, '', '', '', '{}', 1, 0, 0, 0)",
                [source, id, title],
            )?;
            Ok(())
        })
        .unwrap();
    }

    /// The links as (subtask, parent) pairs.
    fn pairs(l: &Links) -> Vec<(&str, &str)> {
        l.links
            .iter()
            .map(|x| (x.key.as_str(), x.main.as_str()))
            .collect()
    }

    /// A ClickUp task, a GitHub issue, its pull request, and an email.
    fn four(d: &Path) {
        put(d, "clickup", "c1", "DEV-123 Checkout totals");
        put(d, "github", "a/b#1", "Checkout totals are wrong");
        put(d, "github", "a/b#2", "Fix checkout totals");
        put(d, "gmail", "m1", "Re: wrong totals");
    }

    #[test]
    fn a_tree_has_many_levels_and_a_move_takes_the_subtasks_with_it() {
        let d = dir("tree");
        four(&d);
        link(&d, "github:a/b#1", "github:a/b#2", 1).unwrap();
        let l = link(&d, "clickup:c1", "github:a/b#1", 2).unwrap();
        assert_eq!(
            pairs(&l),
            [
                ("github:a/b#1", "clickup:c1"),
                ("github:a/b#2", "github:a/b#1")
            ]
        );
        assert!(l.links.iter().all(|x| x.by == "you" && x.why.is_empty()));
        // The root first, then each item before its subtasks.
        let g = group(&d, "github:a/b#2").unwrap();
        assert_eq!(g, ["clickup:c1", "github:a/b#1", "github:a/b#2"]);
        assert_eq!(
            db::read(&d, |c| head_in(c, "github:a/b#2")).unwrap(),
            "clickup:c1"
        );

        // Move out: the issue leaves with its pull request.
        let l = unlink(&d, "github:a/b#1", 3).unwrap();
        assert_eq!(pairs(&l), [("github:a/b#2", "github:a/b#1")]);
        assert_eq!(group(&d, "clickup:c1").unwrap(), ["clickup:c1"]);
        // A top item does not change.
        assert_eq!(pairs(&unlink(&d, "github:a/b#1", 4).unwrap()).len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_cannot_go_under_itself_or_its_own_subtask() {
        let d = dir("loop");
        four(&d);
        link(&d, "clickup:c1", "github:a/b#1", 1).unwrap();
        link(&d, "github:a/b#1", "github:a/b#2", 1).unwrap();
        let e = link(&d, "github:a/b#2", "clickup:c1", 2).unwrap_err();
        assert!(e.to_string().contains("its own subtask"), "{e}");
        assert!(link(&d, "clickup:c1", "clickup:c1", 2).is_err());
        assert!(
            link(&d, "clickup:c1", "github:none#1", 2).is_err(),
            "an unknown item"
        );
        // A move inside the tree is fine: the pull request goes up one level.
        let l = link(&d, "clickup:c1", "github:a/b#2", 3).unwrap();
        assert_eq!(
            pairs(&l),
            [
                ("github:a/b#1", "clickup:c1"),
                ("github:a/b#2", "clickup:c1")
            ]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_tree_has_at_most_max_group_items_and_gone_items_do_not_count() {
        let d = dir("max");
        put(&d, "github", "a/b#0", "T");
        for n in 1..MAX_GROUP {
            put(&d, "clickup", &format!("c{n}"), "T");
            link(&d, "github:a/b#0", &format!("clickup:c{n}"), 1).unwrap();
        }
        put(&d, "gmail", "m1", "T");
        assert!(link(&d, "github:a/b#0", "gmail:m1", 2).is_err());
        // An old database: ClickUp items were deleted before drop_gone_in existed.
        db::write(&d, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = 'clickup'", [])?;
            Ok(())
        })
        .unwrap();
        let l = link(&d, "github:a/b#0", "gmail:m1", 2).unwrap();
        assert_eq!(pairs(&l), [("gmail:m1", "github:a/b#0")]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn when_an_item_is_gone_its_subtasks_move_up() {
        let d = dir("gone");
        four(&d);
        put(&d, "github", "a/b#3", "Other");
        link(&d, "github:a/b#3", "clickup:c1", 1).unwrap();
        link(&d, "clickup:c1", "github:a/b#1", 1).unwrap();
        link(&d, "clickup:c1", "gmail:m1", 1).unwrap();
        // The user disconnects ClickUp: its items are deleted.
        db::write(&d, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = 'clickup'", [])?;
            drop_gone_in(tx, 2)
        })
        .unwrap();
        assert_eq!(
            pairs(&get(&d).unwrap()),
            [
                ("github:a/b#1", "github:a/b#3"),
                ("gmail:m1", "github:a/b#3")
            ]
        );
        // A gone top item: its subtasks are top items.
        db::write(&d, |tx| {
            tx.execute("DELETE FROM work_item WHERE item_id = 'a/b#3'", [])?;
            drop_gone_in(tx, 3)
        })
        .unwrap();
        assert!(get(&d).unwrap().links.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_deleted_task_gives_its_subtasks_to_its_parent() {
        let d = dir("deleted");
        four(&d);
        let s = TaskStore::new(&d);
        let t = s.add("T", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        link(&d, "clickup:c1", &tk, 1).unwrap();
        link(&d, &tk, "github:a/b#1", 1).unwrap();
        s.remove(&t.id).unwrap();
        assert_eq!(pairs(&get(&d).unwrap()), [("github:a/b#1", "clickup:c1")]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_ai_gets_the_nearest_open_parent_of_each_open_subtask() {
        let d = dir("ai");
        four(&d);
        let s = TaskStore::new(&d);
        let t = s.add("T", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        let u = s.add("U", "", "you").unwrap();
        let uk = format!("task:{}", u.id);
        link(&d, &tk, "clickup:c1", 1).unwrap();
        link(&d, "clickup:c1", "github:a/b#1", 1).unwrap();
        link(&d, "github:a/b#1", &uk, 1).unwrap();
        let g = db::read(&d, parents_for_ai).unwrap();
        assert_eq!(g.len(), 3);
        assert_eq!(g["clickup:c1"], tk);
        assert_eq!(g["github:a/b#1"], "clickup:c1");
        // A done subtask has no parent here. Its open subtask gets the next
        // open item above.
        s.set_done(&t.id, true).unwrap();
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE work_item SET in_list = 0 WHERE source = 'github' AND item_id = 'a/b#1'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let g = db::read(&d, parents_for_ai).unwrap();
        assert_eq!(g.len(), 1, "{g:?}");
        assert_eq!(g[&uk], "clickup:c1");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
