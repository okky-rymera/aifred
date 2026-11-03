//! Conversation history that aifred owns, stored as one JSON line per turn.
//! It does not depend on any backend, so a backend switch keeps the history.

use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Turn {
    pub ts: u64,
    /// "user" or "assistant".
    pub role: String,
    pub text: String,
    pub backend: String,
}

pub struct History {
    path: PathBuf,
}

impl History {
    pub fn new(dir: &Path) -> Self {
        Self { path: dir.join("history.jsonl") }
    }

    pub fn load(&self) -> anyhow::Result<Vec<Turn>> {
        let Ok(f) = std::fs::File::open(&self.path) else { return Ok(vec![]) };
        Ok(std::io::BufReader::new(f)
            .lines()
            .map_while(Result::ok)
            .filter_map(|l| serde_json::from_str(&l).ok())
            .collect())
    }

    pub fn append(&self, role: &str, text: &str, backend: &str) -> anyhow::Result<()> {
        let turn = Turn {
            ts: SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs(),
            role: role.into(),
            text: text.into(),
            backend: backend.into(),
        };
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(&self.path)?;
        writeln!(f, "{}", serde_json::to_string(&turn)?)?;
        Ok(())
    }

    /// Starts a new conversation. The old file is kept with a timestamp.
    pub fn archive(&self) -> anyhow::Result<()> {
        if self.path.exists() {
            let ts = SystemTime::now().duration_since(UNIX_EPOCH)?.as_secs();
            std::fs::rename(&self.path, self.path.with_file_name(format!("history-{ts}.jsonl")))?;
        }
        Ok(())
    }
}
