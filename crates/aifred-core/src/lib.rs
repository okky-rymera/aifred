//! aifred core: memory, history, tasks, tools, and swappable AI backends.

pub mod assistant;
pub mod backend;
pub mod clickup;
pub mod clickup_list;
pub mod files;
pub mod github;
pub mod github_list;
pub mod history;
pub mod settings;
pub mod tasks;

pub use assistant::{Answer, Assistant, default_data_dir};
