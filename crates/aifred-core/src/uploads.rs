//! Files that the user sends with a chat message or keeps on a task.
//!
//! Each file goes into its own folder: `<data folder>/uploads/<id>/<name>`.
//! The AI gets only the path. Claude reads the file with its Read tool, and
//! only inside the folders of the files of that message (backend/claude.rs).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};

/// The largest file that the app takes: 25 MB.
pub const MAX_BYTES: usize = 25 * 1024 * 1024;

/// One saved file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FileRef {
    /// The folder name in uploads. Only letters and digits.
    pub id: String,
    /// The file name, as the user's computer gave it.
    pub name: String,
    /// The size in bytes.
    pub size: u64,
}

fn root(dir: &Path) -> PathBuf {
    dir.join("uploads")
}

/// A name that is safe as one file name: no folders, no control characters,
/// at most 120 characters. An empty name becomes "file".
fn safe_name(name: &str) -> String {
    let last = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let clean: String = last.chars().filter(|c| !c.is_control()).take(120).collect();
    let clean = clean.trim().trim_start_matches('.').trim();
    if clean.is_empty() {
        "file".into()
    } else {
        clean.into()
    }
}

/// An id is a folder name that we made: "f" and digits. Any other text
/// could point out of the uploads folder, so it is an error.
fn check_id(id: &str) -> anyhow::Result<()> {
    let ok = id.len() > 1 && id.starts_with('f') && id[1..].bytes().all(|b| b.is_ascii_digit());
    if !ok {
        bail!("\"{id}\" is not a file id");
    }
    Ok(())
}

/// Saves the bytes of one file. Returns its id, name, and size.
pub fn save(dir: &Path, name: &str, bytes: &[u8]) -> anyhow::Result<FileRef> {
    if bytes.len() > MAX_BYTES {
        bail!(
            "the file is too large ({} MB). The limit is {} MB.",
            bytes.len() / (1024 * 1024),
            MAX_BYTES / (1024 * 1024)
        );
    }
    let name = safe_name(name);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or_default();
    let mut n = 0u32;
    // Two files in the same nanosecond get different folders.
    let (id, folder) = loop {
        let id = format!("f{nanos}{n}");
        let folder = root(dir).join(&id);
        match std::fs::create_dir_all(root(dir)).and_then(|_| std::fs::create_dir(&folder)) {
            Ok(()) => break (id, folder),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => n += 1,
            Err(e) => return Err(e).with_context(|| format!("cannot make {}", folder.display())),
        }
    };
    let path = folder.join(&name);
    std::fs::write(&path, bytes).with_context(|| format!("cannot save {}", path.display()))?;
    Ok(FileRef {
        id,
        name,
        size: bytes.len() as u64,
    })
}

/// The saved file with this id.
pub fn get(dir: &Path, id: &str) -> anyhow::Result<FileRef> {
    let path = path(dir, id)?;
    let size = std::fs::metadata(&path)?.len();
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    Ok(FileRef {
        id: id.into(),
        name,
        size,
    })
}

/// The full path of the saved file with this id.
pub fn path(dir: &Path, id: &str) -> anyhow::Result<PathBuf> {
    check_id(id)?;
    let folder = root(dir).join(id);
    let mut files = std::fs::read_dir(&folder).with_context(|| format!("no file with id {id}"))?;
    match files.next() {
        Some(f) => Ok(f?.path()),
        None => bail!("no file with id {id}"),
    }
}

/// The folder of the file with this id. The AI may read only in it.
pub fn folder(dir: &Path, id: &str) -> anyhow::Result<PathBuf> {
    check_id(id)?;
    let f = root(dir).join(id);
    if !f.is_dir() {
        bail!("no file with id {id}");
    }
    Ok(f)
}

/// The text that tells the AI about the files of a message.
pub fn note(files: &[(FileRef, PathBuf)]) -> String {
    if files.is_empty() {
        return String::new();
    }
    let mut s = String::from("[The user attached these files. Read them with the Read tool:\n");
    for (f, p) in files {
        s.push_str(&format!(
            "- {} ({} bytes, id {}): {}\n",
            f.name,
            f.size,
            f.id,
            p.display()
        ));
    }
    s.push_str("]\n");
    s
}

/// A file that no message or task uses is deleted only after this time.
/// A file that the user just attached waits in the chat box, not in the
/// database, until the user sends it.
pub const KEEP_UNUSED: Duration = Duration::from_secs(24 * 60 * 60);

/// The ids of the files that a task, a main chat message, or a side chat
/// message uses.
fn used(dir: &Path) -> anyhow::Result<HashSet<String>> {
    crate::db::read(dir, |c| {
        let mut q = c.prepare(
            "SELECT json_extract(j.value, '$.id') FROM task, json_each(task.files) j WHERE task.files != '[]'
             UNION SELECT json_extract(j.value, '$.id') FROM turn, json_each(turn.files) j WHERE turn.files != '[]'
             UNION SELECT json_extract(j.value, '$.id') FROM side_turn, json_each(side_turn.meta, '$.files') j
               WHERE side_turn.meta LIKE '%\"files\"%'",
        )?;
        let ids = q.query_map([], |r| r.get::<_, Option<String>>(0))?;
        Ok(ids
            .filter_map(|r| r.transpose())
            .collect::<Result<_, _>>()?)
    })
}

/// Deletes the saved files that no task and no message uses, and that are
/// older than `keep`. For example the files of a deleted task, or a file
/// that the user removed from the chat box. Returns how many it deleted.
pub fn sweep(dir: &Path, keep: Duration) -> anyhow::Result<usize> {
    let Ok(list) = std::fs::read_dir(root(dir)) else {
        return Ok(0);
    };
    let used = used(dir)?;
    let mut n = 0;
    for e in list.flatten() {
        let id = e.file_name().to_string_lossy().into_owned();
        if check_id(&id).is_err() || used.contains(&id) {
            continue;
        }
        let age = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        if age.is_some_and(|a| a >= keep) && std::fs::remove_dir_all(e.path()).is_ok() {
            n += 1;
        }
    }
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-uploads-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn a_file_is_saved_and_found_again() {
        let d = temp("save");
        let f = save(&d, "notes.txt", b"hello").unwrap();
        assert_eq!((f.name.as_str(), f.size), ("notes.txt", 5));
        assert_eq!(get(&d, &f.id).unwrap(), f);
        assert_eq!(std::fs::read(path(&d, &f.id).unwrap()).unwrap(), b"hello");
        let g = save(&d, "notes.txt", b"again").unwrap();
        assert_ne!(f.id, g.id, "the same name twice gets two folders");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_name_cannot_leave_its_folder() {
        assert_eq!(safe_name("../../etc/passwd"), "passwd");
        assert_eq!(safe_name("C:\\Users\\me\\a.pdf"), "a.pdf");
        assert_eq!(safe_name(".."), "file");
        assert_eq!(safe_name(""), "file");
        assert_eq!(safe_name(".hidden"), "hidden");
    }

    #[test]
    fn a_bad_id_is_an_error() {
        let d = temp("id");
        for bad in ["", "f", "../x", "f12/../../a", "x123"] {
            assert!(path(&d, bad).is_err(), "{bad}");
            assert!(folder(&d, bad).is_err(), "{bad}");
        }
        assert!(path(&d, "f123").is_err(), "an id with no folder");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn sweep_deletes_only_old_unused_files() {
        let d = temp("sweep");
        let kept = save(&d, "task.txt", b"a").unwrap();
        let side = save(&d, "side.txt", b"b").unwrap();
        let unused = save(&d, "gone.txt", b"c").unwrap();
        let tasks = crate::tasks::TaskStore::new(&d);
        let t = tasks.add("With a file", "", "you").unwrap();
        tasks
            .update(
                &t.id,
                crate::tasks::TaskFields {
                    files: Some(vec![kept.id.clone()]),
                    ..Default::default()
                },
            )
            .unwrap();
        crate::side::append_pair(
            &d,
            "plan:2026-11-05",
            "look",
            std::slice::from_ref(&side),
            "ok",
            "",
        )
        .unwrap();
        assert_eq!(sweep(&d, KEEP_UNUSED).unwrap(), 0, "new files stay");
        assert_eq!(sweep(&d, Duration::ZERO).unwrap(), 1);
        assert!(folder(&d, &kept.id).is_ok() && folder(&d, &side.id).is_ok());
        assert!(folder(&d, &unused.id).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_large_file_is_an_error() {
        let d = temp("large");
        assert!(save(&d, "big.bin", &vec![0u8; MAX_BYTES + 1]).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
