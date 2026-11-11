//! Memory and chat history tools for aifred, served over MCP (stdio).
//!
//! The AI uses them to find facts about the user and old chats, so the full
//! memory and history do not need to be in every message.
//! These tools only touch the memory and the history in the aifred database
//! ($AIFRED_HOME/aifred.db, or ~/.aifred/aifred.db).

use aifred_core::default_data_dir;
use aifred_core::history::{History, HistoryHit, Turn};
use aifred_core::memory::{Fact, MemoryStore};
use aifred_core::periods;
use aifred_core::text::cut;
use rmcp::{
    ErrorData, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router, transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

/// Old answers can be long. Each message in a search result is cut to this size.
const MAX_MESSAGE_CHARS: usize = 1000;
/// Summaries in each search_history result. Each one covers a day or a quarter (periods.rs).
const SUMMARY_RESULTS: u32 = 3;
/// day_summaries gives at most this many days in one call.
const DAYS_MAX: i64 = 31;
/// day_messages gives at most this many messages in one call.
const DAY_MESSAGES_MAX: usize = 100;

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
    /// 1 to 3 short topic tags, for example ["testing", "people"]. Use the tags from memory_tags when one fits.
    tags: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
struct TagArgs {
    /// One tag, from memory_tags. Example: "testing".
    tag: String,
}

#[derive(Deserialize, JsonSchema)]
struct SetTagsArgs {
    /// Fact id, from search_memory or facts_with_tag.
    id: i64,
    /// The full new list of tags, at most 5. It replaces the old tags. [] removes them.
    tags: Vec<String>,
}

#[derive(Deserialize, JsonSchema)]
struct IdArgs {
    /// Fact id, from search_memory.
    id: i64,
}

#[derive(Deserialize, JsonSchema)]
struct DaysArgs {
    /// The first day, "YYYY-MM-DD".
    from: String,
    /// The last day, "YYYY-MM-DD", at most 30 days after `from` (31 days in all). Default: the same day as `from`.
    to: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct DayArgs {
    /// The day, "YYYY-MM-DD".
    day: String,
    /// Skip this many messages first. Default 0. The result says when there are more.
    offset: Option<u32>,
}

fn fail(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn ok_json(v: &Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        v.to_string(),
    )]))
}

fn fact(f: &Fact) -> Value {
    json!({ "id": f.id, "text": f.text, "tags": f.tags, "always": f.core, "added_by": f.added_by, "updated": f.updated })
}

fn message(t: &Turn) -> Value {
    json!({ "ts": t.ts, "role": t.role, "text": cut(&t.text, MAX_MESSAGE_CHARS) })
}

fn hit(h: &HistoryHit) -> Value {
    let mut v =
        json!({ "conversation": h.conversation, "current": h.current, "message": message(&h.hit) });
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
    #[tool(
        description = "Search the facts that you know about the user, best match first. A fact matches when it has any of the words, also other forms of a word. Each fact has an id for forget_memory."
    )]
    async fn search_memory(
        &self,
        Parameters(a): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let facts = MemoryStore::new(&self.dir)
            .search(&a.query, a.limit)
            .map_err(fail)?;
        ok_json(&json!(facts.iter().map(fact).collect::<Vec<_>>()))
    }

    #[tool(
        description = "Save one lasting fact about the user or their work. Use it only when the user tells you such a fact. Search first, so you do not save the same fact two times."
    )]
    async fn add_memory(
        &self,
        Parameters(a): Parameters<AddArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let f = MemoryStore::new(&self.dir)
            .add_tagged(
                &a.text,
                "aifred",
                a.always.unwrap_or(false),
                &a.tags.unwrap_or_default(),
            )
            .map_err(fail)?;
        ok_json(&fact(&f))
    }

    #[tool(
        description = "List the tags of the facts about the user, with the number of facts for each tag, the most used first. Use it before add_memory, so you reuse a tag that fits."
    )]
    async fn memory_tags(&self) -> Result<CallToolResult, ErrorData> {
        let tags = MemoryStore::new(&self.dir).tags().map_err(fail)?;
        ok_json(&json!(tags))
    }

    #[tool(
        description = "Read all facts about the user that have one tag, for example all \"testing\" facts. Use it when a topic needs more facts than search_memory gives."
    )]
    async fn facts_with_tag(
        &self,
        Parameters(a): Parameters<TagArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let facts = MemoryStore::new(&self.dir).with_tag(&a.tag).map_err(fail)?;
        ok_json(&json!(facts.iter().map(fact).collect::<Vec<_>>()))
    }

    #[tool(
        description = "Change the tags of one fact, for example when the user asks for it, or to merge two tags with the same meaning. The new list replaces the old one."
    )]
    async fn tag_memory(
        &self,
        Parameters(a): Parameters<SetTagsArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let f = MemoryStore::new(&self.dir)
            .set_tags(a.id, &a.tags)
            .map_err(fail)?;
        ok_json(&fact(&f))
    }

    #[tool(
        description = "Forget one fact, for example when the user says it is wrong or old. The user can restore it in the AIfred app."
    )]
    async fn forget_memory(
        &self,
        Parameters(a): Parameters<IdArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let f = MemoryStore::new(&self.dir).forget(a.id).map_err(fail)?;
        ok_json(&json!({ "forgotten": f.id, "text": f.text }))
    }

    #[tool(
        description = "Search all chats with the user, also old ones, best match first. \"summaries\" are short summaries of one day (period \"YYYY-MM-DD\") or one quarter (period \"YYYY-Qn\"). \"messages\" are single messages of the main chat, each with the message before and after it. Times are Unix seconds. \"current\" is true for the conversation that is open now. For all messages of a day, use day_messages."
    )]
    async fn search_history(
        &self,
        Parameters(a): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let summaries =
            periods::search(&self.dir, &a.query, Some(SUMMARY_RESULTS)).map_err(fail)?;
        let hits = History::new(&self.dir)
            .search(&a.query, a.limit)
            .map_err(fail)?;
        ok_json(
            &json!({ "summaries": summaries, "messages": hits.iter().map(hit).collect::<Vec<_>>() }),
        )
    }

    #[tool(
        description = "The days from `from` to `to` (both included, at most 31 days) that have chats, oldest first, each with its summary and its number of messages. Use it for a range of days, for example last week. A day with only a few short messages, and today, have no summary (null): day_messages has their messages."
    )]
    async fn day_summaries(
        &self,
        Parameters(a): Parameters<DaysArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let to = a.to.as_deref().unwrap_or(&a.from);
        let (f, t) = (
            periods::check_day(&a.from).map_err(fail)?,
            periods::check_day(to).map_err(fail)?,
        );
        if t < f || (t - f).num_days() >= DAYS_MAX {
            return Err(ErrorData::invalid_params(
                format!("the range must be from 1 to {DAYS_MAX} days, with `from` first"),
                None,
            ));
        }
        let days = periods::days(&self.dir, &a.from, to).map_err(fail)?;
        ok_json(&json!(days))
    }

    #[tool(
        description = "All messages of one day, oldest first: the main chat (\"chat\": \"main\") and the side chats (the plan chat of a day, the chat of an item, the chat of a mission). Use it when the summary of the day is not enough. Each message is cut to 1,000 characters. At most 100 messages in one call: when \"more\" is true, call again with `offset`."
    )]
    async fn day_messages(
        &self,
        Parameters(a): Parameters<DayArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let all = periods::messages_of_day(&self.dir, &a.day).map_err(fail)?;
        let offset = a.offset.unwrap_or(0) as usize;
        let page: Vec<Value> = all
            .iter()
            .skip(offset)
            .take(DAY_MESSAGES_MAX)
            .map(|m| {
                json!({ "ts": m.ts, "chat": m.chat, "role": m.role, "text": cut(&m.text, MAX_MESSAGE_CHARS) })
            })
            .collect();
        let more = offset + page.len() < all.len();
        ok_json(
            &json!({ "day": a.day, "total": all.len(), "offset": offset, "messages": page, "more": more }),
        )
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
        let dir =
            std::env::temp_dir().join(format!("aifred-memory-mcp-{name}-{}", std::process::id()));
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
        Parameters(SearchArgs {
            query: q.into(),
            limit: None,
        })
    }

    #[tokio::test]
    async fn add_search_and_forget_a_fact() {
        let t = tools("facts");
        let added = body(
            t.add_memory(Parameters(AddArgs {
                text: "Sam sends the hosting invoice".into(),
                always: None,
                tags: Some(vec!["Billing".into()]),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(added["added_by"], "aifred");
        assert_eq!(added["always"], false, "not in every message unless asked");
        assert_eq!(added["tags"], json!(["billing"]));
        assert_eq!(
            body(t.memory_tags().await.unwrap()),
            json!([{ "tag": "billing", "facts": 1 }])
        );
        let tagged = body(
            t.facts_with_tag(Parameters(TagArgs {
                tag: "billing".into(),
            }))
            .await
            .unwrap(),
        );
        assert_eq!(tagged[0]["text"], "Sam sends the hosting invoice");
        let id0 = tagged[0]["id"].as_i64().unwrap();
        let retagged = body(
            t.tag_memory(Parameters(SetTagsArgs {
                id: id0,
                tags: vec!["money".into(), "people".into()],
            }))
            .await
            .unwrap(),
        );
        assert_eq!(retagged["tags"], json!(["money", "people"]));
        let found = body(t.search_memory(search("invoices")).await.unwrap());
        assert_eq!(found[0]["text"], "Sam sends the hosting invoice");
        let id = found[0]["id"].as_i64().unwrap();
        t.forget_memory(Parameters(IdArgs { id })).await.unwrap();
        assert!(
            body(t.search_memory(search("invoice")).await.unwrap())
                .as_array()
                .unwrap()
                .is_empty()
        );
        assert!(
            t.forget_memory(Parameters(IdArgs { id: 999 }))
                .await
                .is_err()
        );
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn search_history_cuts_long_messages() {
        let t = tools("history");
        let h = History::new(&t.dir);
        h.append("user", "Tell me about the dentist", "claude")
            .unwrap();
        h.append(
            "assistant",
            &format!("dentist {}", "x".repeat(MAX_MESSAGE_CHARS)),
            "claude",
        )
        .unwrap();
        periods::set_day_summary(
            &t.dir,
            "2026-11-10",
            "- The user asked about the dentist.",
            2,
            0,
        )
        .unwrap();
        let out = body(t.search_history(search("dentist")).await.unwrap());
        assert_eq!(
            out["summaries"][0]["summary"],
            "- The user asked about the dentist."
        );
        assert_eq!(out["summaries"][0]["period"], "2026-11-10");
        let hits = &out["messages"];
        let long = hits
            .as_array()
            .unwrap()
            .iter()
            .find(|h| h["message"]["role"] == "assistant")
            .unwrap();
        assert!(
            long["message"]["text"]
                .as_str()
                .unwrap()
                .ends_with("[... shortened]")
        );
        assert_eq!(long["before"]["text"], "Tell me about the dentist");
        std::fs::remove_dir_all(&t.dir).unwrap();
    }

    #[tokio::test]
    async fn the_day_tools_give_summaries_of_a_range_and_the_messages_of_a_day() {
        let t = tools("days");
        let h = History::new(&t.dir);
        h.append("user", "Remind me about the dentist", "claude")
            .unwrap();
        h.append("assistant", "Noted.", "claude").unwrap();
        aifred_core::side::append_pair(
            &t.dir,
            "item:task:a",
            "Is it urgent?",
            &[],
            "No.",
            "",
            false,
        )
        .unwrap();
        periods::set_day_summary(&t.dir, "2026-11-09", "- Bali.", 1, 0).unwrap();
        periods::set_day_summary(&t.dir, "2026-11-10", "- Dentist.", 2, 0).unwrap();
        let days = |from: &str, to: Option<&str>| {
            Parameters(DaysArgs {
                from: from.into(),
                to: to.map(String::from),
            })
        };
        let out = body(
            t.day_summaries(days("2026-11-01", Some("2026-11-30")))
                .await
                .unwrap(),
        );
        assert_eq!(out[0]["day"], "2026-11-09");
        assert_eq!(out[1]["summary"], "- Dentist.");
        let out = body(t.day_summaries(days("2026-11-10", None)).await.unwrap());
        assert_eq!(out.as_array().unwrap().len(), 1);
        // Today has messages and no summary yet.
        let today = aifred_core::plan::today();
        let out = body(t.day_summaries(days(&today, None)).await.unwrap());
        assert_eq!(
            (out[0]["summary"].is_null(), out[0]["messages"].as_u64()),
            (true, Some(4))
        );
        assert!(
            t.day_summaries(days("2026-11-10", Some("2026-11-01")))
                .await
                .is_err()
        );
        assert!(
            t.day_summaries(days("2026-11-01", Some("2026-12-31")))
                .await
                .is_err()
        );

        let day = |offset: Option<u32>| {
            Parameters(DayArgs {
                day: today.clone(),
                offset,
            })
        };
        let out = body(t.day_messages(day(None)).await.unwrap());
        assert_eq!(out["total"], 4);
        assert_eq!(out["more"], false);
        let chats: Vec<&str> = out["messages"]
            .as_array()
            .unwrap()
            .iter()
            .map(|m| m["chat"].as_str().unwrap())
            .collect();
        assert_eq!(chats, vec!["main", "main", "item:task:a", "item:task:a"]);
        let out = body(t.day_messages(day(Some(3))).await.unwrap());
        assert_eq!(out["messages"][0]["text"], "No.");
        // A page has at most 100 messages: 4 so far, 101 more.
        for i in 0..101 {
            h.append("user", &format!("Message {i}"), "claude").unwrap();
        }
        let out = body(t.day_messages(day(None)).await.unwrap());
        assert_eq!(
            (
                out["total"].as_u64(),
                out["messages"].as_array().unwrap().len(),
                out["more"].as_bool()
            ),
            (Some(105), 100, Some(true))
        );
        let out = body(t.day_messages(day(Some(100))).await.unwrap());
        assert_eq!(
            (
                out["messages"].as_array().unwrap().len(),
                out["more"].as_bool()
            ),
            (5, Some(false))
        );
        assert!(
            t.day_messages(Parameters(DayArgs {
                day: "soon".into(),
                offset: None
            }))
            .await
            .is_err()
        );
        std::fs::remove_dir_all(&t.dir).unwrap();
    }
}
