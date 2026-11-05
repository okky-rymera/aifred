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
use crate::uploads::{self, FileRef};

/// The priorities, most important first. The same names as in ClickUp.
pub const PRIORITIES: [&str; 4] = ["urgent", "high", "normal", "low"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    #[serde(default)]
    pub done: bool,
}

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
    /// The day the task must be done, "YYYY-MM-DD".
    #[serde(default)]
    pub due: Option<String>,
    /// The first day to work on the task, "YYYY-MM-DD". Before it, the task waits.
    #[serde(default)]
    pub start: Option<String>,
    /// One of PRIORITIES, or None.
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub checklist: Vec<ChecklistItem>,
    /// Files that the user keeps on the task (uploads.rs).
    #[serde(default)]
    pub files: Vec<FileRef>,
}

/// The fields to set on a task. None keeps the field as it is (on an add:
/// empty). For `due`, `start`, and `priority`, an empty text clears the field.
#[derive(Debug, Default, Clone, Deserialize)]
pub struct TaskFields {
    pub title: Option<String>,
    pub notes: Option<String>,
    pub due: Option<String>,
    pub start: Option<String>,
    pub priority: Option<String>,
    pub tags: Option<Vec<String>>,
    pub checklist: Option<Vec<StepFields>>,
    /// The ids of the files of the task (uploads.rs). The full new list.
    pub files: Option<Vec<String>>,
}

/// One step in a new checklist. When `done` is None, the step keeps the
/// done value of the old step with the same text (or not done, for a new
/// step). So the AI can add a step and not un-tick the steps that are done.
#[derive(Debug, Clone, Deserialize)]
pub struct StepFields {
    pub text: String,
    #[serde(default)]
    pub done: Option<bool>,
}

impl From<ChecklistItem> for StepFields {
    fn from(c: ChecklistItem) -> Self {
        Self {
            text: c.text,
            done: Some(c.done),
        }
    }
}

pub struct TaskStore {
    dir: PathBuf,
}

/// A task id that is never given out again, also not after a delete.
/// It uses the time in nanoseconds. The caller still checks for a clash.
fn new_id() -> String {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    format!("t{nanos}")
}

const COLUMNS: &str = "id, title, notes, done, added_by, created, done_at, due, start, priority, tags, checklist, files";

/// A JSON column as a value. A bad value is a read error, not an empty list.
pub(crate) fn json_column<T: serde::de::DeserializeOwned>(
    r: &Row,
    i: usize,
) -> rusqlite::Result<T> {
    let text: String = r.get(i)?;
    serde_json::from_str(&text).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(i, rusqlite::types::Type::Text, e.into())
    })
}

fn from_row(r: &Row) -> rusqlite::Result<Task> {
    Ok(Task {
        id: r.get(0)?,
        title: r.get(1)?,
        notes: r.get(2)?,
        done: r.get(3)?,
        added_by: r.get(4)?,
        created: r.get(5)?,
        done_at: r.get(6)?,
        due: r.get(7)?,
        start: r.get(8)?,
        priority: r.get(9)?,
        tags: json_column(r, 10)?,
        checklist: json_column(r, 11)?,
        files: json_column(r, 12)?,
    })
}

/// Adds a task from the old tasks.json. The import runs on the first schema,
/// before the newer columns exist, so it writes only the first columns.
/// Old tasks.json files have no newer fields.
pub(crate) fn insert(tx: &Transaction, t: &Task) -> anyhow::Result<()> {
    tx.execute(
        "INSERT INTO task (id, title, notes, done, added_by, created, done_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        rusqlite::params![t.id, t.title, t.notes, t.done, t.added_by, t.created as i64, t.done_at.map(|d| d as i64)],
    )?;
    Ok(())
}

/// True for a real day written as "YYYY-MM-DD".
fn is_date(s: &str) -> bool {
    let b = s.as_bytes();
    if b.len() != 10 || b[4] != b'-' || b[7] != b'-' {
        return false;
    }
    let num = |r: std::ops::Range<usize>| {
        let part = &s[r];
        part.bytes()
            .all(|c| c.is_ascii_digit())
            .then(|| part.parse::<u32>().ok())
            .flatten()
    };
    let (Some(y), Some(m), Some(d)) = (num(0..4), num(5..7), num(8..10)) else {
        return false;
    };
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let days = match m {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&d)
}

/// An empty text clears the date.
fn clean_date(name: &str, s: &str) -> anyhow::Result<Option<String>> {
    let s = s.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if !is_date(s) {
        bail!("the {name} date \"{s}\" is not a day in the form YYYY-MM-DD");
    }
    Ok(Some(s.into()))
}

fn clean_priority(s: &str) -> anyhow::Result<Option<String>> {
    let s = s.trim().to_lowercase();
    if s.is_empty() || s == "none" {
        return Ok(None);
    }
    if !PRIORITIES.contains(&s.as_str()) {
        bail!(
            "the priority \"{s}\" is not one of: {}",
            PRIORITIES.join(", ")
        );
    }
    Ok(Some(s))
}

/// A tag as it is saved: no spaces at the ends and no "#" in front.
/// Example: " # work " becomes "work".
pub fn clean_tag(t: &str) -> &str {
    t.trim().trim_start_matches('#').trim()
}

/// Cleans each tag (clean_tag). Drops empty tags and tags that are already
/// in the list with other capital letters.
fn clean_tags(tags: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = clean_tag(t);
        if !t.is_empty() && !out.iter().any(|o| o.to_lowercase() == t.to_lowercase()) {
            out.push(t.into());
        }
    }
    out
}

/// Puts the fields on the task, and checks them.
fn apply(t: &mut Task, f: TaskFields) -> anyhow::Result<()> {
    if let Some(title) = f.title {
        t.title = title.trim().into();
    }
    if t.title.is_empty() {
        bail!("the task title is empty");
    }
    if let Some(notes) = f.notes {
        t.notes = notes.trim().into();
    }
    if let Some(due) = f.due {
        t.due = clean_date("due", &due)?;
    }
    if let Some(start) = f.start {
        t.start = clean_date("start", &start)?;
    }
    if let Some(p) = f.priority {
        t.priority = clean_priority(&p)?;
    }
    if let Some(tags) = f.tags {
        t.tags = clean_tags(&tags);
    }
    if let Some(list) = f.checklist {
        let old = std::mem::take(&mut t.checklist);
        let was_done = |text: &str| {
            old.iter()
                .any(|o| o.done && o.text.to_lowercase() == text.to_lowercase())
        };
        t.checklist = list
            .into_iter()
            .map(|i| {
                let text: String = i.text.trim().into();
                let done = i.done.unwrap_or_else(|| was_done(&text));
                ChecklistItem { text, done }
            })
            .filter(|i| !i.text.is_empty())
            .collect();
    }
    // The text form "YYYY-MM-DD" sorts in date order.
    if let (Some(start), Some(due)) = (&t.start, &t.due)
        && start > due
    {
        bail!("the start date {start} is after the due date {due}");
    }
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

    /// The saved files for the file ids in `f`, or None when `f` has no list.
    fn files(&self, f: &TaskFields) -> anyhow::Result<Option<Vec<FileRef>>> {
        f.files
            .as_ref()
            .map(|ids| ids.iter().map(|id| uploads::get(&self.dir, id)).collect())
            .transpose()
    }

    /// Adds a task with only a title and a description.
    pub fn add(&self, title: &str, notes: &str, added_by: &str) -> anyhow::Result<Task> {
        self.create(
            TaskFields {
                title: Some(title.into()),
                notes: Some(notes.into()),
                ..Default::default()
            },
            added_by,
        )
    }

    /// Adds a task with all the fields in `f`. The title is necessary.
    pub fn create(&self, f: TaskFields, added_by: &str) -> anyhow::Result<Task> {
        let mut task = Task {
            id: String::new(),
            title: String::new(),
            notes: String::new(),
            done: false,
            added_by: added_by.into(),
            created: now(),
            done_at: None,
            due: None,
            start: None,
            priority: None,
            tags: Vec::new(),
            checklist: Vec::new(),
            files: Vec::new(),
        };
        let files = self.files(&f)?;
        apply(&mut task, f)?;
        if let Some(files) = files {
            task.files = files;
        }
        db::write(&self.dir, |tx| {
            task.id = new_id();
            while tx
                .query_row("SELECT 1 FROM task WHERE id = ?1", [&task.id], |_| Ok(()))
                .optional()?
                .is_some()
            {
                task.id = new_id();
            }
            // insert() already writes the title and the description.
            insert(tx, &task)?;
            write_fields(tx, &task, false)?;
            Ok(task)
        })
    }

    /// Changes the fields in `f`. The other fields stay as they are.
    pub fn update(&self, id: &str, f: TaskFields) -> anyhow::Result<Task> {
        db::write(&self.dir, |tx| {
            let Some(mut task) = tx
                .query_row(
                    &format!("SELECT {COLUMNS} FROM task WHERE id = ?1"),
                    [id],
                    from_row,
                )
                .optional()?
            else {
                bail!("no task with id {id}");
            };
            let old_text = (task.title.clone(), task.notes.clone());
            let files = self.files(&f)?;
            apply(&mut task, f)?;
            if let Some(files) = files {
                task.files = files;
            }
            let text_changed = old_text != (task.title.clone(), task.notes.clone());
            write_fields(tx, &task, text_changed)?;
            Ok(task)
        })
    }

    /// Ticks or un-ticks one step of the checklist. Only this step changes,
    /// so two fast ticks do not undo each other. `text` must be the text of
    /// the step at `index`: if the checklist changed, the tick fails.
    pub fn set_step(&self, id: &str, index: usize, text: &str, done: bool) -> anyhow::Result<Task> {
        db::write(&self.dir, |tx| {
            let Some(mut task) = tx
                .query_row(
                    &format!("SELECT {COLUMNS} FROM task WHERE id = ?1"),
                    [id],
                    from_row,
                )
                .optional()?
            else {
                bail!("no task with id {id}");
            };
            match task.checklist.get_mut(index) {
                Some(step) if step.text == text => step.done = done,
                _ => bail!("the checklist changed. Reload the list and try again"),
            }
            tx.execute(
                "UPDATE task SET checklist = ?1 WHERE id = ?2",
                rusqlite::params![serde_json::to_string(&task.checklist)?, id],
            )?;
            Ok(task)
        })
    }

    pub fn set_done(&self, id: &str, done: bool) -> anyhow::Result<Task> {
        db::write(&self.dir, |tx| {
            let done_at = done.then(|| now() as i64);
            if tx.execute(
                "UPDATE task SET done = ?1, done_at = ?2 WHERE id = ?3",
                rusqlite::params![done, done_at, id],
            )? == 0
            {
                bail!("no task with id {id}");
            }
            Ok(tx.query_row(
                &format!("SELECT {COLUMNS} FROM task WHERE id = ?1"),
                [id],
                from_row,
            )?)
        })
    }

    /// Tasks that match `query` by words or by meaning in the title or the
    /// notes, best match first. Done tasks only when `include_done` is true.
    pub fn search(
        &self,
        query: &str,
        include_done: bool,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<Task>> {
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
            Ok(ids
                .iter()
                .map(|id| get.query_row([id], from_row))
                .collect::<Result<_, _>>()?)
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

/// Writes the fields that the user can change. The title and the
/// description are written only when `text_changed` is true: each write of
/// them runs the word-search trigger (db.rs), which indexes the task again.
fn write_fields(tx: &Transaction, t: &Task, text_changed: bool) -> anyhow::Result<()> {
    if text_changed {
        tx.execute(
            "UPDATE task SET title = ?1, notes = ?2 WHERE id = ?3",
            rusqlite::params![t.title, t.notes, t.id],
        )?;
    }
    tx.execute(
        "UPDATE task SET due = ?1, start = ?2, priority = ?3, tags = ?4, checklist = ?5, files = ?6 WHERE id = ?7",
        rusqlite::params![
            t.due,
            t.start,
            t.priority,
            serde_json::to_string(&t.tags)?,
            serde_json::to_string(&t.checklist)?,
            serde_json::to_string(&t.files)?,
            t.id
        ],
    )?;
    Ok(())
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

    fn fields() -> TaskFields {
        TaskFields {
            title: Some(" Book flights ".into()),
            due: Some("2026-12-20".into()),
            start: Some("2026-12-01".into()),
            priority: Some("High".into()),
            tags: Some(vec![
                "travel".into(),
                "#Home".into(),
                " ".into(),
                "TRAVEL".into(),
            ]),
            checklist: Some(vec![
                StepFields {
                    text: " Pick dates ".into(),
                    done: Some(true),
                },
                StepFields {
                    text: "  ".into(),
                    done: Some(false),
                },
                StepFields {
                    text: "Pay".into(),
                    done: None,
                },
            ]),
            ..Default::default()
        }
    }

    #[test]
    fn create_keeps_all_fields_and_cleans_them() {
        let (s, dir) = store("fields");
        let t = s.create(fields(), "aifred").unwrap();
        assert_eq!(t.title, "Book flights");
        assert_eq!(t.due.as_deref(), Some("2026-12-20"));
        assert_eq!(t.start.as_deref(), Some("2026-12-01"));
        assert_eq!(t.priority.as_deref(), Some("high"));
        assert_eq!(t.tags, vec!["travel", "Home"], "no empty tags, no copies");
        assert_eq!(
            t.checklist,
            vec![
                ChecklistItem {
                    text: "Pick dates".into(),
                    done: true
                },
                ChecklistItem {
                    text: "Pay".into(),
                    done: false
                }
            ]
        );
        let saved = s.load().unwrap().pop().unwrap();
        assert_eq!(
            (saved.tags, saved.checklist, saved.due),
            (t.tags, t.checklist, t.due),
            "the database keeps the same fields"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn update_changes_only_the_given_fields() {
        let (s, dir) = store("update");
        let t = s.create(fields(), "you").unwrap();
        let u = s
            .update(
                &t.id,
                TaskFields {
                    priority: Some("".into()),
                    due: Some("2026-12-24".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(u.priority, None, "an empty text clears the priority");
        assert_eq!(u.due.as_deref(), Some("2026-12-24"));
        assert_eq!(
            (
                u.title.as_str(),
                u.start.as_deref(),
                u.tags.len(),
                u.checklist.len()
            ),
            ("Book flights", Some("2026-12-01"), 2, 2)
        );
        let u = s
            .update(
                &t.id,
                TaskFields {
                    start: Some(" ".into()),
                    tags: Some(vec![]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!((u.start, u.tags.len()), (None, 0));
        assert!(
            s.update("t123", TaskFields::default()).is_err(),
            "unknown id"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn bad_fields_are_rejected_and_save_nothing() {
        let (s, dir) = store("bad");
        let t = s.add("Pay rent", "", "you").unwrap();
        for (f, words) in [
            (
                TaskFields {
                    due: Some("2026-02-29".into()),
                    ..Default::default()
                },
                "not a day",
            ),
            (
                TaskFields {
                    due: Some("tomorrow".into()),
                    ..Default::default()
                },
                "not a day",
            ),
            (
                TaskFields {
                    start: Some("2026-13-01".into()),
                    ..Default::default()
                },
                "not a day",
            ),
            (
                TaskFields {
                    priority: Some("p1".into()),
                    ..Default::default()
                },
                "not one of",
            ),
            (
                TaskFields {
                    start: Some("2026-12-02".into()),
                    due: Some("2026-12-01".into()),
                    ..Default::default()
                },
                "after the due date",
            ),
            (
                TaskFields {
                    title: Some(" ".into()),
                    ..Default::default()
                },
                "title is empty",
            ),
        ] {
            let e = s.update(&t.id, f).unwrap_err().to_string();
            assert!(e.contains(words), "got {e}");
        }
        assert!(
            s.update(
                &t.id,
                TaskFields {
                    due: Some("2028-02-29".into()),
                    ..Default::default()
                }
            )
            .is_ok(),
            "a leap day is a real day"
        );
        s.update(
            &t.id,
            TaskFields {
                due: Some("2026-12-01".into()),
                ..Default::default()
            },
        )
        .unwrap();
        let e = s
            .update(
                &t.id,
                TaskFields {
                    start: Some("2026-12-05".into()),
                    ..Default::default()
                },
            )
            .unwrap_err();
        assert!(
            e.to_string().contains("after the due date"),
            "checks against the saved due date"
        );
        assert_eq!(s.load().unwrap()[0].start, None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_step_with_no_done_value_keeps_its_tick() {
        let (s, dir) = store("keep-tick");
        let t = s.create(fields(), "you").unwrap();
        let step = |text: &str| StepFields {
            text: text.into(),
            done: None,
        };
        let u = s
            .update(
                &t.id,
                TaskFields {
                    checklist: Some(vec![step("pick DATES"), step("Pay"), step("Pack")]),
                    ..Default::default()
                },
            )
            .unwrap();
        let done: Vec<bool> = u.checklist.iter().map(|c| c.done).collect();
        assert_eq!(
            done,
            vec![true, false, false],
            "the done step stays done; a new step is not done"
        );
        let u = s
            .update(
                &t.id,
                TaskFields {
                    checklist: Some(vec![StepFields {
                        text: "Pick dates".into(),
                        done: Some(false),
                    }]),
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(!u.checklist[0].done, "a done value that is given wins");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn set_step_changes_only_one_step() {
        let (s, dir) = store("set-step");
        let t = s.create(fields(), "you").unwrap();
        // Two ticks made from the same old list: the second must not undo the first.
        s.set_step(&t.id, 0, "Pick dates", false).unwrap();
        s.set_step(&t.id, 1, "Pay", true).unwrap();
        let saved = s.load().unwrap().pop().unwrap();
        assert_eq!(
            saved.checklist.iter().map(|c| c.done).collect::<Vec<_>>(),
            vec![false, true]
        );
        let e = s.set_step(&t.id, 1, "Pick dates", true).unwrap_err();
        assert!(e.to_string().contains("checklist changed"), "got {e}");
        assert!(s.set_step(&t.id, 5, "Pay", true).is_err(), "no step 5");
        assert!(s.set_step("t123", 0, "Pay", true).is_err(), "unknown id");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn search_follows_a_title_change() {
        let (s, dir) = store("search-edit");
        let t = s.create(fields(), "you").unwrap();
        s.update(
            &t.id,
            TaskFields {
                title: Some("Book trains".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert!(
            s.search("flights", false, None).unwrap().is_empty(),
            "the old title is not found"
        );
        assert_eq!(s.search("trains", false, None).unwrap().len(), 1);
        s.update(
            &t.id,
            TaskFields {
                priority: Some("low".into()),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(
            s.search("trains", false, None).unwrap().len(),
            1,
            "other changes keep the search entry"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn many_fast_adds_get_unique_ids() {
        let (s, dir) = store("ids");
        let ids: std::collections::HashSet<_> = (0..5)
            .map(|i| s.add(&format!("Task {i}"), "", "you").unwrap().id)
            .collect();
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
        assert!(
            s.set_done(&old.id, true).is_err(),
            "an old id must not change the new task"
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn search_finds_title_and_notes_and_hides_done_tasks() {
        let (s, dir) = store("search");
        let a = s.add("Pay the invoice", "for the hosting", "you").unwrap();
        let b = s.add("Call Sam", "about invoices", "you").unwrap();
        s.add("Buy milk", "", "you").unwrap();
        let ids = |v: Vec<Task>| v.into_iter().map(|t| t.id).collect::<Vec<_>>();
        assert_eq!(
            ids(s.search("invoice", false, None).unwrap()),
            vec![a.id.clone(), b.id.clone()],
            "title first"
        );
        assert_eq!(
            ids(s.search("hosting", false, None).unwrap()),
            vec![a.id.clone()]
        );
        s.set_done(&a.id, true).unwrap();
        assert_eq!(
            ids(s.search("invoice", false, None).unwrap()),
            vec![b.id.clone()]
        );
        assert_eq!(s.search("invoice", true, None).unwrap().len(), 2);
        s.remove(&b.id).unwrap();
        assert!(
            s.search("sam", true, None).unwrap().is_empty(),
            "a deleted task is not found"
        );
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
                    .args([
                        "--exact",
                        "tasks::tests::two_programs_write_at_the_same_time",
                        "--quiet",
                    ])
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
