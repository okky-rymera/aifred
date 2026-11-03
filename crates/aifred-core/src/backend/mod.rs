//! The swap point between aifred and an AI service.
//!
//! aifred owns memory, history, and the tool list. A backend only turns one
//! request into one answer. Today there is one backend (`claude`). An API
//! backend (Grok, DeepSeek, Anthropic API) goes next to it later.

pub mod claude;

use std::path::PathBuf;

use crate::history::Turn;

/// One MCP tool server that the backend may use.
#[derive(Debug, Clone)]
pub struct ToolServer {
    /// Short name. Claude shows tools as `mcp__<name>__<tool>`.
    pub name: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    /// Tool names this server may run. Everything else is denied.
    pub allowed_tools: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AskRequest {
    /// Rules plus memory. Rebuilt on every call so memory edits apply at once.
    pub system: String,
    /// Recent turns that aifred stored. A backend uses them when it cannot
    /// continue its own session.
    pub history: Vec<Turn>,
    pub prompt: String,
    pub tools: Vec<ToolServer>,
    /// Backend-specific conversation id from the last answer, if any.
    pub session: Option<String>,
}

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct AskStats {
    /// Time from start until the backend sent its first event.
    pub startup_ms: u128,
    pub total_ms: u128,
    /// Time spent waiting on the model, as the backend reports it.
    pub model_ms: Option<u64>,
    pub cost_usd: Option<f64>,
    pub tool_calls: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AskResponse {
    pub text: String,
    pub session: Option<String>,
    pub stats: AskStats,
}

/// Progress that a backend reports while it builds an answer.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    /// The model started to use a tool.
    ToolCall { server: String, tool: String, input: serde_json::Value },
    /// A backend notice, for example a session that could not resume.
    Notice { message: String },
}

/// Callback for progress events.
pub type OnEvent<'a> = &'a mut (dyn FnMut(Event) + Send);

pub trait Backend: Send + Sync {
    fn name(&self) -> &'static str;
    fn ask(
        &self,
        req: &AskRequest,
        on_event: OnEvent<'_>,
    ) -> impl std::future::Future<Output = anyhow::Result<AskResponse>> + Send;
}
