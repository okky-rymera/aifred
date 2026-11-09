//! Task context: AIfred's short notes about one item, from the
//! user's answers. For example: "Fix sync is the ClickUp sync bug that Ana
//! reported. It blocks the release on Friday."
//!
//! - The brief finds the items that it cannot rank: a short title, no
//!   description, no goal, no due day, no parent. The home page asks the
//!   user about at most 3 of them each day ("Help AIfred understand").
//! - The user answers in the chat of the item. The AI saves a short summary
//!   with the set_item_context tool.
//! - The overview, the brief, and the item chat get the notes.
//! - The item panel shows them in a closed section. The user can change
//!   or clear them, with Undo.
//!
//! The notes are for one item key. A mission uses the notes of its items.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db;

/// The longest note that AIfred keeps, in characters. The note goes into
/// each brief and each overview, so it must stay short.
pub const MAX_CHARS: usize = 600;

/// The most items that the home page asks about in one day.
pub const MAX_ASK: usize = 3;

/// A title with this many words or fewer is short: it does not say much.
const SHORT_TITLE_WORDS: usize = 4;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ItemContext {
    pub key: String,
    pub text: String,
    /// "you" or "aifred": who wrote it last.
    pub by: String,
    pub updated: u64,
}

pub fn get(dir: &Path, key: &str) -> anyhow::Result<Option<ItemContext>> {
    let key = key.trim();
    crate::items::split(key)?;
    db::read(dir, |c| get_in(c, key))
}

pub(crate) fn get_in(c: &Connection, key: &str) -> anyhow::Result<Option<ItemContext>> {
    Ok(c.query_row(
        "SELECT key, text, by, updated FROM item_context WHERE key = ?1",
        [key],
        |r| {
            Ok(ItemContext {
                key: r.get(0)?,
                text: r.get(1)?,
                by: r.get(2)?,
                updated: r.get::<_, i64>(3)?.max(0) as u64,
            })
        },
    )
    .optional()?)
}

/// What set() did: the note from before (for the Undo) and the saved text.
#[derive(Debug, Clone, PartialEq)]
pub struct Saved {
    pub before: Option<ItemContext>,
    pub text: String,
    /// True when the text was longer than MAX_CHARS and its end was cut.
    pub cut: bool,
}

/// Saves the note of `key`. An empty text clears it. A longer text than
/// MAX_CHARS is cut. Spaces around the key do not count.
pub fn set(dir: &Path, key: &str, text: &str, by: &str, now: u64) -> anyhow::Result<Saved> {
    let key = key.trim();
    crate::items::split(key)?;
    check_by(by)?;
    let full = text.trim();
    let text = shorten(full);
    let cut = text != full;
    let before = db::write(dir, |tx| {
        let before = get_in(tx, key)?;
        if text.is_empty() {
            tx.execute("DELETE FROM item_context WHERE key = ?1", [key])?;
        } else {
            tx.execute(
                "INSERT INTO item_context (key, text, by, updated) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (key) DO UPDATE SET text = ?2, by = ?3, updated = ?4",
                rusqlite::params![key, text, by, now as i64],
            )?;
        }
        Ok(before)
    })?;
    Ok(Saved { before, text, cut })
}

fn check_by(by: &str) -> anyhow::Result<()> {
    if by != "you" && by != "aifred" {
        bail!("\"{by}\" is not \"you\" or \"aifred\"");
    }
    Ok(())
}

/// Puts back the note of `key` as it was before a change: the Undo of set().
/// None: the item had no note.
pub fn restore(dir: &Path, key: &str, before: Option<&ItemContext>) -> anyhow::Result<()> {
    let key = key.trim();
    crate::items::split(key)?;
    if let Some(b) = before {
        if b.key != key {
            bail!("the note is for {}, not for {key}", b.key);
        }
        check_by(&b.by)?;
    }
    db::write(dir, |tx| {
        tx.execute("DELETE FROM item_context WHERE key = ?1", [key])?;
        if let Some(b) = before {
            tx.execute(
                "INSERT INTO item_context (key, text, by, updated) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![key, shorten(&b.text), b.by, b.updated as i64],
            )?;
        }
        Ok(())
    })
}

/// The user does not want to explain `key`: AIfred does not ask again.
/// `undo` true asks again.
pub fn skip(dir: &Path, key: &str, undo: bool, now: u64) -> anyhow::Result<()> {
    let key = key.trim();
    crate::items::split(key)?;
    db::write(dir, |tx| {
        if undo {
            tx.execute("DELETE FROM context_skip WHERE key = ?1", [key])?;
        } else {
            tx.execute(
                "INSERT INTO context_skip (key, ts) VALUES (?1, ?2) ON CONFLICT (key) DO NOTHING",
                rusqlite::params![key, now as i64],
            )?;
        }
        Ok(())
    })
}

fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let c: String = text.chars().take(MAX_CHARS - 1).collect();
    format!("{}…", c.trim_end())
}

/// All notes, by item key.
pub(crate) fn map_in(c: &Connection) -> anyhow::Result<HashMap<String, String>> {
    let mut q = c.prepare("SELECT key, text FROM item_context")?;
    let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

pub(crate) fn skipped_in(c: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut q = c.prepare("SELECT key FROM context_skip")?;
    let rows = q.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The items of `ask` (the saved list of a brief) that the home page still
/// asks about: no notes, not skipped, not corrected on the day of the brief
/// (`corrected`, from learn::corrected_on), and open. brief::get and the
/// item chat use the same rule.
pub(crate) fn still_ask_in(
    c: &Connection,
    corrected: &HashSet<String>,
    ask: Vec<String>,
) -> anyhow::Result<Vec<String>> {
    let skipped = skipped_in(c)?;
    let mut out = Vec::new();
    for k in ask {
        if !corrected.contains(&k)
            && !skipped.contains(&k)
            && get_in(c, &k)?.is_none()
            && crate::items::is_open(c, &k)?
        {
            out.push(k);
        }
    }
    Ok(out)
}

/// True when the brief of `day` still asks the user about `key` (see
/// still_ask_in). The item chat then knows what AIfred asked. The home page
/// shows only today's brief, so the chat gives today as `day`.
pub(crate) fn asked_in(c: &Connection, day: &str, key: &str) -> anyhow::Result<bool> {
    let ask: Option<String> = c
        .query_row("SELECT ask FROM brief WHERE day = ?1", [day], |r| r.get(0))
        .optional()?;
    let Some(ask) = ask else {
        return Ok(false);
    };
    let ask: Vec<String> = serde_json::from_str(&ask)?;
    if !ask.iter().any(|k| k == key) {
        return Ok(false);
    }
    let corrected = crate::learn::corrected_on(c, day)?;
    Ok(!still_ask_in(c, &corrected, vec![key.to_string()])?.is_empty())
}

/// The part of the item chat's first note about the task context of `key`:
/// the saved notes, and the question when the newest brief asks about the
/// item. The user's answer then makes sense to the AI.
pub fn for_chat(dir: &Path, key: &str) -> anyhow::Result<String> {
    for_chat_on(dir, key, &crate::plan::today())
}

/// for_chat with the brief of `day`.
pub(crate) fn for_chat_on(dir: &Path, key: &str, day: &str) -> anyhow::Result<String> {
    let key = key.trim();
    db::read(dir, |c| {
        if let Some(n) = get_in(c, key)? {
            return Ok(format!(
                "[Task context: your notes about this item, from the user's own answers. Change them with set_item_context when the user gives new facts.]\n{}\n",
                n.text
            ));
        }
        let mut out = "[This item has no task context yet. When the user says what it is, how it links to other work, or how urgent it is, save a short summary with set_item_context.]\n".to_string();
        if asked_in(c, day, key)? {
            out.push_str(&format!(
                "[On the home page, AIfred asked the user about this item: \"{QUESTION}\" The user's message can be the answer.]\n"
            ));
        }
        Ok(out)
    })
}

/// The question of the home page about one item. The item chat gets it too.
pub const QUESTION: &str =
    "What is this? Is it part of other work? Is it urgent, and by when must it be done?";

/// The items of an overview (overview.rs) that AIfred cannot rank, at most
/// MAX_ASK: a short title, no description (summary), no due day, no
/// parent, no note, and no goal in the brief (`goal`). An item that is the
/// parent of other items, or a GitHub item with linked work (`linked`), is
/// left out too: the brief can place it from that work. AIfred's own tasks
/// and emails are left out: AIfred knows what its tasks are, and an email
/// has its text. The items that the brief names (`why`) come first: they
/// need the user today, so a good rank matters most for them.
pub fn to_ask(
    overview: &Value,
    goal: &BTreeMap<String, i64>,
    why: &BTreeMap<String, String>,
    skipped: &HashSet<String>,
) -> Vec<String> {
    let items = |lists: &[&'static str]| -> Vec<&Value> {
        lists
            .iter()
            .flat_map(|p| overview.pointer(p).and_then(Value::as_array))
            .flatten()
            .collect()
    };
    let asked = items(&["/tasks", "/github/items", "/clickup/tasks"]);
    // The items that are the parent of another item. Any item can be a child.
    let parents: HashSet<&str> = items(&[
        "/tasks",
        "/github/items",
        "/clickup/tasks",
        "/email/needs_reply",
    ])
    .into_iter()
    .filter_map(|i| i["parent"].as_str())
    .collect();
    let mut keys: Vec<String> = asked
        .into_iter()
        .filter(|i| i["doer"].as_str().unwrap_or("you") == "you")
        .filter(|i| !i["key"].as_str().is_some_and(|k| parents.contains(k)))
        .filter(|i| {
            ["summary", "due", "parent", "context", "linked"]
                .iter()
                .all(|f| i.get(*f).is_none_or(Value::is_null))
        })
        .filter(|i| {
            let words = i["title"]
                .as_str()
                .unwrap_or_default()
                .split_whitespace()
                .count();
            (1..=SHORT_TITLE_WORDS).contains(&words)
        })
        .filter_map(|i| i["key"].as_str().map(String::from))
        .filter(|k| !goal.contains_key(k) && !skipped.contains(k))
        .collect();
    // A stable sort: the items in `why` first, else the overview order.
    // Each key is in one list only, so no key comes two times.
    keys.sort_by_key(|k| !why.contains_key(k));
    keys.truncate(MAX_ASK);
    keys
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-context-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_note_is_saved_cut_cleared_and_put_back() {
        let d = temp("set");
        assert_eq!(
            set(&d, "task:t1", "Fix the sync bug.", "aifred", 5)
                .unwrap()
                .before,
            None
        );
        let saved = get(&d, "task:t1").unwrap().unwrap();
        assert_eq!(
            (saved.text.as_str(), saved.by.as_str(), saved.updated),
            ("Fix the sync bug.", "aifred", 5)
        );
        let long = "word ".repeat(300);
        let s = set(&d, "task:t1", &long, "you", 6).unwrap();
        assert_eq!(
            s.before,
            Some(saved.clone()),
            "set gives the note from before"
        );
        let cut = get(&d, "task:t1").unwrap().unwrap().text;
        assert_eq!(cut.chars().count(), MAX_CHARS);
        assert!(cut.ends_with('…'));
        assert!(s.cut, "set says that it cut the text");
        assert_eq!(s.text, cut, "set gives the saved text");
        assert!(!set(&d, " task:t1 ", "Short.", "you", 6).unwrap().cut);
        assert_eq!(
            get(&d, "task:t1").unwrap().unwrap().text,
            "Short.",
            "spaces around the key do not make a second key"
        );
        set(&d, "task:t1", "  ", "you", 7).unwrap();
        assert_eq!(get(&d, "task:t1").unwrap(), None, "an empty text clears it");
        restore(&d, "task:t1", Some(&saved)).unwrap();
        assert_eq!(
            get(&d, "task:t1").unwrap(),
            Some(saved.clone()),
            "the Undo puts it back as it was"
        );
        restore(&d, "task:t1", None).unwrap();
        assert_eq!(get(&d, "task:t1").unwrap(), None);
        let bob = ItemContext {
            by: "bob".into(),
            ..saved
        };
        assert!(
            restore(&d, "task:t1", Some(&bob)).is_err(),
            "the Undo keeps only \"you\" or \"aifred\""
        );
        assert_eq!(get(&d, "task:t1").unwrap(), None);
        assert!(set(&d, "jira:1", "x", "you", 1).is_err());
        assert!(set(&d, "task:t1", "x", "bob", 1).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_unclear_items_are_asked_about_and_the_brief_items_first() {
        let o = json!({
            "tasks": [
                {"key": "task:top", "doer": "you", "title": "Release"},
                {"key": "task:a", "doer": "you", "title": "Fix sync"},
                {"key": "task:long", "doer": "you", "title": "Write the release notes for version 2"},
                {"key": "task:due", "doer": "you", "title": "Pay", "due": "2026-11-09"},
                {"key": "task:sum", "doer": "you", "title": "Pay", "summary": "Rent for May."},
                {"key": "task:sub", "doer": "you", "title": "Pay", "parent": "clickup:zz"},
                {"key": "task:ctx", "doer": "you", "title": "Pay", "context": "Rent."},
                {"key": "task:goal", "doer": "you", "title": "Pay"},
                {"key": "task:skip", "doer": "you", "title": "Pay"},
                {"job": "j1", "doer": "aifred", "title": "Check"},
                {"key": "task:b", "doer": "you", "title": "Call Bo"},
                {"key": "task:kid", "doer": "you", "title": "Write the release notes for 2", "parent": "task:top"}
            ],
            "github": {"items": [
                {"key": "github:a/b#8", "title": "Sync", "linked": ["a/b#9"]},
                {"key": "github:a/b#7", "title": "Sync"}
            ]},
            "clickup": {"tasks": [{"key": "clickup:c1", "title": "Docs"}]},
            "email": {"needs_reply": [{"key": "gmail:m1", "title": "Hi"}]}
        });
        let goal = BTreeMap::from([("task:goal".to_string(), 1)]);
        let why = BTreeMap::from([("github:a/b#7".to_string(), "Late.".to_string())]);
        let skipped = HashSet::from(["task:skip".to_string()]);
        assert_eq!(
            to_ask(&o, &goal, &why, &skipped),
            ["github:a/b#7", "task:a", "task:b"]
        );
        assert_eq!(
            to_ask(&o, &goal, &BTreeMap::new(), &skipped),
            ["task:a", "task:b", "github:a/b#7"]
        );
    }

    #[test]
    fn a_skip_stops_the_question_and_its_undo_asks_again() {
        let d = temp("skip");
        skip(&d, "task:t1", false, 5).unwrap();
        skip(&d, "task:t1", false, 6).unwrap();
        db::read(&d, |c| {
            assert_eq!(skipped_in(c)?, HashSet::from(["task:t1".to_string()]));
            Ok(())
        })
        .unwrap();
        skip(&d, "task:t1", true, 7).unwrap();
        db::read(&d, |c| {
            assert!(skipped_in(c)?.is_empty());
            Ok(())
        })
        .unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}
