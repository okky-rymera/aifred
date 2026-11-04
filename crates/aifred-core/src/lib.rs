//! aifred core: memory, history, tasks, tools, and swappable AI backends.

pub mod assistant;
pub mod backend;
pub mod clickup;
pub mod clickup_list;
pub mod db;
pub mod embed;
pub mod files;
pub mod github;
pub mod github_list;
pub mod history;
mod import;
pub mod memory;
pub mod overview;
pub mod program;
pub mod settings;
pub mod summaries;
pub mod tasks;
pub mod text;

pub use assistant::{Answer, Assistant, default_data_dir};
