//! Memory and chat history tools for aifred, served over MCP (stdio).
//!
//! The AI uses them to find facts about the user and old chats, so the full
//! memory and history do not need to be in every message.
//! These tools only touch the memory and the history in the aifred database
//! ($AIFRED_HOME/aifred.db, or ~/.aifred/aifred.db).

use aifred_core::default_data_dir;
use aifred_core::text::cut;
use aifred_core::history::{History, HistoryHit, Turn};
use aifred_core::memory::{Fact, MemoryStore};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt,
    handler::server::wrapper::Parameters,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Old answers can be long. Each message in a search result is cut to this size.
const MAX_MESSAGE_CHARS: usize = 1000;
/// Summaries in each search_history result. Each one covers a whole conversation.
const SUMMARY_RESULTS: u32 = 3;

#[derive(Clone)]
struct MemoryTools {
    dir: std::path::PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Words to find. Example: "dentist appointment".
    query: String,
    /// Maximum number of results (1-50). Default 10.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct AddArgs {
    /// One short fact about the user, in one sentence. Example: "Sam sends the hosting invoice each month."
    text: String,
    /// True for a fact that matters in most chats, for example how the user wants answers.
    /// Such a fact goes into every message. Default false: you find the fact with search_memory.
    always: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    /// Fact id, from search_memory.
    id: i64,
}

fn fail(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn ok_json(v: &Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(v.to_string())]))
}

fn fact(f: &Fact) -> Value {
    json!({ "id": f.id, "text": f.text, "always": f.core, "added_by": f.added_by, "updated": f.updated })
}

fn message(t: &Turn) -> Value {
    json!({ "ts": t.ts, "role": t.role, "text": cut(&t.text, MAX_MESSAGE_CHARS) })
}

fn hit(h: &HistoryHit) -> Value {
    let mut v = json!({ "conversation": h.conversation, "current": h.current, "message": message(&h.hit) });
    if let Some(b) = &h.before {
        v["before"] = message(b);
    }
    if let Some(a) = &h.after {
        v["after"] = message(a);
    }
    v
}

#[tool_router]
impl MemoryTools {
    #[tool(description = "Search the facts that you know about the user, best match first. A fact matches when it has any of the words, also other forms of a word. Each fact has an id for forget_memory.")]
    async fn search_memory(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, ErrorData> {
        let facts = MemoryStore::new(&self.dir).search(&a.query, a.limit).map_err(fail)?;
        ok_json(&json!(facts.iter().map(fact).collect::<Vec<_>>()))
    }

    #[tool(description = "Save one lasting fact about the user or their work. Use it only when the user tells you such a fact. Search first, so you do not save the same fact two times.")]
    async fn add_memory(&self, Parameters(a): Parameters<AddArgs>) -> Result<CallToolResult, ErrorData> {
        let f = MemoryStore::new(&self.dir).add(&a.text, "aifred", a.always.unwrap_or(false)).map_err(fail)?;
        ok_json(&fact(&f))
    }

    #[tool(description = "Forget one fact, for example when the user says it is wrong or old. The user can restore it in the AIfred app.")]
    async fn forget_memory(&self, Parameters(a): Parameters<IdArgs>) -> Result<CallToolResult, ErrorData> {
        let f = MemoryStore::new(&self.dir).forget(a.id).map_err(fail)?;
        ok_json(&json!({ "forgotten": f.id, "text": f.text }))
    }

    #[tool(description = "Search all conversations with the user, also old ones, best match first. \"summaries\" are short summaries of whole conversations. \"messages\" are single messages, each with the message before and after it. Times are Unix seconds. \"current\" is true for the conversation that is open now.")]
    async fn search_history(&self, Parameters(a): Parameters<SearchArgs>) -> Result<CallToolResult, ErrorData> {
        let h = History::new(&self.dir);
        let summaries = h.search_summaries(&a.query, Some(SUMMARY_RESULTS)).map_err(fail)?;
        let hits = h.search(&a.query, a.limit).map_err(fail)?;
        ok_json(&json!({ "summaries": summaries, "messages": hits.iter().map(hit).collect::<Vec<_>>() }))
    }
}

#[tool_handler]
impl ServerHandler for MemoryTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Serves these tools over stdio until Claude Code closes the connection.
pub async fn serve() -> anyhow::Result<()> {
    let dir = default_data_dir()?;
    std::fs::create_dir_all(&dir)?;
    let server = MemoryTools { dir }.serve(stdio()).await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tools(name: &str) -> MemoryTools {
        let dir = std::env::temp_dir().join(format!("aifred-memory-mcp-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        MemoryTools { dir }
    }

    /// The JSON text that the tool sends back to Claude.
    fn body(r: CallToolResult) -> Value {
        let v = serde_json::to_value(r).unwrap();
        serde_json::from_str(v["content"][0]["text"].as_str().unwrap()).unwrap()
    }

    fn search(q: &str) -> Parameters<SearchArgs> {
        Parameters(SearchArgs { query: q.into(), limit: None })
    }

    #[tokio::test]
    async fn add_search_and_forget_a_fact() {
        let t = tools("facts");
        let added = body(
            t.add_memory(Parameters(AddArgs { text: "Sam sends the hosting invoice".into(), always: None }))
                .await
                .unwrap(),
        );
        assert_eq!(added["added_by"], "aifred");
        assert_eq!(added["always"], false, "not in every message unless asked");
        let found = body(t.search_memory(search("invoices")).await.unwrap());
        assert_eq!(found[0]["text"], "Sam sends the hosting invoice");
        let id = found[0]["id"].as_i64().unwrap();
        t.forget_memory(Parameters(IdArgs { id })).await.unwrap();
        assert!(body(t.search_memory(search("invoice")).await.unwrap()).as_array().unwrap().is_empty());
        assert!(t.forget_memory(Parameters(IdArgs { id: 999 })).await.is_err());
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn search_history_cuts_long_messages() {
        let t = tools("history");
        let h = History::new(&t.dir);
        h.append("user", "Tell me about the dentist", "claude").unwrap();
        h.append("assistant", &format!("dentist {}", "x".repeat(MAX_MESSAGE_CHARS)), "claude").unwrap();
        let id = h.current().unwrap().unwrap();
        h.set_summary(id, "- The user asked about the dentist.", 2).unwrap();
        let out = body(t.search_history(search("dentist")).await.unwrap());
        assert_eq!(out["summaries"][0]["summary"], "- The user asked about the dentist.");
        assert!(out["summaries"][0]["ended"].is_null(), "the current conversation");
        let hits = &out["messages"];
        let long = hits.as_array().unwrap().iter().find(|h| h["message"]["role"] == "assistant").unwrap();
        assert!(long["message"]["text"].as_str().unwrap().ends_with("[... shortened]"));
        assert_eq!(long["before"]["text"], "Tell me about the dentist");
        std::fs::remove_dir_all(&t.dir).unwrap();
    }
}
