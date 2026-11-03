//! Safe JSON files in the data folder. The desktop app and the tool servers
//! can change the same file at the same time, so every change takes a lock.

use std::path::Path;

use anyhow::Context;
use serde::{Serialize, de::DeserializeOwned};

/// Reads a JSON file. A missing file gives the default value.
/// A broken file is an error, so nobody overwrites data we cannot read.
pub fn load<T: DeserializeOwned + Default>(path: &Path) -> anyhow::Result<T> {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).with_context(|| format!("cannot read {}", path.display())),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(T::default()),
        Err(e) => Err(e).with_context(|| format!("cannot read {}", path.display())),
    }
}

/// The time now, in Unix seconds.
pub fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or_default()
}

/// Writes to a temporary file first, so a crash never leaves half a file.
/// The data is on the disk before the rename, so a power cut cannot leave an
/// empty file. The temporary name includes the process id, so two programs
/// never share it. If a step fails, the temporary file is removed.
pub fn save<T: Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let bytes = serde_json::to_vec_pretty(value)?;
    let result = write_synced(&tmp, &bytes)
        .with_context(|| format!("cannot write {}", tmp.display()))
        .and_then(|()| std::fs::rename(&tmp, path).with_context(|| format!("cannot write {}", path.display())));
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
        return result;
    }
    // Also put the rename on the disk. This can fail on some systems; the
    // file is already correct, so we ignore that.
    if let Some(dir) = path.parent().and_then(|d| std::fs::File::open(d).ok()) {
        let _ = dir.sync_all();
    }
    Ok(())
}

fn write_synced(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut f = std::fs::File::create(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

fn lock(path: &Path) -> anyhow::Result<std::fs::File> {
    let lock_path = path.with_extension("lock");
    let lock = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&lock_path)
        .with_context(|| format!("cannot open {}", lock_path.display()))?;
    lock.lock().with_context(|| format!("cannot lock {}", lock_path.display()))?;
    Ok(lock)
}

/// Replaces the whole file, with the lock. It does not read the old file,
/// so it also repairs a broken file.
pub fn replace<T: Serialize + ?Sized>(path: &Path, value: &T) -> anyhow::Result<()> {
    let _lock = lock(path)?;
    save(path, value)
}

/// Read, change, and save, with the file locked for the whole step.
/// Without the lock, one program can overwrite the change of another.
/// The lock is on a separate file, because save() replaces the JSON file.
/// If `f` fails, nothing is saved.
pub fn update<T, R>(path: &Path, f: impl FnOnce(&mut T) -> anyhow::Result<R>) -> anyhow::Result<R>
where
    T: Serialize + DeserializeOwned + Default,
{
    let _lock = lock(path)?;
    let mut value = load(path)?;
    let out = f(&mut value)?;
    save(path, &value)?;
    Ok(out)
    // The lock is released when `_lock` is dropped.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replace_repairs_a_broken_file_and_leaves_no_temp_file() {
        let d = std::env::temp_dir().join(format!("aifred-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let p = d.join("x.json");
        std::fs::write(&p, "not json").unwrap();
        assert!(load::<Vec<u32>>(&p).is_err());
        replace(&p, &vec![1u32, 2]).unwrap();
        assert_eq!(load::<Vec<u32>>(&p).unwrap(), vec![1, 2]);
        let names: Vec<_> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(!names.iter().any(|n| n.to_string_lossy().ends_with(".tmp")), "{names:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
