//! The AIfred program file. One file holds all parts of AIfred:
//!
//! - `aifred`: the window.
//! - `aifred ask | chat | remember | new | index`: the terminal commands.
//! - `aifred tool <name>`: one AI tool server. Claude Code starts it.
//! - `aifred embed-helper <data folder> <socket>`: the meaning search helper.
//!
//! The tool servers and the helper still run as their own processes. Only
//! the file is shared.

use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The first word for a tool server: `aifred tool <name>`.
pub const TOOL: &str = "tool";
/// The first word for the meaning search helper.
pub const EMBED_HELPER: &str = "embed-helper";

static PATH: OnceLock<PathBuf> = OnceLock::new();

/// Saves the path of the running AIfred program. The main function calls it
/// first. Test programs do not call it, so they never start a copy of
/// themselves as the helper.
pub fn set_path(path: PathBuf) {
    let _ = PATH.set(path);
}

/// The path that set_path saved, if any.
pub fn path() -> Option<&'static Path> {
    PATH.get().map(PathBuf::as_path)
}
