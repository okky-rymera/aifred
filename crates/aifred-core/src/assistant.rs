//! The assistant: memory, history, tools, and one backend.
//! The terminal commands and the desktop app both use this.

use std::path::PathBuf;

use anyhow::Context;
use serde::{Deserialize, Serialize};

use crate::backend::{AskRequest, AskStats, Backend, Event, OnEvent, ToolServer};
use crate::history::{History, Turn};
use crate::tasks::TaskStore;

const BASE_RULES: &str = "\
You are aifred, the user's personal assistant.
- Use the tools to look up facts. Do not guess about the user's work items.
- Keep answers short. Lead with what needs action, most urgent first.
- When you list items, give each one a markdown link and a one-line summary.
- You can add tasks to the user's local to-do list and mark them done.
- Add a task only when the user asks you to track, note, or remember to do something.
- GitHub is read-only. If the user asks for a change there, say what you would change.
- ClickUp is read-only for you. The user marks ClickUp tasks done in the AIfred to-do list.";

/// What the backend reported for one message.
#[derive(Debug, Clone, Serialize)]
pub struct Answer {
    pub text: String,
    pub stats: AskStats,
    /// Model time and cost for this message only (not the whole conversation).
    pub model_ms: Option<u64>,
    pub cost_usd: Option<f64>,
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
    /// Folder that holds the tool server programs.
    pub tools_dir: PathBuf,
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
    pub fn new(dir: PathBuf, backend: B, tools_dir: PathBuf) -> anyhow::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        // Make the path full. The tool servers run in another folder
        // (the Claude work folder), so a relative path would point
        // them to a different tasks.json.
        let dir = std::path::absolute(&dir).with_context(|| format!("bad data folder {}", dir.display()))?;
        Ok(Self { dir, backend, tools_dir })
    }

    fn history(&self) -> History {
        History::new(&self.dir)
    }

    fn memory_path(&self) -> PathBuf {
        self.dir.join("memory.md")
    }

    fn session_path(&self) -> PathBuf {
        self.dir.join(format!("session-{}.json", self.backend.name()))
    }

    pub fn memory(&self) -> String {
        std::fs::read_to_string(self.memory_path()).unwrap_or_default()
    }

    pub fn set_memory(&self, text: &str) -> anyhow::Result<()> {
        Ok(std::fs::write(self.memory_path(), text)?)
    }

    pub fn remember(&self, fact: &str) -> anyhow::Result<()> {
        let mut m = self.memory();
        if !m.is_empty() && !m.ends_with('\n') {
            m.push('\n');
        }
        m.push_str(&format!("- {fact}\n"));
        self.set_memory(&m)
    }

    pub fn turns(&self) -> anyhow::Result<Vec<Turn>> {
        self.history().load()
    }

    pub fn new_conversation(&self) -> anyhow::Result<()> {
        self.history().archive()?;
        let _ = std::fs::remove_file(self.session_path());
        Ok(())
    }

    /// Rules and memory only. It must stay the same between messages:
    /// any change here makes the backend re-read the whole conversation
    /// (no prompt cache hit). The current time goes into each message.
    fn system_prompt(&self) -> String {
        format!("{BASE_RULES}\n\n# What you know about the user\n{}", self.memory())
    }

    fn now() -> String {
        std::process::Command::new("date")
            .arg("+%A %Y-%m-%d %H:%M %Z")
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default()
    }

    /// The path of a tool server program, or None if it is not built.
    fn tool_path(&self, name: &str) -> Option<PathBuf> {
        let p = self.tools_dir.join(name);
        p.exists().then_some(p)
    }

    pub fn tasks(&self) -> TaskStore {
        TaskStore::new(&self.dir)
    }

    /// The tool servers that are built, and a notice for each one that is
    /// missing. A missing server must not stop the whole chat.
    fn tool_servers(&self) -> (Vec<ToolServer>, Vec<String>) {
        let mut servers = vec![];
        let mut missing = vec![];
        let mut notice = |name: &str, what: &str| {
            missing.push(format!(
                "{what} are off: {} is missing. Run `cargo build` in the workspace.",
                self.tools_dir.join(name).display()
            ))
        };

        match self.tool_path("aifred-tasks-mcp") {
            Some(command) => servers.push(ToolServer {
                name: "tasks".into(),
                command,
                args: vec![],
                // The server must use the same data folder as this app.
                env: vec![("AIFRED_HOME".into(), self.dir.display().to_string())],
                allowed_tools: ["list_tasks", "add_task", "complete_task"].map(String::from).to_vec(),
            }),
            None => notice("aifred-tasks-mcp", "To-do list tools"),
        }

        match self.tool_path("aifred-github-mcp") {
            Some(command) => {
                let mut env = vec![];
                if let Ok(owners) = std::env::var("AIFRED_GITHUB_OWNERS") {
                    env.push(("AIFRED_GITHUB_OWNERS".to_string(), owners));
                }
                servers.push(ToolServer {
                    name: "github".into(),
                    command,
                    args: vec![],
                    env,
                    allowed_tools: ["my_issues", "review_requests", "my_pull_requests", "item_details"]
                        .map(String::from)
                        .to_vec(),
                });
            }
            None => notice("aifred-github-mcp", "GitHub tools"),
        }

        // ClickUp tools only when the user connected ClickUp. The server reads
        // the token from the data folder itself, so the token never goes into
        // the tool settings file.
        if crate::clickup::token(&self.dir).is_some() {
            match self.tool_path("aifred-clickup-mcp") {
                Some(command) => servers.push(ToolServer {
                    name: "clickup".into(),
                    command,
                    args: vec![],
                    env: vec![("AIFRED_HOME".into(), self.dir.display().to_string())],
                    allowed_tools: ["my_tasks", "task_details"].map(String::from).to_vec(),
                }),
                None => notice("aifred-clickup-mcp", "ClickUp tools"),
            }
        }
        (servers, missing)
    }

    pub async fn ask(&self, prompt: &str, on_event: OnEvent<'_>) -> anyhow::Result<Answer> {
        let history = self.history();
        let saved: SessionState = std::fs::read(self.session_path())
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();

        let (tools, missing) = self.tool_servers();
        for message in missing {
            on_event(Event::Notice { message });
        }
        let req = AskRequest {
            system: self.system_prompt(),
            history: history.load()?,
            prompt: format!("[Now: {}]\n{prompt}", Self::now()),
            tools,
            session: saved.id.clone(),
        };
        let res = self.backend.ask(&req, on_event).await?;

        history.append("user", prompt, self.backend.name())?;
        history.append("assistant", &res.text, self.backend.name())?;

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
        std::fs::write(self.session_path(), serde_json::to_vec(&state)?)?;

        Ok(Answer { text: res.text, stats: res.stats, model_ms, cost_usd })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::claude::ClaudeBackend;

    fn assistant(dir: PathBuf, tools_dir: PathBuf) -> Assistant<ClaudeBackend> {
        let backend = ClaudeBackend { work_dir: dir.join("claude"), model: None };
        Assistant::new(dir, backend, tools_dir).unwrap()
    }

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-assistant-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
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
    fn missing_tool_servers_give_notices_not_an_error() {
        let dir = temp("no-tools");
        let (servers, missing) = assistant(dir.clone(), dir.join("empty")).tool_servers();
        assert!(servers.is_empty());
        // AIFRED_CLICKUP_TOKEN in the developer's shell also adds a ClickUp notice.
        let missing = without_clickup(missing);
        assert_eq!(missing.len(), 2);
        assert!(missing[0].contains("aifred-tasks-mcp"), "got {missing:?}");
        assert!(missing[1].contains("aifred-github-mcp"), "got {missing:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn built_tool_server_is_used_and_gets_the_data_folder() {
        let dir = temp("one-tool");
        std::fs::write(dir.join("aifred-tasks-mcp"), "").unwrap();
        let a = assistant(dir.clone(), dir.clone());
        let (servers, missing) = a.tool_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].name, "tasks");
        assert_eq!(servers[0].env, vec![("AIFRED_HOME".to_string(), a.dir.display().to_string())]);
        assert_eq!(without_clickup(missing).len(), 1, "only GitHub is missing");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The notices without the ClickUp one. It shows only when a token is set,
    /// and AIFRED_CLICKUP_TOKEN can set one outside the test.
    fn without_clickup(missing: Vec<String>) -> Vec<String> {
        missing.into_iter().filter(|m| !m.contains("aifred-clickup-mcp")).collect()
    }

    #[test]
    fn clickup_tools_start_only_with_a_token_and_never_get_it() {
        let dir = temp("clickup-tool");
        std::fs::write(dir.join("aifred-clickup-mcp"), "").unwrap();
        let a = assistant(dir.clone(), dir.clone());
        let names = |a: &Assistant<ClaudeBackend>| a.tool_servers().0.into_iter().map(|s| s.name).collect::<Vec<_>>();
        if std::env::var_os("AIFRED_CLICKUP_TOKEN").is_none() {
            assert!(!names(&a).contains(&"clickup".to_string()), "no token, no ClickUp tools");
        }
        crate::clickup::save_token(&a.dir, "pk_1_SECRET").unwrap();
        let (servers, _) = a.tool_servers();
        let s = servers.iter().find(|s| s.name == "clickup").expect("ClickUp tools start");
        assert_eq!(s.allowed_tools, vec!["my_tasks", "task_details"], "read-only tools only");
        assert!(!format!("{:?}", s.env).contains("SECRET"), "the token stays out of the tool settings");
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
