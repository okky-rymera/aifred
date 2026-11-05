//! The AI tool servers of AIfred, served over MCP (stdio).
//!
//! They are part of the AIfred program. Claude Code starts one as
//! `aifred tool <name>` for each message, and stops it after the answer.
//!
//! - `tasks`: the local tasks, and the overview of all work items.
//! - `memory`: the facts about the user and the old chats.
//! - `github`: read-only GitHub search, with the user's `gh` login.
//! - `clickup`: read-only ClickUp tasks, with the saved ClickUp token.

pub mod clickup;
pub mod github;
pub mod memory;
pub mod tasks;

/// Runs the tool server with this name until Claude Code closes the connection.
pub fn run(name: &str) -> anyhow::Result<()> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    runtime.block_on(async {
        match name {
            "tasks" => tasks::serve().await,
            "memory" => memory::serve().await,
            "github" => github::serve().await,
            "clickup" => clickup::serve().await,
            _ => anyhow::bail!("unknown tool server: {name}"),
        }
    })
}
