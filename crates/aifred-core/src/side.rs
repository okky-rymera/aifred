//! Chats apart from the main conversation:
//! - "plan:YYYY-MM-DD": the plan chat of one day,
//! - "item:<item key>": the chat about one item (items.rs).
//!
//! Each message of a side chat sends the earlier messages of the same chat
//! again, so the chats do not mix with each other or with the main conversation.

use std::path::Path;

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::db;
use crate::files::now;
use crate::uploads::FileRef;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SideTurn {
    pub id: i64,
    pub ts: u64,
    /// "user" or "assistant".
    pub role: String,
    pub text: String,
    #[serde(default)]
    pub files: Vec<FileRef>,
    /// For an answer: the time, the model, and the lookups, in one short line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub info: Option<String>,
}

#[derive(Default, Serialize, Deserialize)]
struct Meta {
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    files: Vec<FileRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    info: Option<String>,
}

/// A thread name is "plan:YYYY-MM-DD" or "item:<item key>".
pub fn check(thread: &str) -> anyhow::Result<()> {
    match thread.split_once(':') {
        Some(("plan", day)) => crate::plan::check_day(day),
        Some(("item", key)) => crate::items::split(key).map(|_| ()),
        _ => bail!("\"{thread}\" is not a chat name"),
    }
}

/// The messages of the thread, oldest first.
pub fn load(dir: &Path, thread: &str) -> anyhow::Result<Vec<SideTurn>> {
    check(thread)?;
    db::read(dir, |c| {
        let mut q = c.prepare(
            "SELECT id, ts, role, text, meta FROM side_turn WHERE thread = ?1 ORDER BY id",
        )?;
        let rows = q.query_map([thread], |r| {
            Ok((
                r.get::<_, i64>(0)?,
                r.get::<_, u64>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, String>(3)?,
                r.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|r| {
            let (id, ts, role, text, meta) = r?;
            let m: Meta = serde_json::from_str(&meta).unwrap_or_default();
            Ok(SideTurn {
                id,
                ts,
                role,
                text,
                files: m.files,
                info: m.info,
            })
        })
        .collect()
    })
}

/// Adds one question and its answer, in one change.
pub fn append_pair(
    dir: &Path,
    thread: &str,
    question: &str,
    files: &[FileRef],
    answer: &str,
    info: &str,
) -> anyhow::Result<()> {
    check(thread)?;
    db::write(dir, |tx| {
        let ts = now() as i64;
        let q = serde_json::to_string(&Meta {
            files: files.to_vec(),
            info: None,
        })?;
        let a = serde_json::to_string(&Meta {
            files: vec![],
            info: Some(info.into()),
        })?;
        tx.execute(
            "INSERT INTO side_turn (thread, ts, role, text, meta) VALUES (?1, ?2, 'user', ?3, ?4)",
            rusqlite::params![thread, ts, question, q],
        )?;
        tx.execute("INSERT INTO side_turn (thread, ts, role, text, meta) VALUES (?1, ?2, 'assistant', ?3, ?4)", rusqlite::params![thread, ts, answer, a])?;
        Ok(())
    })
}

/// Removes all messages of the thread.
pub fn clear(dir: &Path, thread: &str) -> anyhow::Result<()> {
    check(thread)?;
    db::write(dir, |tx| {
        tx.execute("DELETE FROM side_turn WHERE thread = ?1", [thread])?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_thread_keeps_its_own_messages() {
        let d = std::env::temp_dir().join(format!("aifred-side-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let f = FileRef {
            id: "f1".into(),
            name: "a.txt".into(),
            size: 3,
        };
        append_pair(
            &d,
            "plan:2026-11-04",
            "Plan my day",
            std::slice::from_ref(&f),
            "Done.",
            "2.0s",
        )
        .unwrap();
        append_pair(&d, "item:task:t1", "What is this?", &[], "A task.", "1.0s").unwrap();
        let plan = load(&d, "plan:2026-11-04").unwrap();
        assert_eq!(
            plan.iter().map(|t| t.role.as_str()).collect::<Vec<_>>(),
            ["user", "assistant"]
        );
        assert_eq!(plan[0].files, vec![f]);
        assert_eq!(plan[1].info.as_deref(), Some("2.0s"));
        assert_eq!(load(&d, "item:task:t1").unwrap().len(), 2);
        clear(&d, "plan:2026-11-04").unwrap();
        assert!(load(&d, "plan:2026-11-04").unwrap().is_empty());
        assert_eq!(
            load(&d, "item:task:t1").unwrap().len(),
            2,
            "a clear changes only its own thread"
        );
        for bad in ["plan:4 Nov", "plan:abcdefghij", "item:jira:1", "main", ""] {
            assert!(load(&d, bad).is_err(), "{bad}");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }
}
