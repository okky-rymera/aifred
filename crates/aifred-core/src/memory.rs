//! What AIfred knows about the user. Each fact is one row in the database.
//!
//! - "Core" facts go into the AI instructions with each message.
//! - The AI finds the other facts with a search (the memory tool server, aifred-tools).
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
}

const COLUMNS: &str = "m.id, m.text, m.core, m.added_by, m.created, m.updated, m.forgotten";

fn from_row(r: &Row) -> rusqlite::Result<Fact> {
    Ok(Fact {
        id: r.get(0)?,
        text: r.get(1)?,
        core: r.get(2)?,
        added_by: r.get(3)?,
        created: r.get(4)?,
        updated: r.get(5)?,
        forgotten: r.get(6)?,
    })
}

fn get(c: &Connection, id: i64) -> anyhow::Result<Fact> {
    let mut q = c.prepare(&format!("SELECT {COLUMNS} FROM memory m WHERE m.id = ?1"))?;
    let mut rows = q.query_map([id], from_row)?;
    match rows.next() {
        Some(f) => Ok(f?),
        None => bail!("no fact with id {id}"),
    }
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
        let text = clean(text)?;
        db::write(&self.dir, |tx| {
            tx.execute(
                "INSERT INTO memory (text, style, pos, core, added_by, created, updated)
                 VALUES (?1, 'item', (SELECT COALESCE(MAX(pos), -1) + 1 FROM memory), ?2, ?3, ?4, ?4)",
                rusqlite::params![text, core, added_by, now() as i64],
            )?;
            get(tx, tx.last_insert_rowid())
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
