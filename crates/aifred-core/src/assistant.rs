//! The assistant: memory, history, tools, and one backend.
//! The terminal commands and the desktop app both use this.

use std::path::{Path, PathBuf};

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::backend::{AskRequest, AskStats, Backend, Event, OnEvent, ToolServer};
use crate::history::{History, Turn};
use crate::memory::MemoryStore;
use crate::tasks::TaskStore;

const BASE_RULES: &str = "\
You are aifred, the user's personal assistant.
- Use the tools to look up facts. Do not guess about the user's work items.
- Keep answers short. Lead with what needs action, most urgent first.
- When you list items, give each one a markdown link and a one-line summary.
- You can add tasks to the user's local to-do list, change them, and mark them done.
- A local task can have a due day, a start day, a priority, tags, and a checklist. Set them only when the user gives them.
- A task with a start day after today waits. Do not put it in the plan for today.
- Add a task only when the user asks you to track, note, or remember to do something.
- GitHub is read-only. If the user asks for a change there, say what you would change.
- ClickUp is read-only for you. The user marks ClickUp tasks done in the AIfred to-do list.
- Search memory before you say that you do not know something about the user.
- Add a fact to memory only when the user tells you something lasting about themselves or their work. Do not add a fact that is already in memory.
- Search old chats when the user refers to an earlier conversation.
- When the user asks what to do, what is open, or for a plan, call overview first. It has all open items with a short summary each. Open the details of an item only when its summary is not enough.
- When the user asks about a topic, use search_tasks, search_my_items, or search_my_tasks.";

/// Memory up to this size goes into the AI instructions in full. Above it,
/// only the core facts go there, and each message gets the facts that match it.
const FULL_MEMORY_MAX_TOKENS: u64 = 2000;
/// How many matching facts each message gets when the memory is large.
const ADDED_FACTS: u32 = 8;

/// A long conversation gets a new summary each time this many tokens of
/// messages (4 characters count as 1 token) are not in its summary yet.
const SUMMARY_EVERY_TOKENS: u64 = 20_000;
/// An ended conversation shorter than this gets no summary: a search finds
/// its few messages directly, and the summary would cost an AI call.
const SUMMARY_MIN_CHARS: usize = 2000;
/// Each message goes into the summary call with at most this many characters.
const SUMMARY_TURN_CHARS: usize = 4000;
/// One summary call reads at most this many characters of messages (about
/// 25,000 tokens). A larger backlog, for example an imported history, gets
/// its summary in more than one call.
const SUMMARY_BATCH_CHARS: usize = 100_000;
/// summarize_old writes at most this many old conversations in one run.
/// Each one is at least one AI call, so a large import is done over a few starts.
const OLD_SUMMARIES_PER_RUN: usize = 5;

const SUMMARY_RULES: &str = "\
You write a summary of a conversation between a user and AIfred, the user's personal assistant.
- Keep decisions, facts about the user, tasks, names, dates, links, and open questions.
- If there is an earlier summary, keep its points that still matter, and add the new points.
- Write at most 250 words, as short bullet points in plain English.
- Do not add facts that are not in the text. Write only the summary.";

/// What the backend reported for one message.
#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    pub text: String,
    pub stats: AskStats,
    /// Model time and cost for this message only (not the whole conversation).
    pub model_ms: Option<u64>,
    pub cost_usd: Option<f64>,
    /// The facts that the app added to this message, because they match it.
    pub memory: Vec<String>,
    /// The model that the user picked for this message. None: the default.
    pub model: Option<String>,
}

/// Saved per backend: the session id and the running totals the backend
/// reports, so aifred can show the cost of each message.
#[derive(Debug, Default, Serialize, Deserialize)]
struct SessionState {
    id: Option<String>,
    model_ms: u64,
    cost_usd: f64,
}

pub struct Assistant<B: Backend> {
    pub dir: PathBuf,
    pub backend: B,
    /// The AIfred program. The tool servers run as `<program> tool <name>`.
    pub program: PathBuf,
    /// One summary at a time. Two at the same time would cost two AI calls.
    summary: tokio::sync::Mutex<()>,
    /// One run of the item summaries at a time, for the same reason. It holds the items with no usable summary
    /// from the AI in this run of the app, so they do not cost more calls.
    item_summaries: tokio::sync::Mutex<std::collections::HashSet<crate::summaries::Key>>,
}

/// Rules and memory only. It must stay the same between messages:
/// any change here makes the backend re-read the whole conversation
/// (no prompt cache hit). The current time goes into each message.
/// `small` is from memory_is_small: ask checks it once for each message.
fn system_prompt(dir: &Path, small: bool) -> anyhow::Result<String> {
    // A large memory: only the core facts, so a new fact that is not
    // core does not change this text.
    let memory = MemoryStore::new(dir);
    let facts = if small { memory.all_text()? } else { memory.core_text()? };
    Ok(format!("{BASE_RULES}\n\n# What you know about the user\nSearch memory for facts that are not here.\n{facts}"))
}

/// True when all facts fit in the AI instructions (4 characters count as 1 token).
fn memory_is_small(dir: &Path) -> anyhow::Result<bool> {
    Ok(MemoryStore::new(dir).size_chars()?.div_ceil(4) <= FULL_MEMORY_MAX_TOKENS)
}

/// $AIFRED_HOME, or ~/.aifred.
pub fn default_data_dir() -> anyhow::Result<PathBuf> {
    if let Ok(d) = std::env::var("AIFRED_HOME") {
        return Ok(PathBuf::from(d));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".aifred"))
}

impl<B: Backend> Assistant<B> {
    pub fn new(dir: PathBuf, backend: B, program: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        // Make the path full. The tool servers run in another folder
        // (the Claude work folder), so a relative path would point
        // them to a different database.
        let dir = std::path::absolute(&dir).with_context(|| format!("bad data folder {}", dir.display()))?;
        Ok(Self {
            dir,
            backend,
            program,
            summary: tokio::sync::Mutex::new(()),
            item_summaries: tokio::sync::Mutex::new(Default::default()),
        })
    }

    fn history(&self) -> History {
        History::new(&self.dir)
    }

    /// Runs database and file work on a thread for blocking work. Each
    /// database step can wait up to 5 seconds for a lock (db.rs). On an
    /// async thread, that wait stops the other work of the app, and the
    /// window freezes.
    async fn off_thread<T: Send + 'static>(
        &self,
        f: impl FnOnce(&Path) -> anyhow::Result<T> + Send + 'static,
    ) -> anyhow::Result<T> {
        let dir = self.dir.clone();
        tokio::task::spawn_blocking(move || f(&dir)).await?
    }

    fn session_path(&self) -> PathBuf {
        self.dir.join(format!("session-{}.json", self.backend.name()))
    }

    /// What AIfred knows about the user.
    pub fn memory(&self) -> MemoryStore {
        MemoryStore::new(&self.dir)
    }

    /// Saves a fact from the user. It is a core fact: the user asked for it.
    pub fn remember(&self, fact: &str) -> anyhow::Result<()> {
        self.memory().add(fact, "you", true).map(|_| ())
    }

    /// Makes the missing meaning search vectors (embed.rs). Returns how many.
    /// The first call downloads the model. It can take some seconds.
    pub fn fill_vectors(&self) -> anyhow::Result<usize> {
        crate::embed::fill_missing(&self.dir)
    }

    pub fn turns(&self) -> anyhow::Result<Vec<Turn>> {
        self.history().load()
    }

    /// Starts a new conversation. Returns the id of the conversation that
    /// ended. Give it to summarize_ended().
    pub fn new_conversation(&self) -> anyhow::Result<Option<i64>> {
        let ended = self.history().archive()?;
        let _ = std::fs::remove_file(self.session_path());
        Ok(ended)
    }

    /// Writes the last summary of a conversation that ended. Short
    /// conversations get none. Returns true when it made a summary.
    pub async fn summarize_ended(&self, conversation: i64) -> anyhow::Result<bool> {
        let _one = self.summary.lock().await;
        let u = self.off_thread(move |d| History::new(d).unsummarized(conversation)).await?;
        let chars: usize = u.turns.iter().map(|t| t.text.len()).sum();
        if u.turns.is_empty() || (u.summary.is_none() && chars < SUMMARY_MIN_CHARS) {
            return Ok(false);
        }
        while self.summarize(conversation).await? {}
        Ok(true)
    }

    /// Writes the missing summaries of ended conversations, newest first, at
    /// most OLD_SUMMARIES_PER_RUN. Returns how many it made. It stops at the
    /// first error, so a broken login does not spend more calls.
    pub async fn summarize_old(&self) -> anyhow::Result<usize> {
        let ids = self
            .off_thread(|d| History::new(d).ended_without_full_summary(SUMMARY_MIN_CHARS, OLD_SUMMARIES_PER_RUN))
            .await?;
        let mut made = 0;
        for id in ids {
            if self.summarize_ended(id).await? {
                made += 1;
            }
        }
        Ok(made)
    }

    /// Writes a new summary of the current conversation when more than
    /// SUMMARY_EVERY_TOKENS of its messages are not in the summary yet.
    /// Returns true when it made a summary. It skips the work when another
    /// summary is running.
    pub async fn summarize_if_long(&self) -> anyhow::Result<bool> {
        let Ok(_one) = self.summary.try_lock() else { return Ok(false) };
        let Some(u) = self
            .off_thread(|d| {
                let h = History::new(d);
                h.current()?.map(|id| Ok((id, h.unsummarized(id)?))).transpose()
            })
            .await?
        else {
            return Ok(false);
        };
        let (id, u) = u;
        let chars: usize = u.turns.iter().map(|t| t.text.len()).sum();
        if (chars as u64).div_ceil(4) <= SUMMARY_EVERY_TOKENS {
            return Ok(false);
        }
        while self.summarize(id).await? {}
        Ok(true)
    }

    /// One AI call: the old summary and the oldest new messages, up to
    /// SUMMARY_BATCH_CHARS, become the new summary. Returns true when more
    /// messages are left after this call.
    async fn summarize(&self, conversation: i64) -> anyhow::Result<bool> {
        let u = self.off_thread(move |d| History::new(d).unsummarized(conversation)).await?;
        let mut prompt = String::new();
        if let Some(s) = &u.summary {
            prompt.push_str(&format!("Earlier summary:\n{s}\n\n"));
        }
        prompt.push_str("Messages:\n");
        let mut used = 0;
        let mut taken: usize = 0;
        for t in &u.turns {
            let text: String = t.text.chars().take(SUMMARY_TURN_CHARS).collect();
            // Always take one message, so each call makes progress.
            if taken > 0 && used + text.len() > SUMMARY_BATCH_CHARS {
                break;
            }
            used += text.len();
            taken += 1;
            prompt.push_str(&format!("\n[{}]\n{text}\n", t.role));
        }
        let Some(last) = taken.checked_sub(1).and_then(|i| u.ids.get(i)).copied() else { return Ok(false) };
        let summary = self.backend.complete(SUMMARY_RULES, &prompt, None).await?;
        anyhow::ensure!(!summary.is_empty(), "the summary is empty");
        let more = taken < u.turns.len();
        self.off_thread(move |d| History::new(d).set_summary(conversation, &summary, last)).await?;
        Ok(more)
    }

    /// Writes the missing summaries of long descriptions (summaries.rs), for
    /// the overview tool. Each group of summaries::BATCH items is one AI call
    /// with the fast model. Returns how many it saved. It skips the work when
    /// another run is busy.
    pub async fn summarize_items(&self) -> anyhow::Result<usize> {
        use crate::summaries;
        let Ok(mut skip) = self.item_summaries.try_lock() else { return Ok(0) };
        let mut saved = 0;
        for _ in 0..summaries::MAX_CALLS {
            let skip_now = skip.clone();
            let items = self.off_thread(move |d| summaries::pending(d, summaries::BATCH, &skip_now)).await?;
            if items.is_empty() {
                break;
            }
            // A failed call (no network, not logged in) can work next time:
            // the items are not skipped.
            let answer = self.backend.complete(summaries::RULES, &summaries::prompt(&items), Some(summaries::MODEL)).await?;
            // The AI gave no usable summary: skip these items until the app
            // starts again, so the next runs go on to the other items.
            let parsed = match summaries::parse(&answer, items.len()) {
                Ok(p) => p,
                Err(e) => {
                    skip.extend(items.iter().map(|i| i.key()));
                    return Err(e);
                }
            };
            skip.extend(items.iter().zip(&parsed).filter(|(_, s)| s.is_none()).map(|(i, _)| i.key()));
            saved += self.off_thread(move |d| summaries::save(d, &items, &parsed)).await?;
        }
        Ok(saved)
    }

    /// The facts that are not core and match the message, for a large memory.
    /// Empty for a small memory: then all facts are in the instructions.
    async fn matching_facts(&self, prompt: &str, small: bool) -> anyhow::Result<Vec<String>> {
        if small {
            return Ok(vec![]);
        }
        // The search can load the model, so it runs off the async thread.
        let (memory, prompt) = (self.memory(), prompt.to_string());
        let facts = tokio::task::spawn_blocking(move || memory.search_other(&prompt, Some(ADDED_FACTS))).await??;
        Ok(facts.into_iter().map(|f| f.text).collect())
    }

    /// The message to the backend: the time, the matching facts, and the text
    /// of the user. The facts go here and not into the instructions, so the
    /// instructions stay the same between messages.
    fn message(now: &str, facts: &[String], prompt: &str) -> String {
        let mut m = format!("[Now: {now}]\n");
        if !facts.is_empty() {
            m.push_str("[Facts from memory that can help with this message:\n");
            for f in facts {
                m.push_str(&format!("- {f}\n"));
            }
            m.push_str("]\n");
        }
        m.push_str(prompt);
        m
    }

    fn now() -> String {
        std::process::Command::new("date")
            .arg("+%A %Y-%m-%d %H:%M %Z")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    /// The path of a tool server program, or None if it is not built.
    pub fn tasks(&self) -> TaskStore {
        TaskStore::new(&self.dir)
    }

    /// The tool servers, or a notice when the AIfred program is missing.
    /// A missing program must not stop the whole chat.
    fn tool_servers(&self) -> (Vec<ToolServer>, Vec<String>) {
        if !self.program.is_file() {
            let notice = format!("The AI tools are off: the AIfred program {} is missing.", self.program.display());
            return (vec![], vec![notice]);
        }
        let server = |name: &str, env: Vec<(String, String)>, allowed_tools: &[&str]| ToolServer {
            name: name.into(),
            command: self.program.clone(),
            args: vec![crate::program::TOOL.into(), name.into()],
            env,
            allowed_tools: allowed_tools.iter().map(|t| t.to_string()).collect(),
        };

        // The server must use the same data folder as this app. The GitHub
        // owner filter goes to each server that downloads the GitHub list
        // (the overview tool of the tasks server, and the GitHub server).
        // Without it, a download saves the items of all owners.
        let home = || vec![("AIFRED_HOME".to_string(), self.dir.display().to_string())];
        let home_with_owners = || {
            let mut env = home();
            if let Ok(owners) = std::env::var("AIFRED_GITHUB_OWNERS") {
                env.push(("AIFRED_GITHUB_OWNERS".to_string(), owners));
            }
            env
        };

        let mut servers = vec![
            server(
                "tasks",
                home_with_owners(),
                &["overview", "list_tasks", "search_tasks", "add_task", "update_task", "complete_task"],
            ),
            server("memory", home(), &["search_memory", "add_memory", "forget_memory", "search_history"]),
            // The server searches the saved GitHub list in the data folder.
            server(
                "github",
                home_with_owners(),
                &["my_issues", "review_requests", "my_pull_requests", "search_my_items", "item_details"],
            ),
        ];
        // ClickUp tools only when the user connected ClickUp. The server reads
        // the token from the data folder itself, so the token never goes into
        // the tool settings file.
        if crate::clickup::token(&self.dir).is_some() {
            servers.push(server("clickup", home(), &["my_tasks", "search_my_tasks", "task_details"]));
        }
        (servers, vec![])
    }

    pub async fn ask(&self, prompt: &str, on_event: OnEvent<'_>) -> anyhow::Result<Answer> {
        let session_path = self.session_path();
        let (model, saved, small, summary, unsummarized, system) = self
            .off_thread({
                let session_path = session_path.clone();
                move |dir| {
                    // The model that the user picked last in the chat box.
                    let model = crate::settings::chat_model(dir)?;
                    let saved: SessionState = std::fs::read(&session_path)
                        .ok()
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or_default();
                    let small = memory_is_small(dir)?;
                    // A replay needs the summary and every turn after it.
                    let history = History::new(dir);
                    let (summary, unsummarized) = match history.current()? {
                        Some(id) => {
                            let u = history.unsummarized(id)?;
                            (u.summary, u.turns)
                        }
                        None => (None, vec![]),
                    };
                    Ok((model, saved, small, summary, unsummarized, system_prompt(dir, small)?))
                }
            })
            .await?;

        let (tools, missing) = self.tool_servers();
        for message in missing {
            on_event(Event::Notice { message });
        }
        let facts = match self.matching_facts(prompt, small).await {
            Ok(f) => f,
            Err(e) => {
                // The AI can still search memory itself.
                on_event(Event::Notice { message: format!("Cannot add facts from memory: {e:#}") });
                vec![]
            }
        };
        let req = AskRequest {
            system,
            history: unsummarized,
            summary,
            prompt: Self::message(&Self::now(), &facts, prompt),
            tools,
            session: saved.id.clone(),
            model: model.clone(),
        };
        let res = self.backend.ask(&req, on_event).await?;

        // Claude reports totals for the whole conversation. A new session
        // (different id) starts its totals from zero.
        let same = res.session.is_some() && res.session == saved.id;
        let (base_ms, base_cost) = if same { (saved.model_ms, saved.cost_usd) } else { (0, 0.0) };
        let model_ms = res.stats.model_ms.map(|v| v.saturating_sub(base_ms));
        let cost_usd = res.stats.cost_usd.map(|v| (v - base_cost).max(0.0));
        let state = SessionState {
            id: res.session.clone(),
            model_ms: res.stats.model_ms.unwrap_or(base_ms),
            cost_usd: res.stats.cost_usd.unwrap_or(base_cost),
        };
        let (name, prompt, text) = (self.backend.name(), prompt.to_string(), res.text.clone());
        self.off_thread(move |dir| {
            let history = History::new(dir);
            history.append("user", &prompt, name)?;
            history.append("assistant", &text, name)?;
            std::fs::write(&session_path, serde_json::to_vec(&state)?)?;
            Ok(())
        })
        .await?;

        Ok(Answer { text: res.text, stats: res.stats, model_ms, cost_usd, memory: facts, model })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::claude::ClaudeBackend;

    fn assistant(dir: PathBuf, program: PathBuf) -> Assistant<ClaudeBackend> {
        let backend = ClaudeBackend { work_dir: dir.join("claude"), model: None };
        Assistant::new(dir, backend, program).unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-assistant-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A backend with no AI. It saves what it gets, so the tests can check it.
    #[derive(Default)]
    struct FakeBackend {
        prompts: std::sync::Mutex<Vec<String>>,
        summaries: std::sync::Mutex<Vec<Option<String>>>,
        histories: std::sync::Mutex<Vec<Vec<Turn>>>,
        models: std::sync::Mutex<Vec<Option<String>>>,
        item_calls: std::sync::Mutex<Vec<String>>,
    }

    impl Backend for FakeBackend {
        fn name(&self) -> &'static str {
            "fake"
        }

        async fn ask(&self, req: &AskRequest, _on_event: OnEvent<'_>) -> anyhow::Result<crate::backend::AskResponse> {
            self.summaries.lock().unwrap().push(req.summary.clone());
            self.histories.lock().unwrap().push(req.history.clone());
            self.models.lock().unwrap().push(req.model.clone());
            Ok(crate::backend::AskResponse { text: "ok".into(), session: None, stats: AskStats::default() })
        }

        async fn complete(&self, system: &str, prompt: &str, model: Option<&str>) -> anyhow::Result<String> {
            if system == crate::summaries::RULES {
                assert_eq!(model, Some(crate::summaries::MODEL), "item summaries use the fast model");
                self.item_calls.lock().unwrap().push(prompt.to_string());
                let n = prompt.matches("### Item ").count();
                let all: serde_json::Map<String, serde_json::Value> =
                    (1..=n).map(|i| (i.to_string(), format!("summary {i}").into())).collect();
                return Ok(serde_json::Value::Object(all).to_string());
            }
            assert!(system.contains("summary"));
            let mut p = self.prompts.lock().unwrap();
            p.push(prompt.to_string());
            Ok(format!("- summary {}", p.len()))
        }
    }

    fn fake(name: &str) -> (Assistant<FakeBackend>, PathBuf) {
        let dir = temp(name);
        (Assistant::new(dir.clone(), FakeBackend::default(), dir.join("no-tools")).unwrap(), dir)
    }

    #[tokio::test]
    async fn a_message_uses_the_model_from_the_chat_box() {
        let (a, dir) = fake("model");
        let mut quiet = |_e: Event| {};
        a.ask("hi", &mut quiet).await.unwrap();
        crate::settings::set_chat_model(&dir, Some("haiku")).unwrap();
        let ans = a.ask("hi again", &mut quiet).await.unwrap();
        assert_eq!(*a.backend.models.lock().unwrap(), vec![None, Some("haiku".to_string())]);
        assert_eq!(ans.model.as_deref(), Some("haiku"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn long_task_descriptions_get_summaries_in_groups() {
        let (a, dir) = fake("item-summaries");
        for i in 0..20 {
            a.tasks().add(&format!("Task {i}"), &"Plan the release with the team. ".repeat(20), "you").unwrap();
        }
        a.tasks().add("Short", "Call Sam.", "you").unwrap();
        assert_eq!(a.summarize_items().await.unwrap(), 20);
        assert_eq!(a.backend.item_calls.lock().unwrap().len(), 2, "15 items in one call, then 5");
        assert_eq!(a.summarize_items().await.unwrap(), 0, "nothing left to do");
        assert_eq!(a.backend.item_calls.lock().unwrap().len(), 2, "no AI call when nothing is missing");
        let o = crate::overview::build(&dir, None).unwrap();
        let summaries: Vec<&str> = o["local_tasks"].as_array().unwrap().iter().map(|t| t["summary"].as_str().unwrap()).collect();
        assert!(summaries.contains(&"Call Sam.") && summaries.iter().filter(|s| s.starts_with("summary ")).count() == 20);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn old_conversations_with_no_summary_get_one_later() {
        let (a, dir) = fake("summary-old");
        a.history().append("user", "Hi", "fake").unwrap();
        a.new_conversation().unwrap();
        a.history().append("user", &"Plan the Bali trip. ".repeat(150), "fake").unwrap();
        let long = a.new_conversation().unwrap().unwrap();
        // The current conversation never gets a summary here.
        a.history().append("user", &"Book the flights. ".repeat(150), "fake").unwrap();
        assert_eq!(a.summarize_old().await.unwrap(), 1, "only the long ended conversation");
        assert!(a.history().unsummarized(long).unwrap().turns.is_empty());
        assert_eq!(a.summarize_old().await.unwrap(), 0, "nothing left to do");
        assert_eq!(a.backend.prompts.lock().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn an_ended_conversation_gets_a_summary_only_when_it_is_long_enough() {
        let (a, dir) = fake("summary-ended");
        a.history().append("user", "Hi", "fake").unwrap();
        let short = a.new_conversation().unwrap().unwrap();
        assert!(!a.summarize_ended(short).await.unwrap(), "a short chat costs no AI call");
        a.history().append("user", &"Plan the Bali trip. ".repeat(150), "fake").unwrap();
        let long = a.new_conversation().unwrap().unwrap();
        assert!(a.summarize_ended(long).await.unwrap());
        assert!(!a.summarize_ended(long).await.unwrap(), "no new messages, no new summary");
        assert_eq!(a.backend.prompts.lock().unwrap().len(), 1);
        assert_eq!(a.history().search_summaries("summary", None).unwrap()[0].conversation, long);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_long_conversation_gets_a_summary_for_each_20000_tokens() {
        let (a, dir) = fake("summary-long");
        let msg = "x".repeat(10_000); // 2,500 tokens
        for _ in 0..8 {
            a.history().append("user", &msg, "fake").unwrap();
        }
        assert!(!a.summarize_if_long().await.unwrap(), "20,000 tokens is not more than the limit");
        a.history().append("assistant", "One more answer.", "fake").unwrap();
        assert!(a.summarize_if_long().await.unwrap());
        assert_eq!(a.history().current_summary().unwrap().as_deref(), Some("- summary 1"));
        a.history().append("user", "Short", "fake").unwrap();
        assert!(!a.summarize_if_long().await.unwrap(), "only a few new tokens");
        for _ in 0..9 {
            a.history().append("user", &msg, "fake").unwrap();
        }
        assert!(a.summarize_if_long().await.unwrap());
        let prompts = a.backend.prompts.lock().unwrap().clone();
        assert!(prompts[1].starts_with("Earlier summary:\n- summary 1"), "the new summary builds on the old one");
        assert!(!prompts[1].contains("One more answer."), "old messages are not sent again");
        assert!(prompts[1].contains("[user]\nShort"));

        // The next question carries the summary, for a lost session.
        a.ask("Next", &mut |_| {}).await.unwrap();
        assert_eq!(a.backend.summaries.lock().unwrap().last().unwrap().as_deref(), Some("- summary 2"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_large_backlog_gets_its_summary_in_parts() {
        // For example an imported history with no summary: 300 messages of
        // 1,000 characters. One call would be about 300 KB.
        let (a, dir) = fake("summary-parts");
        let msg = "y".repeat(1000);
        for _ in 0..300 {
            a.history().append("user", &msg, "fake").unwrap();
        }
        assert!(a.summarize_if_long().await.unwrap());
        let prompts = a.backend.prompts.lock().unwrap().clone();
        assert_eq!(prompts.len(), 3, "100,000 characters for each call");
        assert!(prompts.iter().all(|p| p.len() < SUMMARY_BATCH_CHARS + 20_000));
        assert!(prompts[1].starts_with("Earlier summary:\n- summary 1"));
        let id = a.history().current().unwrap().unwrap();
        assert!(a.history().unsummarized(id).unwrap().turns.is_empty(), "all messages are in the summary");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_replay_gets_the_turns_after_the_summary() {
        let (a, dir) = fake("replay-after-summary");
        let msg = "z".repeat(10_000);
        for _ in 0..9 {
            a.history().append("user", &msg, "fake").unwrap();
        }
        assert!(a.summarize_if_long().await.unwrap());
        for i in 0..15 {
            a.history().append("user", &format!("after {i}"), "fake").unwrap();
        }
        a.ask("Next", &mut |_| {}).await.unwrap();
        let sent = a.backend.histories.lock().unwrap().last().unwrap().clone();
        assert_eq!(sent.len(), 15, "every turn after the summary, not only the last 10");
        assert_eq!(sent[0].text, "after 0");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_small_memory_goes_into_the_instructions_in_full() {
        let dir = temp("small-memory");
        let a = assistant(dir.clone(), dir.clone());
        a.memory().add("I like tea", "you", true).unwrap();
        a.memory().add("The office wifi is slow on Mondays", "aifred", false).unwrap();
        assert!(memory_is_small(&a.dir).unwrap());
        let p = system_prompt(&a.dir, true).unwrap();
        assert!(p.contains("- I like tea") && p.contains("- The office wifi"), "{p}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[tokio::test]
    async fn a_large_memory_adds_only_the_matching_facts_to_the_message() {
        let dir = temp("large-memory");
        let a = assistant(dir.clone(), dir.clone());
        a.memory().add("I like tea", "you", true).unwrap();
        a.memory().add("Sam sends the hosting invoice each month", "aifred", false).unwrap();
        // More than 2,000 tokens of other facts.
        for i in 0..150 {
            a.memory().add(&format!("Fact number {i} about the garden and the old shed in the yard behind the house"), "aifred", false).unwrap();
        }
        assert!(!memory_is_small(&a.dir).unwrap());
        let before = system_prompt(&a.dir, false).unwrap();
        assert!(before.contains("- I like tea"), "core facts stay");
        assert!(!before.contains("invoice"), "other facts are not in the instructions");

        let facts = a.matching_facts("When does the hosting invoice come?", false).await.unwrap();
        assert_eq!(facts[0], "Sam sends the hosting invoice each month");
        assert!(facts.len() <= ADDED_FACTS as usize);
        assert!(!facts.contains(&"I like tea".to_string()), "core facts are already in the instructions");

        a.memory().add("A new fact that is not core", "aifred", false).unwrap();
        assert_eq!(system_prompt(&a.dir, false).unwrap(), before, "the instructions stay the same");

        let m = Assistant::<ClaudeBackend>::message("Monday", &facts[..1], "When?");
        assert_eq!(
            m,
            "[Now: Monday]\n[Facts from memory that can help with this message:\n- Sam sends the hosting invoice each month\n]\nWhen?"
        );
        assert_eq!(Assistant::<ClaudeBackend>::message("Monday", &[], "When?"), "[Now: Monday]\nWhen?");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn relative_data_folder_becomes_a_full_path() {
        let base = temp("relative");
        // A relative path, as a user can type it in AIFRED_HOME.
        let rel = PathBuf::from(format!("aifred-relative-test-{}", std::process::id()));
        let a = assistant(rel.clone(), base.clone());
        assert!(a.dir.is_absolute(), "got {}", a.dir.display());
        assert_eq!(a.dir, std::env::current_dir().unwrap().join(&rel));
        let _ = std::fs::remove_dir_all(&a.dir);
        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn missing_program_gives_a_notice_not_an_error() {
        let dir = temp("no-tools");
        let (servers, missing) = assistant(dir.clone(), dir.join("aifred")).tool_servers();
        assert!(servers.is_empty());
        assert_eq!(missing.len(), 1);
        assert!(missing[0].contains("AI tools are off"), "got {missing:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn tool_servers_run_the_program_and_get_the_data_folder() {
        let dir = temp("tools");
        std::fs::write(dir.join("aifred"), "").unwrap();
        let a = assistant(dir.clone(), dir.join("aifred"));
        let (servers, missing) = a.tool_servers();
        assert!(missing.is_empty(), "got {missing:?}");
        let names: Vec<_> = servers.iter().map(|s| s.name.as_str()).filter(|n| *n != "clickup").collect();
        assert_eq!(names, ["tasks", "memory", "github"]);
        let tasks = &servers[0];
        assert_eq!(tasks.command, dir.join("aifred"));
        assert_eq!(tasks.args, ["tool", "tasks"]);
        assert_eq!(tasks.env[0], ("AIFRED_HOME".to_string(), a.dir.display().to_string()));
        assert_eq!(servers[1].env, vec![("AIFRED_HOME".to_string(), a.dir.display().to_string())]);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clickup_tools_start_only_with_a_token_and_never_get_it() {
        let dir = temp("clickup-tool");
        std::fs::write(dir.join("aifred"), "").unwrap();
        let a = assistant(dir.clone(), dir.join("aifred"));
        let names = |a: &Assistant<ClaudeBackend>| a.tool_servers().0.into_iter().map(|s| s.name).collect::<Vec<_>>();
        if std::env::var_os("AIFRED_CLICKUP_TOKEN").is_none() {
            assert!(!names(&a).contains(&"clickup".to_string()), "no token, no ClickUp tools");
        }
        crate::clickup::save_token(&a.dir, "pk_1_SECRET").unwrap();
        let (servers, _) = a.tool_servers();
        let s = servers.iter().find(|s| s.name == "clickup").expect("ClickUp tools start");
        assert_eq!(s.allowed_tools, vec!["my_tasks", "search_my_tasks", "task_details"], "read-only tools only");
        assert_eq!(s.args, ["tool", "clickup"]);
        assert!(!format!("{:?}", s.env).contains("SECRET"), "the token stays out of the tool settings");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
