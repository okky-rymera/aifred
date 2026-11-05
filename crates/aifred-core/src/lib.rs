//! aifred core: memory, history, tasks, tools, and swappable AI backends.

pub mod assistant;
pub mod backend;
pub mod brief;
pub mod clickup;
pub mod clickup_list;
pub mod db;
pub mod embed;
pub mod files;
pub mod github;
pub mod github_list;
pub mod history;
mod import;
pub mod items;
pub mod jobs;
pub mod memory;
pub mod overview;
pub mod plan;
pub mod program;
pub mod settings;
pub mod side;
pub mod summaries;
pub mod tasks;
pub mod text;
pub mod update;
pub mod uploads;

pub use assistant::{Answer, Assistant, default_data_dir};
