//! The one-time move of the old files into the database. It runs in the same
//! transaction that makes the tables (db::upgrade). If one file cannot be
//! read, nothing is saved and the error shows, so no data is lost.
//!
//! - memory.md becomes memory facts (see memory.rs).
//! - history.jsonl becomes the current conversation.
//! - Each history-<time>.jsonl becomes one ended conversation.
//! - tasks.json becomes task rows, with the same ids.
//!
//! After the save, each imported file gets the name <name>.imported. It is
//! not deleted. The old saved GitHub and ClickUp lists are not imported: the
//! app downloads them again. Their files and the old lock files are deleted.

use std::io::BufRead;
use std::path::{Path, PathBuf};

use anyhow::Context;
use rusqlite::Transaction;

use crate::history::{self, Turn};
use crate::tasks::{self, Task};
use crate::{files, memory};

/// Files that are not imported. They are deleted after the import.
const DROP: &[&str] = &[
    "tasks.lock",
    "github-cache.json",
    "github-cache.lock",
    "clickup-cache.json",
    "clickup-cache.lock",
];

/// What to do with the old files after the import is saved.
pub(crate) struct Imported {
    files: Vec<PathBuf>,
}

pub(crate) fn run(tx: &Transaction, dir: &Path, now: u64) -> anyhow::Result<Imported> {
    let mut done = Vec::new();

    let path = dir.join("memory.md");
    if let Some(text) = read(&path)? {
        memory::set_text(tx, &String::from_utf8_lossy(&text), "you", now)?;
        done.push(path);
    }

    let path = dir.join("tasks.json");
    if path.exists() {
        let list: Vec<Task> = files::load(&path)?;
        for t in &list {
            tasks::insert(tx, t)
                .with_context(|| format!("cannot import task {} from {}", t.id, path.display()))?;
        }
        done.push(path);
    }

    // Ended conversations first, oldest first, so the ids keep the time order.
    let mut old = Vec::new();
    for entry in std::fs::read_dir(dir).with_context(|| format!("cannot read {}", dir.display()))? {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default();
        if let Some(ts) = name
            .strip_prefix("history-")
            .and_then(|n| n.strip_suffix(".jsonl"))
            .and_then(|t| t.parse::<u64>().ok())
        {
            old.push((ts, path));
        }
    }
    old.sort();
    for (ended, path) in old {
        history::insert_conversation(tx, &turns(&path)?, Some(ended), now)?;
        done.push(path);
    }
    let path = dir.join("history.jsonl");
    if path.exists() {
        history::insert_conversation(tx, &turns(&path)?, None, now)?;
        done.push(path);
    }

    Ok(Imported { files: done })
}

impl Imported {
    /// Renames the imported files and deletes the files that are not needed.
    /// The data is already saved, so a failure here only prints a notice.
    pub(crate) fn finish(self, dir: &Path) {
        for path in self.files {
            let mut to = path.clone().into_os_string();
            to.push(".imported");
            if let Err(e) = std::fs::rename(&path, &to) {
                eprintln!(
                    "aifred: imported {}, but cannot rename it: {e}",
                    path.display()
                );
            }
        }
        for name in DROP {
            match std::fs::remove_file(dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    eprintln!("aifred: cannot remove {name}: {e}")
                }
                _ => {}
            }
        }
    }
}

/// The file, or None if it is not there.
fn read(path: &Path) -> anyhow::Result<Option<Vec<u8>>> {
    match std::fs::read(path) {
        Ok(b) => Ok(Some(b)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// The messages in a history file. The old app skipped lines that it could
/// not read, so the import skips them too. The file is kept as .imported.
fn turns(path: &Path) -> anyhow::Result<Vec<Turn>> {
    let f = std::fs::File::open(path).with_context(|| format!("cannot read {}", path.display()))?;
    let mut out = Vec::new();
    for line in std::io::BufReader::new(f).lines() {
        let line = line.with_context(|| format!("cannot read {}", path.display()))?;
        if let Ok(t) = serde_json::from_str(&line) {
            out.push(t);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use crate::db::tests::temp;
    use crate::history::History;
    use crate::memory::MemoryStore;
    use crate::tasks::TaskStore;

    fn turn(ts: u64, text: &str) -> String {
        format!(r#"{{"ts":{ts},"role":"user","text":"{text}","backend":"claude"}}"#) + "\n"
    }

    #[test]
    fn old_files_move_into_the_database_once() {
        let d = temp("import");
        std::fs::write(d.join("memory.md"), "- I like tea.\n- I work at Rymera.\n").unwrap();
        std::fs::write(
            d.join("tasks.json"),
            // The second task is from an old version without notes and done_at.
            r#"[{"id":"t5","title":"Pay invoice","notes":"by Friday","done":true,"added_by":"you","created":3,"done_at":4},
                {"id":"t1","title":"Old","done":false,"added_by":"aifred","created":1}]"#,
        )
        .unwrap();
        std::fs::write(d.join("history-200.jsonl"), turn(150, "second chat")).unwrap();
        std::fs::write(
            d.join("history-100.jsonl"),
            turn(50, "first chat") + "{ broken line\n",
        )
        .unwrap();
        std::fs::write(d.join("history.jsonl"), turn(300, "now")).unwrap();
        for name in ["tasks.lock", "github-cache.json", "clickup-cache.json"] {
            std::fs::write(d.join(name), "{}").unwrap();
        }

        assert_eq!(
            MemoryStore::new(&d).core_text().unwrap(),
            "- I like tea.\n- I work at Rymera."
        );
        let tasks = TaskStore::new(&d).load().unwrap();
        let ids: Vec<_> = tasks
            .iter()
            .map(|t| (t.id.as_str(), t.done, t.done_at))
            .collect();
        assert_eq!(
            ids,
            vec![("t5", true, Some(4)), ("t1", false, None)],
            "same ids and order"
        );
        assert_eq!(tasks[0].notes, "by Friday");
        assert_eq!(tasks[1].notes, "");
        let now: Vec<_> = History::new(&d)
            .load()
            .unwrap()
            .into_iter()
            .map(|t| t.text)
            .collect();
        assert_eq!(now, vec!["now"]);
        let old: Vec<(i64, i64, String)> = crate::db::read(&d, |c| {
            let mut q = c.prepare(
                "SELECT c.started, c.ended, t.text FROM conversation c JOIN turn t ON t.conversation_id = c.id
                 WHERE c.ended IS NOT NULL ORDER BY c.id",
            )?;
            Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?.collect::<Result<_, _>>()?)
        })
        .unwrap();
        assert_eq!(
            old,
            vec![
                (50, 100, "first chat".into()),
                (150, 200, "second chat".into())
            ]
        );

        let mut names: Vec<_> = std::fs::read_dir(&d)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        names.retain(|n| !n.starts_with("aifred.db"));
        names.sort();
        assert_eq!(
            names,
            vec![
                "history-100.jsonl.imported",
                "history-200.jsonl.imported",
                "history.jsonl.imported",
                "memory.md.imported",
                "tasks.json.imported"
            ]
        );

        // A new memory.md after the import is not imported again.
        std::fs::write(d.join("memory.md"), "- new\n").unwrap();
        assert_eq!(
            MemoryStore::new(&d).core_text().unwrap(),
            "- I like tea.\n- I work at Rymera."
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_broken_file_stops_the_import_and_saves_nothing() {
        let d = temp("import-broken");
        std::fs::write(d.join("memory.md"), "- I like tea.\n").unwrap();
        std::fs::write(d.join("tasks.json"), "{oops").unwrap();
        let e = format!("{:#}", TaskStore::new(&d).load().unwrap_err());
        assert!(e.contains("tasks.json"), "the error names the file: {e}");
        assert!(
            TaskStore::new(&d).add("New", "", "you").is_err(),
            "nothing works until the file is fixed"
        );
        assert_eq!(
            std::fs::read_to_string(d.join("tasks.json")).unwrap(),
            "{oops",
            "the file is not changed"
        );
        assert!(d.join("memory.md").exists(), "no file is renamed");

        std::fs::write(d.join("tasks.json"), "[]").unwrap();
        assert!(TaskStore::new(&d).load().unwrap().is_empty());
        assert_eq!(
            MemoryStore::new(&d).core_text().unwrap(),
            "- I like tea.",
            "the import runs again, once"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
