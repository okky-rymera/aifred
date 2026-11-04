//! Local tasks: the ones the user or aifred adds by hand.
//! Stored in the task table of the database (db.rs). The desktop app and the
//! tasks tool server both use it.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::bail;
use rusqlite::{OptionalExtension, Row, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::files::now;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    pub done: bool,
    /// "you" or "aifred".
    pub added_by: String,
    pub created: u64,
    #[serde(default)]
    pub done_at: Option<u64>,
}

pub struct TaskStore {
    dir: PathBuf,
}

/// A task id that is never given out again, also not after a delete.
/// It uses the time in nanoseconds. The caller still checks for a clash.
fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("t{nanos}")
}

const COLUMNS: &str = "id, title, notes, done, added_by, created, done_at";

fn from_row(r: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        done: r.get(3)?,
        added_by: r.get(4)?,
        created: r.get(5)?,
        done_at: r.get(6)?,
    })
}

/// Adds a task as it is. The import of tasks.json also uses it.
pub(crate) fn insert(tx: &Transaction, t: &Task) -> anyhow::Result<()> {
    tx.execute(
        &format!("INSERT INTO task ({COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)"),
        rusqlite::params![t.id, t.title, t.notes, t.done, t.added_by, t.created as i64, t.done_at.map(|d| d as i64)],
    )?;
    Ok(())
}

impl TaskStore {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    /// All tasks, in the order they were added.
    pub fn load(&self) -> anyhow::Result<Vec<Task>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(&format!("SELECT {COLUMNS} FROM task ORDER BY rowid"))?;
            Ok(q.query_map([], from_row)?.collect::<Result<_, _>>()?)
        })
    }

    pub fn add(&self, title: &str, notes: &str, added_by: &str) -> anyhow::Result<Task> {
        let title = title.trim();
        if title.is_empty() {
            bail!("the task title is empty");
        }
        db::write(&self.dir, |tx| {
            let mut id = new_id();
            while tx.query_row("SELECT 1 FROM task WHERE id = ?1", [&id], |_| Ok(())).optional()?.is_some() {
                id = new_id();
            }
            let task = Task {
                id,
                title: title.into(),
                notes: notes.trim().into(),
                done: false,
                added_by: added_by.into(),
                created: now(),
                done_at: None,
            };
            insert(tx, &task)?;
            Ok(task)
        })
    }

    /// Changes the title and the description (notes) of a task.
    pub fn edit(&self, id: &str, title: &str, notes: &str) -> anyhow::Result<Task> {
        let title = title.trim();
        if title.is_empty() {
            bail!("the task title is empty");
        }
        db::write(&self.dir, |tx| {
            if tx.execute("UPDATE task SET title = ?1, notes = ?2 WHERE id = ?3", [title, notes.trim(), id])? == 0 {
                bail!("no task with id {id}");
            }
            Ok(tx.query_row(&format!("SELECT {COLUMNS} FROM task WHERE id = ?1"), [id], from_row)?)
        })
    }

    pub fn set_done(&self, id: &str, done: bool) -> anyhow::Result<Task> {
        db::write(&self.dir, |tx| {
            let done_at = done.then(|| now() as i64);
            if tx.execute("UPDATE task SET done = ?1, done_at = ?2 WHERE id = ?3", rusqlite::params![done, done_at, id])? == 0 {
                bail!("no task with id {id}");
            }
            Ok(tx.query_row(&format!("SELECT {COLUMNS} FROM task WHERE id = ?1"), [id], from_row)?)
        })
    }

    /// Tasks that match `query` by words or by meaning in the title or the
    /// notes, best match first. Done tasks only when `include_done` is true.
    pub fn search(&self, query: &str, include_done: bool, limit: Option<u32>) -> anyhow::Result<Vec<Task>> {
        db::read(&self.dir, |c| {
            let ids = db::hybrid(
                c,
                &self.dir,
                query,
                "task_vec",
                "SELECT t.rowid FROM task_fts f JOIN task t ON t.rowid = f.rowid
                 WHERE task_fts MATCH ?1 AND (?2 OR t.done = 0) ORDER BY bm25(task_fts, 2.0, 1.0)",
                "SELECT 1 FROM task WHERE rowid = ?1 AND (?2 OR done = 0)",
                &[&include_done],
                limit,
            )?;
            let mut get = c.prepare(&format!("SELECT {COLUMNS} FROM task WHERE rowid = ?1"))?;
            Ok(ids.iter().map(|id| get.query_row([id], from_row)).collect::<Result<_, _>>()?)
        })
    }

    pub fn remove(&self, id: &str) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            if tx.execute("DELETE FROM task WHERE id = ?1", [id])? == 0 {
                bail!("no task with id {id}");
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A new, empty folder for each test. Tests run at the same time,
    /// so each one needs its own folder.
    fn store(name: &str) -> (TaskStore, PathBuf) {
        let dir = std::env::temp_dir().join(format!("aifred-tasks-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        (TaskStore::new(&dir), dir)
    }

    #[test]
    fn add_done_remove() {
        let (s, dir) = store("basic");
        let a = s.add("Write report", "", "you").unwrap();
        let b = s.add("Write report", "", "aifred").unwrap();
        assert_ne!(a.id, b.id);
        assert!(s.set_done(&a.id, true).unwrap().done_at.is_some());
        s.remove(&b.id).unwrap();
        let all = s.load().unwrap();
        assert_eq!(all.len(), 1);
        assert!(all[0].done);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn missing_file_means_no_tasks() {
        let (s, dir) = store("missing");
        assert!(s.load().unwrap().is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn add_trims_text_and_rejects_empty_title() {
        let (s, dir) = store("trim");
        let t = s.add("  Call Sam  ", "  before 5pm ", "you").unwrap();
        assert_eq!(t.title, "Call Sam");
        assert_eq!(t.notes, "before 5pm");
        assert_eq!(t.added_by, "you");
        assert!(!t.done);
        assert!(s.add("   ", "", "you").is_err());
        assert_eq!(s.load().unwrap().len(), 1, "a failed add saves nothing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn many_fast_adds_get_unique_ids() {
        let (s, dir) = store("ids");
        let ids: std::collections::HashSet<_> = (0..5).map(|i| s.add(&format!("Task {i}"), "", "you").unwrap().id).collect();
        assert_eq!(ids.len(), 5);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn undo_done_clears_done_time() {
        let (s, dir) = store("undo");
        let t = s.add("Pay invoice", "", "you").unwrap();
        s.set_done(&t.id, true).unwrap();
        let back = s.set_done(&t.id, false).unwrap();
        assert!(!back.done);
        assert!(back.done_at.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_id_is_an_error() {
        let (s, dir) = store("unknown");
        assert!(s.set_done("t123", true).is_err());
        assert!(s.remove("t123").is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The app and the tool server change the tasks at the same time.
    /// Each writer opens the database on its own, like a separate program.
    #[test]
    fn writers_at_the_same_time_lose_no_change() {
        let (_, dir) = store("race");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let dir = dir.clone();
                std::thread::spawn(move || {
                    let s = TaskStore::new(&dir);
                    for j in 0..10 {
                        s.add(&format!("Task {i}-{j}"), "", "you").unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert_eq!(TaskStore::new(&dir).load().unwrap().len(), 80);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn deleted_id_is_not_given_out_again() {
        let (s, dir) = store("reuse");
        let old = s.add("Old", "", "you").unwrap();
        s.remove(&old.id).unwrap();
        let new = s.add("New", "", "you").unwrap();
        assert_ne!(old.id, new.id);
        assert!(s.set_done(&old.id, true).is_err(), "an old id must not change the new task");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn search_finds_title_and_notes_and_hides_done_tasks() {
        let (s, dir) = store("search");
        let a = s.add("Pay the invoice", "for the hosting", "you").unwrap();
        let b = s.add("Call Sam", "about invoices", "you").unwrap();
        s.add("Buy milk", "", "you").unwrap();
        let ids = |v: Vec<Task>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(ids(s.search("invoice", false, None).unwrap()), vec![a.id.clone(), b.id.clone()], "title first");
        assert_eq!(ids(s.search("hosting", false, None).unwrap()), vec![a.id.clone()]);
        s.set_done(&a.id, true).unwrap();
        assert_eq!(ids(s.search("invoice", false, None).unwrap()), vec![b.id.clone()]);
        assert_eq!(s.search("invoice", true, None).unwrap().len(), 2);
        s.remove(&b.id).unwrap();
        assert!(s.search("sam", true, None).unwrap().is_empty(), "a deleted task is not found");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// Two real programs write at the same time. Threads share one process,
    /// so this test starts itself two more times as the writers.
    #[test]
    fn two_programs_write_at_the_same_time() {
        const CHILD: &str = "AIFRED_TEST_WRITER_DIR";
        if let Some(dir) = std::env::var_os(CHILD) {
            let s = TaskStore::new(Path::new(&dir));
            for j in 0..25 {
                if let Err(e) = s.add(&format!("Task {}-{j}", std::process::id()), "", "you") {
                    panic!("{e:#}");
                }
            }
            return;
        }
        let (_, dir) = store("processes");
        let exe = std::env::current_exe().unwrap();
        let children: Vec<_> = (0..2)
            .map(|_| {
                std::process::Command::new(&exe)
                    .args(["--exact", "tasks::tests::two_programs_write_at_the_same_time", "--quiet"])
                    .env(CHILD, &dir)
                    .stdout(std::process::Stdio::null())
                    .spawn()
                    .unwrap()
            })
            .collect();
        for mut c in children {
            assert!(c.wait().unwrap().success(), "a writer failed");
        }
        assert_eq!(TaskStore::new(&dir).load().unwrap().len(), 50);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
