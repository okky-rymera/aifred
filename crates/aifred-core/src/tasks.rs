//! Local tasks: the ones the user or aifred adds by hand.
//! Stored in tasks.json in the data folder. The desktop app and the tasks
//! tool server both use this file.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};

use crate::files::{self, now};
use serde::{Deserialize, Serialize};

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
    path: PathBuf,
}

/// A task id that is never given out again, also not after a delete.
/// It uses the time in nanoseconds. The caller still checks the file for a clash.
fn new_id() -> String {
    let nanos = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    format!("t{nanos}")
}

impl TaskStore {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("tasks.json") }
    }

    pub fn load(&self) -> anyhow::Result<Vec<Task>> {
        files::load(&self.path)
    }

    /// Locked read, change, and save. See files::update.
    fn update<T>(&self, f: impl FnOnce(&mut Vec<Task>) -> anyhow::Result<T>) -> anyhow::Result<T> {
        files::update(&self.path, f)
    }

    pub fn add(&self, title: &str, notes: &str, added_by: &str) -> anyhow::Result<Task> {
        let title = title.trim();
        if title.is_empty() {
            bail!("the task title is empty");
        }
        self.update(|tasks| {
            let created = now();
            let mut id = new_id();
            while tasks.iter().any(|t| t.id == id) {
                id = new_id();
            }
            let task = Task {
                id,
                title: title.into(),
                notes: notes.trim().into(),
                done: false,
                added_by: added_by.into(),
                created,
                done_at: None,
            };
            tasks.push(task.clone());
            Ok(task)
        })
    }

    pub fn set_done(&self, id: &str, done: bool) -> anyhow::Result<Task> {
        self.update(|tasks| {
            let t = tasks.iter_mut().find(|t| t.id == id).with_context(|| format!("no task with id {id}"))?;
            t.done = done;
            t.done_at = done.then(now);
            Ok(t.clone())
        })
    }

    pub fn remove(&self, id: &str) -> anyhow::Result<()> {
        self.update(|tasks| {
            let before = tasks.len();
            tasks.retain(|t| t.id != id);
            if tasks.len() == before {
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

    #[test]
    fn save_leaves_no_temporary_file() {
        let (s, dir) = store("tmp");
        s.add("One", "", "you").unwrap();
        assert!(dir.join("tasks.json").exists());
        let tmp: Vec<_> = std::fs::read_dir(&dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".tmp"))
            .collect();
        assert!(tmp.is_empty(), "left over: {tmp:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The app and the tool server change the file at the same time.
    /// Each writer opens the file on its own, like a separate program.
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
    fn broken_file_is_reported_not_erased() {
        let (s, dir) = store("broken");
        std::fs::write(dir.join("tasks.json"), "{oops").unwrap();
        assert!(s.load().is_err());
        assert!(s.add("New", "", "you").is_err(), "do not overwrite a file we cannot read");
        assert_eq!(std::fs::read_to_string(dir.join("tasks.json")).unwrap(), "{oops");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn old_file_without_optional_fields_loads() {
        let (s, dir) = store("old");
        std::fs::write(
            dir.join("tasks.json"),
            r#"[{"id":"t1","title":"Old","done":false,"added_by":"you","created":1}]"#,
        )
        .unwrap();
        let all = s.load().unwrap();
        assert_eq!(all[0].notes, "");
        assert!(all[0].done_at.is_none());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
