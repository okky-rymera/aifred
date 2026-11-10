//! What AIfred knows about the user. Each fact is one row in the database.
//!
//! - "Core" facts go into the AI instructions with each message.
//! - The AI finds the other facts with a search (the memory tool server, aifred-tools).
//! - A fact can have tags, for example "testing" and "people". The AI can
//!   read all facts with one tag (facts_with_tag), and sees the tag names
//!   with memory_tags.
//! - A forgotten fact is not deleted. It gets a "forgotten" time, and the
//!   user can restore it in the app.
//!
//! The import of the old memory.md uses set_text: each line that starts
//! with "- " becomes one fact, and each block of other lines becomes one fact.

use std::path::{Path, PathBuf};

use anyhow::bail;
use rusqlite::{Connection, Row, Transaction};
use serde::Serialize;

use crate::db;
use crate::files::now;

#[derive(Debug, Clone, PartialEq)]
enum Style {
    Item,
    Text,
}

impl Style {
    fn as_str(&self) -> &'static str {
        match self {
            Style::Item => "item",
            Style::Text => "text",
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct Fact {
    pub id: i64,
    pub text: String,
    /// True: the fact goes into the AI instructions with each message.
    pub core: bool,
    /// "you" or "aifred".
    pub added_by: String,
    pub created: u64,
    pub updated: u64,
    /// When the fact was forgotten, in Unix seconds. None for a fact in use.
    pub forgotten: Option<u64>,
    /// Lowercase tags, for example ["testing", "people"]. Can be empty.
    pub tags: Vec<String>,
}

/// One tag and the number of facts in use that have it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TagCount {
    pub tag: String,
    pub facts: u64,
}

/// The most tags on one fact. More tags make the tag list less useful.
pub const MAX_TAGS: usize = 5;
/// The longest tag, in characters.
const MAX_TAG_CHARS: usize = 40;

const COLUMNS: &str = "m.id, m.text, m.core, m.added_by, m.created, m.updated, m.forgotten, m.tags";

fn from_row(r: &Row) -> rusqlite::Result<Fact> {
    Ok(Fact {
        id: r.get(0)?,
        text: r.get(1)?,
        core: r.get(2)?,
        added_by: r.get(3)?,
        created: r.get(4)?,
        updated: r.get(5)?,
        forgotten: r.get(6)?,
        // A bad list counts as no tags: the fact itself stays usable.
        tags: serde_json::from_str(&r.get::<_, String>(7)?).unwrap_or_default(),
    })
}

/// Cleans the tags: no "#" at the start (tasks::clean_tag, as for task
/// tags), lowercase, spaces become "-", and each tag once. "QA Team" and
/// "#qa-team" are the same tag. Unlike task tags, fact tags are lowercase:
/// the AI finds facts by the exact tag name (with_tag).
/// A comma splits a tag in two: the app edits the tags as one line with
/// commas.
/// Empty tags go away. An error for more than MAX_TAGS tags or a tag that
/// is too long.
pub fn clean_tags(tags: &[String]) -> anyhow::Result<Vec<String>> {
    let out = split_tags(tags);
    if let Some(t) = out.iter().find(|t| t.chars().count() > MAX_TAG_CHARS) {
        bail!("the tag \"{t}\" is longer than {MAX_TAG_CHARS} characters");
    }
    anyhow::ensure!(
        out.len() <= MAX_TAGS,
        "a fact can have at most {MAX_TAGS} tags"
    );
    Ok(out)
}

/// Like clean_tags, but with no error: a tag that is too long goes away,
/// and only the first MAX_TAGS tags stay. For a new fact from the AI: bad
/// tags must not stop the save of the fact. The answer shows the tags that
/// stayed.
fn keep_good_tags(tags: &[String]) -> Vec<String> {
    let mut out = split_tags(tags);
    out.retain(|t| t.chars().count() <= MAX_TAG_CHARS);
    out.truncate(MAX_TAGS);
    out
}

/// The cleaned tags of clean_tags, with no limits.
fn split_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags.iter().flat_map(|t| t.split(',')) {
        let t = crate::tasks::clean_tag(t)
            .split_whitespace()
            .collect::<Vec<_>>()
            .join("-")
            .to_lowercase();
        if !t.is_empty() && !out.contains(&t) {
            out.push(t);
        }
    }
    out
}

fn get(c: &Connection, id: i64) -> anyhow::Result<Fact> {
    let mut q = c.prepare(&format!("SELECT {COLUMNS} FROM memory m WHERE m.id = ?1"))?;
    let mut rows = q.query_map([id], from_row)?;
    match rows.next() {
        Some(f) => Ok(f?),
        None => bail!("no fact with id {id}"),
    }
}

/// Adds one fact at the end, in a change that is open.
pub(crate) fn add_in(
    tx: &Transaction,
    text: &str,
    added_by: &str,
    core: bool,
) -> anyhow::Result<Fact> {
    add_tagged_in(tx, text, added_by, core, &[])
}

/// add_in, with tags. Bad tags go away (keep_good_tags): the fact is
/// saved also then.
pub(crate) fn add_tagged_in(
    tx: &Transaction,
    text: &str,
    added_by: &str,
    core: bool,
    tags: &[String],
) -> anyhow::Result<Fact> {
    let text = clean(text)?;
    let tags = serde_json::to_string(&keep_good_tags(tags))?;
    tx.execute(
        "INSERT INTO memory (text, style, pos, core, added_by, created, updated, tags)
         VALUES (?1, 'item', (SELECT COALESCE(MAX(pos), -1) + 1 FROM memory), ?2, ?3, ?4, ?4, ?5)",
        rusqlite::params![text, core, added_by, now() as i64, tags],
    )?;
    get(tx, tx.last_insert_rowid())
}

/// Takes a fact out of use, in a change that is open. A fact that is
/// already forgotten keeps its time.
pub(crate) fn forget_in(tx: &Transaction, id: i64) -> anyhow::Result<()> {
    tx.execute(
        "UPDATE memory SET forgotten = COALESCE(forgotten, ?2), updated = ?2 WHERE id = ?1",
        rusqlite::params![id, now() as i64],
    )?;
    Ok(())
}

/// One line of text: a line break would make two facts in the old text form.
fn clean(text: &str) -> anyhow::Result<String> {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    anyhow::ensure!(!text.is_empty(), "the fact is empty");
    Ok(text)
}

/// A block of lines: spaces at the end of each line and empty lines at the
/// start and the end go away. Empty lines inside the block would split it
/// into two facts in the memory text, so they go away too.
fn clean_block(text: &str) -> anyhow::Result<String> {
    let text = text
        .lines()
        .map(str::trim_end)
        .filter(|l| !l.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    anyhow::ensure!(!text.is_empty(), "the fact is empty");
    Ok(text)
}

pub struct MemoryStore {
    dir: PathBuf,
}

impl MemoryStore {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    /// All facts: the ones in use in their order, then the forgotten ones, newest first.
    pub fn list(&self) -> anyhow::Result<Vec<Fact>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(&format!(
                "SELECT {COLUMNS} FROM memory m ORDER BY m.forgotten IS NOT NULL, m.forgotten DESC, m.pos, m.id"
            ))?;
            Ok(q.query_map([], from_row)?.collect::<Result<_, _>>()?)
        })
    }

    /// The core facts that are in use, as text for the AI instructions.
    pub fn core_text(&self) -> anyhow::Result<String> {
        self.text(true)
    }

    /// All facts that are in use, core or not, as text.
    pub fn all_text(&self) -> anyhow::Result<String> {
        self.text(false)
    }

    fn text(&self, core_only: bool) -> anyhow::Result<String> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(&format!(
                "SELECT style, text FROM memory WHERE forgotten IS NULL {} ORDER BY pos, id",
                if core_only { "AND core = 1" } else { "" }
            ))?;
            let rows = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            let mut lines = Vec::new();
            for row in rows {
                let (style, text) = row?;
                let prefix = if style == Style::Item.as_str() {
                    "- "
                } else {
                    ""
                };
                lines.push(format!("{prefix}{text}"));
            }
            Ok(lines.join("\n"))
        })
    }

    /// The number of facts in use that are not core. The AI must search for them.
    pub fn other_count(&self) -> anyhow::Result<u64> {
        db::read(&self.dir, |c| {
            Ok(c.query_row(
                "SELECT count(*) FROM memory WHERE forgotten IS NULL AND core = 0",
                [],
                |r| r.get(0),
            )?)
        })
    }

    /// The length of all facts in use, in characters.
    pub fn size_chars(&self) -> anyhow::Result<u64> {
        db::read(&self.dir, |c| {
            Ok(c.query_row(
                "SELECT COALESCE(SUM(length(text)), 0) FROM memory WHERE forgotten IS NULL",
                [],
                |r| r.get(0),
            )?)
        })
    }

    /// Adds one fact at the end.
    pub fn add(&self, text: &str, added_by: &str, core: bool) -> anyhow::Result<Fact> {
        db::write(&self.dir, |tx| add_in(tx, text, added_by, core))
    }

    /// Adds one fact at the end, with tags (keep_good_tags).
    pub fn add_tagged(
        &self,
        text: &str,
        added_by: &str,
        core: bool,
        tags: &[String],
    ) -> anyhow::Result<Fact> {
        db::write(&self.dir, |tx| {
            add_tagged_in(tx, text, added_by, core, tags)
        })
    }

    /// Replaces the tags of a fact (clean_tags). An empty list removes them.
    pub fn set_tags(&self, id: i64, tags: &[String]) -> anyhow::Result<Fact> {
        let tags = serde_json::to_string(&clean_tags(tags)?)?;
        self.change(id, "tags = ?2", rusqlite::types::Value::Text(tags))
    }

    /// The tags of the facts in use, with their number of facts. The tag
    /// with the most facts first, then by name.
    pub fn tags(&self) -> anyhow::Result<Vec<TagCount>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(
                "SELECT t.value, count(*) FROM memory m, json_each(m.tags) t
                 WHERE m.forgotten IS NULL AND json_valid(m.tags)
                 GROUP BY t.value ORDER BY count(*) DESC, t.value",
            )?;
            let rows = q.query_map([], |r| {
                Ok(TagCount {
                    tag: r.get(0)?,
                    facts: r.get(1)?,
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// The facts in use that have `tag`, in their order. An error for more
    /// than one tag ("qa, testing"): the answer would silently have the
    /// facts of only one of them.
    pub fn with_tag(&self, tag: &str) -> anyhow::Result<Vec<Fact>> {
        let tag = match split_tags(&[tag.to_string()]).as_slice() {
            [] => bail!("the tag is empty"),
            [one] => one.clone(),
            _ => bail!("give one tag, not a list: \"{tag}\""),
        };
        db::read(&self.dir, |c| {
            let mut q = c.prepare(&format!(
                "SELECT {COLUMNS} FROM memory m
                 WHERE m.forgotten IS NULL AND json_valid(m.tags)
                   AND EXISTS (SELECT 1 FROM json_each(m.tags) t WHERE t.value = ?1)
                 ORDER BY m.pos, m.id"
            ))?;
            Ok(q.query_map([tag], from_row)?.collect::<Result<_, _>>()?)
        })
    }

    /// Changes the text of a fact. A "text" fact (a block from memory.md,
    /// for example a heading and its lines) keeps its line breaks. A list
    /// item stays one line.
    pub fn edit(&self, id: i64, text: &str) -> anyhow::Result<Fact> {
        db::write(&self.dir, |tx| {
            use rusqlite::OptionalExtension;
            let style: Option<String> = tx
                .query_row("SELECT style FROM memory WHERE id = ?1", [id], |r| r.get(0))
                .optional()?;
            let Some(style) = style else {
                bail!("no fact with id {id}")
            };
            let text = if style == Style::Text.as_str() {
                clean_block(text)?
            } else {
                clean(text)?
            };
            tx.execute(
                "UPDATE memory SET text = ?2, updated = ?3 WHERE id = ?1",
                rusqlite::params![id, text, now() as i64],
            )?;
            get(tx, id)
        })
    }

    pub fn set_core(&self, id: i64, core: bool) -> anyhow::Result<Fact> {
        self.change(
            id,
            "core = ?2",
            rusqlite::types::Value::Integer(core.into()),
        )
    }

    /// Takes a fact out of use. It stays in the database, and restore() brings it back.
    pub fn forget(&self, id: i64) -> anyhow::Result<Fact> {
        self.change(
            id,
            "forgotten = COALESCE(forgotten, ?2)",
            rusqlite::types::Value::Integer(now() as i64),
        )
    }

    /// Deletes a forgotten fact for good: there is no restore. A fact in use
    /// must be forgotten first, so one click cannot delete it.
    /// - Triggers delete its word search and meaning search rows (db.rs).
    /// - The FTS "optimize" merges the word search index, so no index page
    ///   keeps the words of the fact.
    /// - VACUUM then writes the database again from the rows that are left.
    ///   So no free page or old copy of a row keeps the text: also not from
    ///   edits before this version. It blocks other writers for a short time
    ///   (about 0.02 seconds for a 7 MB file). A delete is a rare click, so
    ///   that is acceptable.
    /// - The checkpoint copies the new pages from the write-ahead log (the
    ///   `-wal` file) into the main file and empties the log.
    /// - The VACUUM or the checkpoint cannot finish while another program
    ///   uses the database. Then the fact is deleted, but old copies of the
    ///   text stay in the file until a later delete. The answer is false in
    ///   that case. It is also false on an error there: the delete is saved,
    ///   so an error would be wrong.
    /// - The learned row (learn.rs) that points to the fact stays: AIfred
    ///   learns a place only once. The id is never used again (AUTOINCREMENT,
    ///   db.rs), so the row cannot point to another fact.
    ///
    /// Old chat messages that have the text of the fact stay.
    pub fn delete(&self, id: i64) -> anyhow::Result<bool> {
        db::write(&self.dir, |tx| {
            let forgotten: Option<Option<i64>> = {
                use rusqlite::OptionalExtension;
                tx.query_row("SELECT forgotten FROM memory WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?
            };
            match forgotten {
                None => bail!("no fact with id {id}"),
                Some(None) => bail!("fact {id} is in use: forget it first"),
                Some(Some(_)) => {}
            }
            tx.execute("DELETE FROM memory WHERE id = ?1", [id])?;
            tx.execute(
                "INSERT INTO memory_fts (memory_fts) VALUES ('optimize')",
                [],
            )?;
            Ok(())
        })?;
        let clean = || -> anyhow::Result<bool> {
            let c = db::open(&self.dir)?;
            c.execute_batch("VACUUM")?;
            // Column 0 is 1 when another program kept the checkpoint from finishing.
            let busy: i64 = c.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |r| r.get(0))?;
            Ok(busy == 0)
        };
        let cleaned = match clean() {
            Ok(done) => done,
            Err(e) => {
                eprintln!("aifred: fact {id} is deleted, but the clean-up failed: {e:#}");
                false
            }
        };
        if !cleaned {
            eprintln!("aifred: fact {id} is deleted, but the database file is not cleaned yet");
        }
        Ok(cleaned)
    }

    pub fn restore(&self, id: i64) -> anyhow::Result<Fact> {
        self.change(id, "forgotten = ?2", rusqlite::types::Value::Null)
    }

    /// Runs one UPDATE on fact `id` and returns the fact after it.
    fn change(&self, id: i64, set: &str, value: rusqlite::types::Value) -> anyhow::Result<Fact> {
        db::write(&self.dir, |tx| {
            let sql = format!("UPDATE memory SET {set}, updated = ?3 WHERE id = ?1");
            if tx.execute(&sql, rusqlite::params![id, value, now() as i64])? == 0 {
                bail!("no fact with id {id}");
            }
            get(tx, id)
        })
    }

    /// Facts in use that match `query` by words or by meaning, best match first.
    pub fn search(&self, query: &str, limit: Option<u32>) -> anyhow::Result<Vec<Fact>> {
        self.search_where(query, limit, false)
    }

    /// Like search, but only the facts that are not core. The core facts are
    /// already in the AI instructions.
    pub fn search_other(&self, query: &str, limit: Option<u32>) -> anyhow::Result<Vec<Fact>> {
        self.search_where(query, limit, true)
    }

    fn search_where(
        &self,
        query: &str,
        limit: Option<u32>,
        other_only: bool,
    ) -> anyhow::Result<Vec<Fact>> {
        db::read(&self.dir, |c| {
            let ids = db::hybrid(
                c,
                &self.dir,
                query,
                "memory_vec",
                "SELECT m.id FROM memory_fts f JOIN memory m ON m.id = f.rowid
                 WHERE memory_fts MATCH ?1 AND m.forgotten IS NULL AND NOT (?2 AND m.core) ORDER BY bm25(memory_fts)",
                "SELECT 1 FROM memory WHERE id = ?1 AND forgotten IS NULL AND NOT (?2 AND core)",
                &[&other_only],
                limit,
            )?;
            ids.into_iter().map(|id| get(c, id)).collect()
        })
    }

    /// Replaces all facts with `text`. Only the tests use it now.
    #[cfg(test)]
    pub(crate) fn set_text(&self, text: &str) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| set_text(tx, text, "you", now()))
    }
}

/// Splits the memory text into facts. Empty lines between facts are not kept.
fn parse(text: &str) -> Vec<(Style, String)> {
    let mut facts = Vec::new();
    let mut block: Vec<&str> = Vec::new();
    let flush = |block: &mut Vec<&str>, facts: &mut Vec<(Style, String)>| {
        let t = block.join("\n");
        let t = t.trim();
        if !t.is_empty() {
            facts.push((Style::Text, t.to_string()));
        }
        block.clear();
    };
    for line in text.lines() {
        match line.strip_prefix("- ").map(str::trim) {
            Some(item) if !item.is_empty() => {
                flush(&mut block, &mut facts);
                facts.push((Style::Item, item.to_string()));
            }
            _ => block.push(line),
        }
    }
    flush(&mut block, &mut facts);
    facts
}

/// Makes the facts the same as `text`. The import of memory.md also uses it.
pub(crate) fn set_text(
    tx: &Transaction,
    text: &str,
    added_by: &str,
    now: u64,
) -> anyhow::Result<()> {
    let mut old: Vec<(i64, String, String)> = {
        let mut q = tx.prepare(
            "SELECT id, style, text FROM memory WHERE forgotten IS NULL ORDER BY pos, id",
        )?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<Result<_, _>>()?
    };
    let now = now as i64;
    for (pos, (style, fact)) in parse(text).into_iter().enumerate() {
        let same = old
            .iter()
            .position(|(_, s, t)| s == style.as_str() && *t == fact);
        match same {
            Some(i) => {
                let (id, ..) = old.remove(i);
                tx.execute(
                    "UPDATE memory SET pos = ?1 WHERE id = ?2",
                    rusqlite::params![pos as i64, id],
                )?;
            }
            None => {
                tx.execute(
                    "INSERT INTO memory (text, style, pos, added_by, created, updated) VALUES (?1, ?2, ?3, ?4, ?5, ?5)",
                    rusqlite::params![fact, style.as_str(), pos as i64, added_by, now],
                )?;
            }
        }
    }
    for (id, ..) in old {
        tx.execute(
            "UPDATE memory SET forgotten = ?1 WHERE id = ?2",
            rusqlite::params![now, id],
        )?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::temp;

    fn texts(facts: &[Fact]) -> Vec<&str> {
        facts.iter().map(|f| f.text.as_str()).collect()
    }

    #[test]
    fn imported_text_becomes_core_facts() {
        let d = temp("memory-round");
        let m = MemoryStore::new(&d);
        assert_eq!(m.core_text().unwrap(), "");
        m.set_text("# Work\nI work at Rymera.\n\n- I like short answers.\n-  Tea, no sugar. \n")
            .unwrap();
        assert_eq!(
            m.core_text().unwrap(),
            "# Work\nI work at Rymera.\n- I like short answers.\n- Tea, no sugar."
        );
        assert_eq!(
            texts(&m.list().unwrap()),
            vec![
                "# Work\nI work at Rymera.",
                "I like short answers.",
                "Tea, no sugar."
            ]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_save_keeps_the_facts_that_did_not_change() {
        let d = temp("memory-keep");
        let m = MemoryStore::new(&d);
        m.set_text("- a\n- b\n").unwrap();
        let before = m.list().unwrap();
        m.set_text("- c\n- b\n").unwrap();
        assert_eq!(m.core_text().unwrap(), "- c\n- b", "the new order");
        let after = m.list().unwrap();
        assert_eq!(
            texts(&after),
            vec!["c", "b", "a"],
            "a is forgotten, not deleted"
        );
        assert_eq!(after[1].id, before[1].id, "b keeps its id");
        assert!(after[2].forgotten.is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_core_facts_go_into_the_instructions() {
        let d = temp("memory-core");
        let m = MemoryStore::new(&d);
        m.add("  likes\n tea ", "you", true).unwrap();
        let other = m
            .add("The office wifi is slow on Mondays", "aifred", false)
            .unwrap();
        assert_eq!(m.core_text().unwrap(), "- likes tea");
        assert_eq!(m.other_count().unwrap(), 1);
        assert_eq!(other.added_by, "aifred");
        assert_eq!(
            m.all_text().unwrap(),
            "- likes tea\n- The office wifi is slow on Mondays"
        );
        assert_eq!(m.size_chars().unwrap(), 9 + 34);
        assert_eq!(
            m.search_other("tea wifi", None).unwrap().len(),
            1,
            "only the fact that is not core"
        );
        m.set_core(other.id, true).unwrap();
        assert_eq!(
            m.core_text().unwrap(),
            "- likes tea\n- The office wifi is slow on Mondays"
        );
        assert!(m.add("  ", "you", true).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_edit_keeps_the_lines_of_a_text_fact() {
        let d = temp("memory-edit-block");
        let m = MemoryStore::new(&d);
        m.set_text("# Work\nI work at Rymera.\n\n- likes tea")
            .unwrap();
        let facts = m.search("work", None).unwrap();
        let block = facts.iter().find(|f| f.text.starts_with("# Work")).unwrap();
        let edited = m
            .edit(block.id, "# Work  \nI work at Rymera Web Co.\n")
            .unwrap();
        assert_eq!(edited.text, "# Work\nI work at Rymera Web Co.");
        let item = m.search("tea", None).unwrap().remove(0);
        assert_eq!(
            m.edit(item.id, "likes\ngreen tea").unwrap().text,
            "likes green tea",
            "a list item stays one line"
        );
        assert_eq!(
            m.all_text().unwrap(),
            "# Work\nI work at Rymera Web Co.\n- likes green tea"
        );
        assert!(m.edit(block.id, " \n ").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn tags_are_cleaned_counted_and_filtered() {
        let d = temp("memory-tags");
        let m = MemoryStore::new(&d);
        let qa = m
            .add_tagged(
                "Zara is a QA under the user",
                "aifred",
                false,
                &[
                    "People".into(),
                    "#testing".into(),
                    " QA Team ".into(),
                    "people".into(),
                ],
            )
            .unwrap();
        assert_eq!(qa.tags, vec!["people", "testing", "qa-team"]);
        m.add_tagged(
            "Run the tests before a review",
            "you",
            true,
            &["testing".into()],
        )
        .unwrap();
        let old = m
            .add_tagged("The old QA lead was Bo", "you", false, &["testing".into()])
            .unwrap();
        m.add("I like tea", "you", true).unwrap();
        m.forget(old.id).unwrap();
        assert_eq!(
            m.tags().unwrap(),
            vec![
                TagCount {
                    tag: "testing".into(),
                    facts: 2
                },
                TagCount {
                    tag: "people".into(),
                    facts: 1
                },
                TagCount {
                    tag: "qa-team".into(),
                    facts: 1
                },
            ],
            "a forgotten fact does not count"
        );
        assert_eq!(
            texts(&m.with_tag("#Testing").unwrap()),
            vec![
                "Zara is a QA under the user",
                "Run the tests before a review"
            ]
        );
        let changed = m.set_tags(qa.id, &["people".into()]).unwrap();
        assert_eq!(changed.tags, vec!["people"]);
        assert_eq!(m.with_tag("testing").unwrap().len(), 1);
        let six: Vec<String> = (0..6).map(|i| format!("t{i}")).collect();
        assert!(m.set_tags(qa.id, &six).is_err(), "more than MAX_TAGS");
        assert!(m.set_tags(qa.id, &["a".repeat(41)]).is_err());
        assert_eq!(
            m.set_tags(qa.id, &["qa, Testing".into(), "people,".into()])
                .unwrap()
                .tags,
            vec!["qa", "testing", "people"],
            "a comma splits a tag"
        );
        assert!(m.with_tag(" # ").is_err());
        assert!(
            m.with_tag("qa, testing").is_err(),
            "one tag only, so no facts go missing"
        );
        // A new fact is saved also with bad tags: they go away.
        let many: Vec<String> = ["a".repeat(41)]
            .into_iter()
            .chain((0..7).map(|i| format!("t{i}")))
            .collect();
        let saved = m.add_tagged("Bad tags", "aifred", false, &many).unwrap();
        assert_eq!(saved.tags, vec!["t0", "t1", "t2", "t3", "t4"]);
        assert!(m.set_tags(999, &[]).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn delete_removes_a_forgotten_fact_for_good() {
        let d = temp("memory-delete");
        let m = MemoryStore::new(&d);
        let other = m.add("I like tea", "you", true).unwrap();
        let secret = m
            .add_tagged(
                &format!("My old door code is 0815-lion {}", "and more ".repeat(800)),
                "you",
                true,
                &["home".into()],
            )
            .unwrap();
        // An edit leaves the old text in a free part of the file: the long
        // text had its own pages, and they are now free but not cleared.
        m.edit(secret.id, "My door code is 4711-zebra").unwrap();
        // A meaning vector with a byte pattern that the test can find.
        let vector: Vec<u8> = std::iter::repeat_n(1234.5678f32.to_le_bytes(), 384)
            .flatten()
            .collect();
        db::write(&d, |tx| {
            tx.execute(
                "INSERT INTO memory_vec (rowid, embedding) VALUES (?1, ?2)",
                rusqlite::params![secret.id, vector],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(
            m.delete(secret.id).is_err(),
            "a fact in use must be forgotten first"
        );
        m.forget(secret.id).unwrap();
        // The patterns that the test looks for, and a check that finds them
        // in the database files.
        let patterns = [
            ("the text", &b"4711-zebra"[..]),
            ("the old text", &b"0815-lion"[..]),
            ("the vector", &vector[..16]),
        ];
        let in_files = |pattern: &[u8]| {
            ["aifred.db", "aifred.db-wal"].iter().any(|name| {
                std::fs::read(d.join(name))
                    .is_ok_and(|b| b.windows(pattern.len()).any(|w| w == pattern))
            })
        };
        // Before the delete, the files have all patterns. Else the check
        // after the delete would prove nothing.
        db::open(&d)
            .unwrap()
            .execute_batch("PRAGMA wal_checkpoint(TRUNCATE)")
            .unwrap();
        for (what, pattern) in patterns {
            assert!(
                in_files(pattern),
                "{what} is not in the file before the delete"
            );
        }
        assert!(
            m.delete(secret.id).unwrap(),
            "nothing else reads, so the file is cleaned"
        );
        assert_eq!(texts(&m.list().unwrap()), vec!["I like tea"]);
        assert!(m.restore(secret.id).is_err(), "no restore");
        assert!(m.delete(secret.id).is_err(), "no fact with that id");
        assert!(m.search("zebra", None).unwrap().is_empty());
        assert!(m.tags().unwrap().is_empty());
        let rows: i64 = db::read(&d, |c| {
            Ok(c.query_row(
                "SELECT count(*) FROM memory_fts WHERE memory_fts MATCH 'zebra'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
        assert_eq!(rows, 0);
        // The text, the old text, and the vector are not in the database
        // files any more.
        for (what, pattern) in patterns {
            assert!(!in_files(pattern), "{what} is still in the file");
        }
        assert_eq!(m.list().unwrap()[0].id, other.id);
        // The id of the deleted fact is not used again, also when it was
        // the highest id.
        let next = m.add("I like coffee", "you", true).unwrap();
        assert!(next.id > secret.id, "id {} is used again", next.id);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn search_finds_word_forms_and_skips_forgotten_facts() {
        let d = temp("memory-search");
        let m = MemoryStore::new(&d);
        let tea = m
            .add("I drink green tea in the morning", "you", false)
            .unwrap();
        let invoice = m
            .add("Sam sends the invoices on Fridays", "you", false)
            .unwrap();
        m.add("My cat is called Miso", "you", false).unwrap();
        assert_eq!(
            texts(&m.search("When does the invoice come?", None).unwrap()),
            vec![invoice.text.as_str()]
        );
        assert_eq!(
            texts(&m.search("Who is Sa", None).unwrap()),
            vec![invoice.text.as_str()],
            "start of a word"
        );
        assert!(
            m.search("the", None).unwrap().is_empty(),
            "common words only"
        );
        m.forget(tea.id).unwrap();
        assert!(
            m.search("tea", None).unwrap().is_empty(),
            "a forgotten fact is not found"
        );
        m.restore(tea.id).unwrap();
        assert_eq!(m.search("tea", None).unwrap().len(), 1);
        let edited = m.edit(tea.id, "I drink coffee").unwrap();
        assert!(
            m.search("tea", None).unwrap().is_empty(),
            "the search uses the new text"
        );
        assert_eq!(
            texts(&m.search("coffee", None).unwrap()),
            vec![edited.text.as_str()]
        );
        assert!(m.forget(999).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
