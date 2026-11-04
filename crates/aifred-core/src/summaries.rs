//! Short summaries of long descriptions: of GitHub items, ClickUp tasks, and
//! local tasks. The overview tool (overview.rs) sends a summary for each item,
//! so the AI can see all your work without reading every description.
//!
//! - A description of SHORT_CHARS or less is its own summary. It costs no AI call.
//! - A longer one gets a summary from the AI, in groups of BATCH items. One
//!   group is one AI call on your Claude login, with the fast MODEL.
//! - A summary goes back to NULL when its title or description changes (a
//!   trigger in db.rs, SCHEMA_5). The next run makes it again.
//! - Only open items get a summary: items in the list, and tasks not done.

use std::collections::HashSet;
use std::path::Path;

use anyhow::Context;
use serde_json::Value;

use crate::db;

/// A description this short is its own summary.
pub const SHORT_CHARS: usize = 300;
/// The items in one AI call.
pub const BATCH: usize = 15;
/// The most AI calls in one run. A first run with many long descriptions
/// makes the rest in the next runs.
pub const MAX_CALLS: usize = 5;
/// The fast model: a summary does not need the most careful model.
pub const MODEL: &str = "haiku";
/// Each description goes into the AI call with at most this many characters.
const INPUT_CHARS: usize = 3000;

pub const RULES: &str = "\
You write short summaries of work items: GitHub issues and pull requests, ClickUp tasks, and to-do tasks.
- For each item, write one or two sentences, at most 40 words.
- Say what must be done, and any blocker, deadline, or person that matters.
- Leave out templates, checklists, greetings, and links.
- Do not add facts that are not in the text.
- Answer with only a JSON object. Each key is the item number, and each value is the summary.";

/// One item that needs a summary.
#[derive(Debug, Clone)]
pub struct Pending {
    /// "work_item" or "task".
    table: &'static str,
    rowid: i64,
    title: String,
    /// The description that the summary is of.
    text: String,
}

/// The key of an item: its table and its row number.
pub type Key = (&'static str, i64);

impl Pending {
    pub fn key(&self) -> Key {
        (self.table, self.rowid)
    }
}

/// Up to `limit` open items with a long description and no summary, in a
/// fixed order. It does not give the items in `skip`: the AI gave no usable
/// summary for them in this run of the app, so a new try costs a call for nothing.
pub fn pending(dir: &Path, limit: usize, skip: &HashSet<Key>) -> anyhow::Result<Vec<Pending>> {
    // Row numbers are whole numbers, so they are safe to put into the SQL text.
    let not_in = |table: &str| {
        let ids: Vec<String> = skip.iter().filter(|k| k.0 == table).map(|k| k.1.to_string()).collect();
        if ids.is_empty() { String::new() } else { format!(" AND rowid NOT IN ({})", ids.join(",")) }
    };
    let sql = format!(
        "SELECT 'work_item', rowid, title, details FROM work_item
           WHERE in_list = 1 AND summary IS NULL AND length(details) > ?1{}
         UNION ALL
         SELECT 'task', rowid, title, notes FROM task
           WHERE done = 0 AND summary IS NULL AND length(notes) > ?1{}
         ORDER BY 1, 2",
        not_in("work_item"),
        not_in("task")
    );
    db::read(dir, |c| {
        let mut q = c.prepare(&sql)?;
        let rows = q.query_map(rusqlite::params![SHORT_CHARS as i64], |r| {
            let table: String = r.get(0)?;
            Ok(Pending {
                table: if table == "task" { "task" } else { "work_item" },
                rowid: r.get(1)?,
                title: r.get(2)?,
                text: r.get(3)?,
            })
        })?;
        // The SQL counts spaces and line breaks too. The display (for_item)
        // joins the words first, so count the same way: a text that shows in
        // full needs no AI call.
        let mut out = Vec::new();
        for row in rows {
            let item = row?;
            if !is_short(&item.text) {
                out.push(item);
                if out.len() == limit {
                    break;
                }
            }
        }
        Ok(out)
    })
}

/// The text with each run of spaces and line breaks made into one space.
fn words(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True when the display shows the whole text, so it needs no summary.
fn is_short(text: &str) -> bool {
    words(text).chars().count() <= SHORT_CHARS
}

/// The text of one AI call. The items have numbers from 1.
pub fn prompt(items: &[Pending]) -> String {
    let mut p = String::from("Write a summary of each item.\n");
    for (i, item) in items.iter().enumerate() {
        let text: String = item.text.chars().take(INPUT_CHARS).collect();
        p.push_str(&format!("\n### Item {}\nTitle: {}\n{}\n", i + 1, item.title, text.trim()));
    }
    p
}

/// Reads the answer of the AI: one summary for each item number. An item
/// with no usable summary gets None. Fails only when there is no JSON object.
pub fn parse(answer: &str, count: usize) -> anyhow::Result<Vec<Option<String>>> {
    // The model can put the object in a code block or add text around it.
    let start = answer.find('{').context("the summary answer has no JSON object")?;
    let end = answer.rfind('}').filter(|&e| e > start).context("the summary answer has no JSON object")?;
    let v: Value = serde_json::from_str(answer.get(start..=end).unwrap_or_default())
        .context("the summary answer is not a JSON object")?;
    Ok((1..=count)
        .map(|n| {
            let s = v[n.to_string()].as_str()?.split_whitespace().collect::<Vec<_>>().join(" ");
            (!s.is_empty()).then(|| s.chars().take(SHORT_CHARS * 2).collect())
        })
        .collect())
}

/// Saves the summaries. A summary is saved only when the title and the
/// description did not change while the AI wrote it. Returns how many it saved.
pub fn save(dir: &Path, items: &[Pending], summaries: &[Option<String>]) -> anyhow::Result<usize> {
    db::write(dir, |tx| {
        let mut saved = 0;
        for (item, summary) in items.iter().zip(summaries) {
            let Some(summary) = summary else { continue };
            let text_column = if item.table == "task" { "notes" } else { "details" };
            saved += tx.execute(
                &format!(
                    "UPDATE {} SET summary = ?1 WHERE rowid = ?2 AND title = ?3 AND {text_column} = ?4",
                    item.table
                ),
                rusqlite::params![summary, item.rowid, item.title, item.text],
            )?;
        }
        Ok(saved)
    })
}

/// The summary to show for an item: the saved summary, or else the start of
/// the description. None when there is no description.
pub fn for_item(summary: Option<String>, text: &str) -> Option<String> {
    if let Some(s) = summary {
        return Some(s);
    }
    let words = words(text);
    if words.is_empty() {
        return None;
    }
    if words.chars().count() <= SHORT_CHARS {
        return Some(words);
    }
    let cut: String = words.chars().take(SHORT_CHARS).collect();
    Some(format!("{cut}…"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::{TaskFields, TaskStore};

    fn notes(text: &str) -> TaskFields {
        TaskFields { notes: Some(text.into()), ..Default::default() }
    }

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-summaries-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn only_open_items_with_a_long_description_need_a_summary() {
        let d = temp("pending");
        let s = TaskStore::new(&d);
        s.add("Short", "Call Sam before 5pm.", "you").unwrap();
        let long = s.add("Long", &"Plan the release. ".repeat(30), "you").unwrap();
        let done = s.add("Done", &"Old work. ".repeat(50), "you").unwrap();
        s.set_done(&done.id, true).unwrap();
        let p = pending(&d, 10, &HashSet::new()).unwrap();
        assert_eq!(p.len(), 1, "short and done tasks cost no AI call");
        assert_eq!(p[0].title, "Long");

        assert_eq!(save(&d, &p, &[Some("Plan the release.".into())]).unwrap(), 1);
        assert!(pending(&d, 10, &HashSet::new()).unwrap().is_empty());

        // A new due date or tag keeps the summary: it is only of the description.
        s.update(&long.id, TaskFields { due: Some("2026-12-01".into()), tags: Some(vec!["work".into()]), ..Default::default() }).unwrap();
        assert!(pending(&d, 10, &HashSet::new()).unwrap().is_empty());

        // A new description makes the summary old: the item needs a new one.
        s.update(&long.id, notes(&"Plan the next release. ".repeat(30))).unwrap();
        assert_eq!(pending(&d, 10, &HashSet::new()).unwrap().len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_summary_of_a_description_that_changed_meanwhile_is_not_saved() {
        let d = temp("changed");
        let s = TaskStore::new(&d);
        let t = s.add("Long", &"Plan the release. ".repeat(30), "you").unwrap();
        let p = pending(&d, 10, &HashSet::new()).unwrap();
        s.update(&t.id, notes(&"Something else. ".repeat(30))).unwrap();
        assert_eq!(save(&d, &p, &[Some("Plan the release.".into())]).unwrap(), 0);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_answer_can_have_a_code_block_and_missing_items() {
        let got = parse("Here:\n```json\n{\"1\": \"  Fix the   cart. \", \"3\": \"\"}\n```", 3).unwrap();
        assert_eq!(got, vec![Some("Fix the cart.".into()), None, None]);
        assert!(parse("no json here", 1).is_err());
        assert!(parse("} then {", 1).is_err(), "a closing brace before the opening brace is not an object");
    }

    #[test]
    fn spaces_and_line_breaks_do_not_make_a_description_long() {
        let d = temp("spaces");
        let s = TaskStore::new(&d);
        // 350 characters, but only 175 after the display joins the words.
        s.add("Lines", &"word\n\n ".repeat(50), "you").unwrap();
        assert!(pending(&d, 10, &HashSet::new()).unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn skipped_items_do_not_block_the_other_items() {
        let d = temp("skip");
        let s = TaskStore::new(&d);
        s.add("First", &"Plan the release. ".repeat(30), "you").unwrap();
        s.add("Second", &"Fix the cart. ".repeat(30), "you").unwrap();
        let first = pending(&d, 1, &HashSet::new()).unwrap();
        assert_eq!(first[0].title, "First", "the order is fixed");
        let skip: HashSet<Key> = first.iter().map(|i| i.key()).collect();
        let next = pending(&d, 1, &skip).unwrap();
        assert_eq!(next[0].title, "Second");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_prompt_numbers_the_items_and_cuts_long_text() {
        let items = vec![
            Pending { table: "task", rowid: 1, title: "A".into(), text: "x".repeat(10_000) },
            Pending { table: "work_item", rowid: 2, title: "B".into(), text: "short".into() },
        ];
        let p = prompt(&items);
        assert!(p.contains("### Item 1\nTitle: A") && p.contains("### Item 2\nTitle: B"));
        assert!(p.len() < 3500);
    }

    #[test]
    fn an_item_with_no_summary_shows_the_start_of_its_description() {
        assert_eq!(for_item(Some("S".into()), "long text"), Some("S".into()));
        assert_eq!(for_item(None, "  a\n b "), Some("a b".into()));
        assert_eq!(for_item(None, ""), None);
        let long = for_item(None, &"word ".repeat(200)).unwrap();
        assert!(long.ends_with('…') && long.chars().count() == SHORT_CHARS + 1);
    }
}
