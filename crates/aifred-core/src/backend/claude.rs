//! Backend that runs the official `claude` program in headless mode.
//!
//! It uses the user's own Claude login. aifred never reads or passes on the
//! login token: it only starts the unmodified `claude` binary.

use std::process::Stdio;
use std::time::Instant;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;

use super::{AskRequest, AskResponse, AskStats, Backend, Event, OnEvent};
use crate::history::Turn;

/// At most this many characters of stored turns go into a replay when there
/// is no session to resume (about 25,000 tokens). The newest turns go first.
const REPLAY_MAX_CHARS: usize = 100_000;

pub struct ClaudeBackend {
    /// Folder the `claude` process runs in. It holds no project files, so no
    /// project CLAUDE.md or project settings load.
    pub work_dir: std::path::PathBuf,
    pub model: Option<String>,
}

impl Backend for ClaudeBackend {
    fn name(&self) -> &'static str {
        "claude"
    }

    async fn ask(&self, req: &AskRequest, on_event: OnEvent<'_>) -> anyhow::Result<AskResponse> {
        match self.run(req, req.session.as_deref(), on_event).await {
            Ok(r) => Ok(r),
            // The saved session can be gone (cleaned up, other machine).
            // aifred owns the history, so start again and replay it.
            Err(e) if req.session.is_some() => {
                on_event(Event::Notice {
                    message: format!(
                        "Cannot resume the Claude session ({e:#}). Starting a new one."
                    ),
                });
                self.run(req, None, on_event).await
            }
            Err(e) => Err(e),
        }
    }

    async fn complete(
        &self,
        system: &str,
        prompt: &str,
        model: Option<&str>,
    ) -> anyhow::Result<String> {
        // Its own folder: an `ask` at the same time writes files into work_dir.
        let dir = self.work_dir.join("once");
        std::fs::create_dir_all(&dir)?;
        let mut cmd = Command::new("claude");
        // The prompt goes in through stdin. One command-line argument can
        // hold at most 128 KiB on Linux, and a summary prompt can be larger.
        cmd.current_dir(&dir)
            .arg("-p")
            .args(["--output-format", "json"])
            // No tools at all: no built-in tools and no MCP servers.
            .args(["--tools", ""])
            .arg("--strict-mcp-config")
            .args(["--permission-mode", "dontAsk"])
            .args(["--setting-sources", "local"])
            .arg("--system-prompt")
            .arg(system)
            // Do not save a session: this call is not a chat.
            .arg("--no-session-persistence")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(m) = model.or(self.model.as_deref()) {
            cmd.args(["--model", m]);
        }
        let mut child = cmd
            .spawn()
            .context("cannot start `claude`. Is Claude Code installed?")?;
        let writer = send_prompt(&mut child, prompt)?;
        let out = child.wait_with_output().await?;
        // Check the write last: when claude stops early, the write fails with
        // "Broken pipe", and the real reason is in claude's own output.
        let written = writer.await?;
        let res: Value = serde_json::from_slice(&out.stdout).with_context(|| {
            format!(
                "claude ended without an answer (exit {}): {}",
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            )
        })?;
        if res["is_error"].as_bool().unwrap_or(false) {
            bail!(
                "claude reported an error: {}",
                res["result"].as_str().unwrap_or(&res.to_string())
            );
        }
        written.context("cannot send the prompt to claude")?;
        Ok(res["result"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_string())
    }
}

/// A new folder for one side chat question, removed when it is dropped.
struct SideDir(std::path::PathBuf);

impl SideDir {
    fn new(work_dir: &std::path::Path) -> anyhow::Result<Self> {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = work_dir
            .join("side")
            .join(format!("{}-{n}", std::process::id()));
        std::fs::create_dir_all(&dir)?;
        Ok(Self(dir))
    }
}

impl Drop for SideDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl ClaudeBackend {
    async fn run(
        &self,
        req: &AskRequest,
        resume: Option<&str>,
        on_event: OnEvent<'_>,
    ) -> anyhow::Result<AskResponse> {
        // Each side chat question has its own new folder: a main question or
        // another side chat question at the same time writes mcp.json and
        // system.md into its own folder. The guard removes the folder after.
        let side_dir = if req.side {
            Some(SideDir::new(&self.work_dir)?)
        } else {
            None
        };
        let work_dir = match &side_dir {
            Some(d) => d.0.clone(),
            None => self.work_dir.clone(),
        };
        std::fs::create_dir_all(&work_dir)?;

        let mcp_path = work_dir.join("mcp.json");
        std::fs::write(&mcp_path, serde_json::to_vec_pretty(&mcp_config(req))?)?;
        let system_path = work_dir.join("system.md");
        std::fs::write(&system_path, &req.system)?;

        let mut allowed: Vec<String> = req
            .tools
            .iter()
            .flat_map(|s| {
                s.allowed_tools
                    .iter()
                    .map(move |t| format!("mcp__{}__{}", s.name, t))
            })
            .collect();
        // Read only in the folders of the files of this message. A path rule
        // with "//" at the start is a full path. In dontAsk mode Claude
        // refuses every other read outside its work folder.
        allowed.extend(
            req.files
                .iter()
                .map(|f| format!("Read(/{}/**)", f.display())),
        );

        let prompt = match resume {
            Some(_) => req.prompt.clone(),
            None => with_replayed_history(&req.history, req.summary.as_deref(), &req.prompt),
        };

        let mut cmd = Command::new("claude");
        // The prompt goes in through stdin: a replay can be larger than the
        // 128 KiB that one command-line argument can hold on Linux.
        cmd.current_dir(&work_dir)
            .arg("-p")
            .args(["--output-format", "stream-json", "--verbose"])
            // No built-in tools: no shell, no file access, no web. Only Read
            // when the user sent files, for the folders in `allowed`.
            .args(["--tools", if req.files.is_empty() { "" } else { "Read" }])
            // Only the MCP servers aifred gives, and only the named tools.
            .arg("--mcp-config")
            .arg(&mcp_path)
            .arg("--strict-mcp-config")
            .args(["--allowedTools", &allowed.join(",")])
            .args(["--permission-mode", "dontAsk"])
            // Skip user settings (hooks, plugins). Auth is not a setting.
            .args(["--setting-sources", "local"])
            .arg("--append-system-prompt-file")
            .arg(&system_path)
            // Memory can change between turns. Do not freeze the prompt.
            .args(["--system-prompt-snapshot", "off"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        for f in &req.files {
            cmd.arg("--add-dir").arg(f);
        }
        if let Some(id) = resume {
            cmd.args(["--resume", id]);
        }
        if req.side {
            // A side chat sends its messages again each time (side.rs).
            cmd.arg("--no-session-persistence");
        }
        // The model that the user picked in the chat box, or AIFRED_MODEL.
        if let Some(m) = req.model.as_deref().or(self.model.as_deref()) {
            cmd.args(["--model", m]);
        }

        let start = Instant::now();
        let mut child = cmd
            .spawn()
            .context("cannot start `claude`. Is Claude Code installed?")?;
        let writer = send_prompt(&mut child, &prompt)?;
        let stdout = child.stdout.take().context("no stdout")?;
        let mut lines = BufReader::new(stdout).lines();

        let mut stats = AskStats::default();
        let mut result: Option<Value> = None;
        while let Some(line) = lines.next_line().await? {
            if stats.startup_ms == 0 {
                stats.startup_ms = start.elapsed().as_millis();
            }
            let Ok(ev) = serde_json::from_str::<Value>(&line) else {
                continue;
            };
            match ev["type"].as_str() {
                Some("assistant") => {
                    for block in ev["message"]["content"].as_array().into_iter().flatten() {
                        if block["type"] == "tool_use" {
                            let name = block["name"].as_str().unwrap_or("?").to_string();
                            // Claude names MCP tools `mcp__<server>__<tool>`.
                            let mut parts = name.splitn(3, "__").skip(1);
                            let (server, tool) = match (parts.next(), parts.next()) {
                                (Some(s), Some(t)) => (s.to_string(), t.to_string()),
                                _ => (String::new(), name.clone()),
                            };
                            on_event(Event::ToolCall {
                                server,
                                tool,
                                input: block["input"].clone(),
                            });
                            stats.tool_calls.push(name);
                        }
                    }
                }
                Some("result") => result = Some(ev),
                _ => {}
            }
        }

        let output = child.wait_with_output().await?;
        // Check the write last, for the same reason as in `complete`.
        let written = writer.await?;
        stats.total_ms = start.elapsed().as_millis();
        let Some(res) = result else {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!(
                "claude ended without an answer (exit {}): {}",
                output.status,
                err.trim()
            );
        };
        if res["is_error"].as_bool().unwrap_or(false) {
            bail!(
                "claude reported an error: {}",
                res["result"].as_str().unwrap_or(&res.to_string())
            );
        }
        written.context("cannot send the prompt to claude")?;

        stats.model_ms = res["duration_api_ms"].as_u64();
        stats.cost_usd = res["total_cost_usd"].as_f64();
        Ok(AskResponse {
            text: res["result"].as_str().unwrap_or_default().to_string(),
            session: res["session_id"].as_str().map(String::from),
            stats,
        })
    }
}

/// Writes the prompt to the child's stdin and then closes it, so `claude`
/// knows that the prompt is complete. It runs in its own task: the child can
/// write output before it reads all of stdin.
fn send_prompt(
    child: &mut tokio::process::Child,
    prompt: &str,
) -> anyhow::Result<tokio::task::JoinHandle<std::io::Result<()>>> {
    let mut stdin = child.stdin.take().context("no stdin")?;
    let bytes = prompt.as_bytes().to_vec();
    Ok(tokio::spawn(async move {
        stdin.write_all(&bytes).await?;
        stdin.shutdown().await
    }))
}

fn mcp_config(req: &AskRequest) -> Value {
    let servers: serde_json::Map<String, Value> = req
        .tools
        .iter()
        .map(|s| {
            let env: serde_json::Map<String, Value> = s
                .env
                .iter()
                .map(|(k, v)| (k.clone(), Value::String(v.clone())))
                .collect();
            (
                s.name.clone(),
                json!({ "command": s.command, "args": s.args, "env": env }),
            )
        })
        .collect();
    json!({ "mcpServers": servers })
}

/// `history` holds the turns that the summary does not cover, oldest first.
/// The newest of them go in, up to REPLAY_MAX_CHARS.
fn with_replayed_history(history: &[Turn], summary: Option<&str>, prompt: &str) -> String {
    let mut used = 0;
    let mut from = history.len();
    while from > 0 {
        let len = history[from - 1].text.len();
        if used + len > REPLAY_MAX_CHARS {
            break;
        }
        used += len;
        from -= 1;
    }
    let recent = &history[from..];
    if recent.is_empty() && summary.is_none() {
        return prompt.to_string();
    }
    let mut s = String::new();
    // The summary keeps the main points of the turns before `history`.
    if let Some(sum) = summary {
        s.push_str(&format!("Summary of this conversation so far:\n{sum}\n\n"));
    }
    if from > 0 {
        s.push_str(&format!(
            "({from} older messages are not shown. Use search_history to find them.)\n\n"
        ));
    }
    s.push_str("Earlier conversation (for context):\n");
    for t in recent {
        s.push_str(&format!("\n[{}]\n{}\n", t.role, t.text));
    }
    s.push_str("\n---\nNew message:\n");
    s.push_str(prompt);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn turn(role: &str, text: &str) -> Turn {
        Turn {
            ts: 1,
            role: role.into(),
            text: text.into(),
            backend: "claude".into(),
            files: vec![],
        }
    }

    #[test]
    fn a_lost_session_gets_the_summary_and_all_turns_after_it() {
        let history: Vec<Turn> = (0..60)
            .map(|i| turn("user", &format!("message {i}")))
            .collect();
        let p = with_replayed_history(&history, Some("- The user plans a trip."), "Next?");
        assert!(
            p.starts_with("Summary of this conversation so far:\n- The user plans a trip."),
            "{p}"
        );
        assert!(
            p.contains("message 0\n") && p.contains("message 59\n"),
            "no turn after the summary is lost"
        );
        assert!(!p.contains("older messages"));
        assert!(p.ends_with("New message:\nNext?"));
        assert_eq!(with_replayed_history(&[], None, "Hi"), "Hi");
        assert!(with_replayed_history(&[], Some("- s"), "Hi").contains("- s"));
    }

    #[test]
    fn a_replay_keeps_the_newest_turns_that_fit() {
        let big = "x".repeat(REPLAY_MAX_CHARS / 2 - 10);
        let mut history: Vec<Turn> = (0..3).map(|_| turn("user", &big)).collect();
        history.push(turn("assistant", "the newest answer"));
        let p = with_replayed_history(&history, None, "Next?");
        assert!(
            p.contains("(1 older messages are not shown."),
            "{}",
            &p[..200]
        );
        assert!(p.contains("the newest answer"));
        assert!(p.len() < REPLAY_MAX_CHARS + 1000);
    }
}
