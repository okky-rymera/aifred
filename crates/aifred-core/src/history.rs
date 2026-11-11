//! Conversation history that aifred owns, in the database (db.rs).
//! It does not depend on any backend, so a backend switch keeps the history.
//! "New conversation" ends the current conversation. The old messages stay.

use std::path::{Path, PathBuf};

use rusqlite::{OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::files::now;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub ts: u64,
    /// "user" or "assistant".
    pub role: String,
    pub text: String,
    pub backend: String,
    /// The files that the user sent with the message (uploads.rs).
    #[serde(default)]
    pub files: Vec<crate::uploads::FileRef>,
}

pub struct History {
    dir: PathBuf,
}

impl History {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    /// The messages of the current conversation, oldest first.
    pub fn load(&self) -> anyhow::Result<Vec<Turn>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(
                "SELECT t.ts, t.role, t.text, t.backend, t.files FROM turn t
                 JOIN conversation c ON c.id = t.conversation_id
                 WHERE c.ended IS NULL ORDER BY t.id",
            )?;
            let rows = q.query_map([], |r| {
                let files: String = r.get(4)?;
                Ok(Turn {
                    ts: r.get(0)?,
                    role: r.get(1)?,
                    text: r.get(2)?,
                    backend: r.get(3)?,
                    files: serde_json::from_str(&files).unwrap_or_default(),
                })
            })?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// The files of the messages of the current conversation, oldest first.
    /// It reads only the messages that have files.
    pub fn current_files(&self) -> anyhow::Result<Vec<crate::uploads::FileRef>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(
                "SELECT t.files FROM turn t
                 JOIN conversation c ON c.id = t.conversation_id
                 WHERE c.ended IS NULL AND t.files != '[]' ORDER BY t.id",
            )?;
            let rows = q.query_map([], |r| r.get::<_, String>(0))?;
            let mut files = vec![];
            for r in rows {
                files.extend(
                    serde_json::from_str::<Vec<crate::uploads::FileRef>>(&r?).unwrap_or_default(),
                );
            }
            Ok(files)
        })
    }

    pub fn append(&self, role: &str, text: &str, backend: &str) -> anyhow::Result<()> {
        self.append_with_files(role, text, backend, &[])
    }

    /// Like append, with the files that the user sent with the message.
    pub fn append_with_files(
        &self,
        role: &str,
        text: &str,
        backend: &str,
        files: &[crate::uploads::FileRef],
    ) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            let turn = Turn {
                ts: now(),
                role: role.into(),
                text: text.into(),
                backend: backend.into(),
                files: files.to_vec(),
            };
            let id = current_conversation(tx, turn.ts)?;
            insert_turn(tx, id, &turn)?;
            // Not in insert_turn: the import of old files runs before the files column exists.
            if !files.is_empty() {
                tx.execute(
                    "UPDATE turn SET files = ?1 WHERE id = ?2",
                    rusqlite::params![serde_json::to_string(files)?, tx.last_insert_rowid()],
                )?;
            }
            Ok(())
        })
    }

    /// Starts a new conversation. The old one is kept. Returns the id of the
    /// conversation that ended, or None when there was none.
    pub fn archive(&self) -> anyhow::Result<Option<i64>> {
        db::write(&self.dir, |tx| {
            let id: Option<i64> = tx
                .query_row("SELECT id FROM conversation WHERE ended IS NULL", [], |r| {
                    r.get(0)
                })
                .optional()?;
            tx.execute(
                "UPDATE conversation SET ended = ?1 WHERE ended IS NULL",
                [now() as i64],
            )?;
            Ok(id)
        })
    }

    /// Ends the current conversation when it started on a day before
    /// `today` ("YYYY-MM-DD", local). Returns the id of the conversation
    /// that ended. The chat starts fresh each day (assistant.rs).
    pub fn archive_if_started_before(&self, today: &str) -> anyhow::Result<Option<i64>> {
        db::write(&self.dir, |tx| {
            let id: Option<i64> = tx
                .query_row(
                    "SELECT id FROM conversation WHERE ended IS NULL AND date(started, 'unixepoch', 'localtime') < ?1",
                    [today],
                    |r| r.get(0),
                )
                .optional()?;
            if let Some(id) = id {
                tx.execute(
                    "UPDATE conversation SET ended = ?1 WHERE id = ?2",
                    rusqlite::params![now() as i64, id],
                )?;
            }
            Ok(id)
        })
    }

    /// The id of the current conversation, or None before the first message.
    pub fn current(&self) -> anyhow::Result<Option<i64>> {
        db::read(&self.dir, |c| {
            Ok(
                c.query_row("SELECT id FROM conversation WHERE ended IS NULL", [], |r| {
                    r.get(0)
                })
                .optional()?,
            )
        })
    }

    /// The summary of the current conversation, if it has one.
    pub fn current_summary(&self) -> anyhow::Result<Option<String>> {
        db::read(&self.dir, |c| {
            Ok(c.query_row(
                "SELECT summary FROM conversation WHERE ended IS NULL",
                [],
                |r| r.get(0),
            )
            .optional()?
            .flatten())
        })
    }

    /// What a new summary of `conversation` needs: the old summary, and the
    /// messages after it, oldest first, with the id of the last one.
    pub fn unsummarized(&self, conversation: i64) -> anyhow::Result<Unsummarized> {
        db::read(&self.dir, |c| {
            let (summary, upto): (Option<String>, Option<i64>) = c.query_row(
                "SELECT summary, summary_upto FROM conversation WHERE id = ?1",
                [conversation],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?;
            let mut q = c.prepare(
                "SELECT id, ts, role, text, backend, files FROM turn WHERE conversation_id = ?1 AND id > ?2 ORDER BY id",
            )?;
            let rows = q.query_map([conversation, upto.unwrap_or(0)], |r| {
                let files: String = r.get(5)?;
                let files = serde_json::from_str(&files).unwrap_or_default();
                Ok((
                    r.get::<_, i64>(0)?,
                    Turn {
                        ts: r.get(1)?,
                        role: r.get(2)?,
                        text: r.get(3)?,
                        backend: r.get(4)?,
                        files,
                    },
                ))
            })?;
            let rows: Vec<(i64, Turn)> = rows.collect::<Result<_, _>>()?;
            let (ids, turns) = rows.into_iter().unzip();
            Ok(Unsummarized {
                summary,
                turns,
                ids,
            })
        })
    }

    /// Saves a summary of `conversation` that covers the messages up to turn `upto`.
    pub fn set_summary(&self, conversation: i64, summary: &str, upto: i64) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute(
                "UPDATE conversation SET summary = ?1, summary_upto = ?2 WHERE id = ?3",
                rusqlite::params![summary, upto, conversation],
            )?;
            Ok(())
        })
    }
}

/// The part of a conversation that its summary does not cover yet.
#[derive(Debug)]
pub struct Unsummarized {
    pub summary: Option<String>,
    pub turns: Vec<Turn>,
    /// The turn id of each item in `turns`.
    pub ids: Vec<i64>,
}

/// One message that a search found, with the message before and after it.
#[derive(Debug, Serialize)]
pub struct HistoryHit {
    pub conversation: i64,
    /// True when the message is in the current conversation.
    pub current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub before: Option<Turn>,
    pub hit: Turn,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub after: Option<Turn>,
}

impl History {
    /// Messages from all conversations that match `query` by words or by
    /// meaning, best match first.
    pub fn search(&self, query: &str, limit: Option<u32>) -> anyhow::Result<Vec<HistoryHit>> {
        db::read(&self.dir, |c| {
            let ids = db::hybrid(
                c,
                &self.dir,
                query,
                "turn_vec",
                "SELECT f.rowid FROM turn_fts f WHERE turn_fts MATCH ?1 ORDER BY bm25(turn_fts)",
                "SELECT 1 FROM turn WHERE id = ?1",
                &[],
                limit,
            )?;
            let mut get = c.prepare(
                "SELECT t.id, t.conversation_id, c.ended IS NULL, t.ts, t.role, t.text, t.backend
                 FROM turn t JOIN conversation c ON c.id = t.conversation_id WHERE t.id = ?1",
            )?;
            let found = ids
                .iter()
                .map(|id| {
                    get.query_row([id], |r| {
                        let turn = Turn {
                            ts: r.get(3)?,
                            role: r.get(4)?,
                            text: r.get(5)?,
                            backend: r.get(6)?,
                            files: vec![],
                        };
                        Ok((
                            r.get::<_, i64>(0)?,
                            r.get::<_, i64>(1)?,
                            r.get::<_, bool>(2)?,
                            turn,
                        ))
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;
            let near = |conversation: i64, id: i64, sql: &str| -> rusqlite::Result<Option<Turn>> {
                c.query_row(sql, [conversation, id], |r| {
                    Ok(Turn {
                        ts: r.get(0)?,
                        role: r.get(1)?,
                        text: r.get(2)?,
                        backend: r.get(3)?,
                        files: vec![],
                    })
                })
                .optional()
            };
            found
                .into_iter()
                .map(|(id, conversation, current, hit)| {
                    Ok(HistoryHit {
                        conversation,
                        current,
                        before: near(conversation, id, "SELECT ts, role, text, backend FROM turn WHERE conversation_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT 1")?,
                        hit,
                        after: near(conversation, id, "SELECT ts, role, text, backend FROM turn WHERE conversation_id = ?1 AND id > ?2 ORDER BY id LIMIT 1")?,
                    })
                })
                .collect()
        })
    }
}

/// The id of the current conversation. Makes one if there is none.
fn current_conversation(tx: &Transaction, now: u64) -> anyhow::Result<i64> {
    let id = tx
        .query_row("SELECT id FROM conversation WHERE ended IS NULL", [], |r| {
            r.get(0)
        })
        .optional()?;
    match id {
        Some(id) => Ok(id),
        None => {
            tx.execute(
                "INSERT INTO conversation (started) VALUES (?1)",
                [now as i64],
            )?;
            Ok(tx.last_insert_rowid())
        }
    }
}

/// Adds a conversation with these messages. `ended` None makes it the
/// current conversation. The import of the old files uses it.
pub(crate) fn insert_conversation(
    tx: &Transaction,
    turns: &[Turn],
    ended: Option<u64>,
    now: u64,
) -> anyhow::Result<()> {
    let started = turns.first().map_or(ended.unwrap_or(now), |t| t.ts);
    tx.execute(
        "INSERT INTO conversation (started, ended) VALUES (?1, ?2)",
        rusqlite::params![started as i64, ended.map(|e| e as i64)],
    )?;
    let id = tx.last_insert_rowid();
    turns.iter().try_for_each(|t| insert_turn(tx, id, t))
}

fn insert_turn(tx: &Transaction, conversation: i64, t: &Turn) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO turn (conversation_id, ts, role, text, backend) VALUES (?1, ?2, ?3, ?4, ?5)",
        rusqlite::params![conversation, t.ts as i64, t.role, t.text, t.backend],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::db::tests::temp;

    #[test]
    fn append_load_and_new_conversation() {
        let d = temp("history");
        let h = History::new(&d);
        assert!(h.load().unwrap().is_empty());
        h.archive().unwrap(); // No error without a conversation.
        h.append("user", "hi", "claude").unwrap();
        h.append("assistant", "hello", "claude").unwrap();
        let t = h.load().unwrap();
        assert_eq!(
            t.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            vec!["hi", "hello"]
        );
        assert_eq!(t[1].role, "assistant");
        h.archive().unwrap();
        assert!(
            h.load().unwrap().is_empty(),
            "a new conversation starts empty"
        );
        h.append("user", "again", "claude").unwrap();
        assert_eq!(h.load().unwrap().len(), 1);
        let (ended, turns): (i64, i64) = db::read(&d, |c| {
            Ok(c.query_row(
                "SELECT (SELECT count(*) FROM conversation WHERE ended IS NOT NULL), (SELECT count(*) FROM turn)",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?)
        })
        .unwrap();
        assert_eq!((ended, turns), (1, 3), "the old conversation is kept");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn summaries_cover_the_messages_up_to_a_turn_and_are_searchable() {
        let d = temp("history-summary");
        let h = History::new(&d);
        assert_eq!(h.archive().unwrap(), None, "no conversation yet");
        h.append("user", "Plan the Bali trip", "claude").unwrap();
        h.append("assistant", "Flights first.", "claude").unwrap();
        let id = h.current().unwrap().unwrap();
        let u = h.unsummarized(id).unwrap();
        assert_eq!((u.summary, u.turns.len()), (None, 2));
        h.set_summary(
            id,
            "- The user plans a holiday in Bali.",
            *u.ids.last().unwrap(),
        )
        .unwrap();
        assert_eq!(
            h.current_summary().unwrap().as_deref(),
            Some("- The user plans a holiday in Bali.")
        );
        h.append("user", "Book a hotel too", "claude").unwrap();
        let u = h.unsummarized(id).unwrap();
        assert_eq!(
            u.turns.iter().map(|t| t.text.as_str()).collect::<Vec<_>>(),
            vec!["Book a hotel too"]
        );
        assert!(u.summary.is_some());
        assert_eq!(h.archive().unwrap(), Some(id));
        assert_eq!(h.current_summary().unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_conversation_from_an_earlier_day_ends_at_the_next_message() {
        let d = temp("history-day");
        let h = History::new(&d);
        assert_eq!(h.archive_if_started_before("2026-11-11").unwrap(), None);
        h.append("user", "Hi", "claude").unwrap();
        let id = h.current().unwrap().unwrap();
        let day = chrono::Local::now().format("%F").to_string();
        assert_eq!(
            h.archive_if_started_before(&day).unwrap(),
            None,
            "started today"
        );
        assert_eq!(h.archive_if_started_before("9999-01-01").unwrap(), Some(id));
        assert_eq!(h.current().unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn search_finds_messages_in_old_conversations_with_their_neighbours() {
        let d = temp("history-search");
        let h = History::new(&d);
        h.append("user", "Remind me about the dentist", "claude")
            .unwrap();
        h.append("assistant", "Noted: dentist on Tuesday.", "claude")
            .unwrap();
        h.append("user", "Thanks", "claude").unwrap();
        h.archive().unwrap();
        h.append("user", "What did we say about the dentists?", "claude")
            .unwrap();
        let hits = h.search("dentist appointment", None).unwrap();
        assert_eq!(hits.len(), 3);
        let old = hits.iter().find(|x| x.hit.role == "assistant").unwrap();
        assert!(!old.current);
        assert_eq!(
            old.before.as_ref().unwrap().text,
            "Remind me about the dentist"
        );
        assert_eq!(old.after.as_ref().unwrap().text, "Thanks");
        assert!(
            hits.iter().any(|x| x.current),
            "the current conversation is found too"
        );
        assert!(h.search("?", None).unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
