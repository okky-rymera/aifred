//! Backend that runs the official `claude` program in headless mode.
//!
//! It uses the user's own Claude login. aifred never reads or passes on the
//! login token: it only starts the unmodified `claude` binary.

use std::process::Stdio;
use std::time::Instant;

use anyhow::{Context, bail};
use serde_json::{Value, json};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

use super::{AskRequest, AskResponse, AskStats, Backend, Event, OnEvent};
use crate::history::Turn;

/// How many stored turns to replay when there is no session to resume.
const REPLAY_TURNS: usize = 10;

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
                    message: format!("Cannot resume the Claude session ({e:#}). Starting a new one."),
                });
                self.run(req, None, on_event).await
            }
            Err(e) => Err(e),
        }
    }
}

impl ClaudeBackend {
    async fn run(
        &self,
        req: &AskRequest,
        resume: Option<&str>,
        on_event: OnEvent<'_>,
    ) -> anyhow::Result<AskResponse> {
        std::fs::create_dir_all(&self.work_dir)?;

        let mcp_path = self.work_dir.join("mcp.json");
        std::fs::write(&mcp_path, serde_json::to_vec_pretty(&mcp_config(req))?)?;
        let system_path = self.work_dir.join("system.md");
        std::fs::write(&system_path, &req.system)?;

        let allowed: Vec<String> = req
            .tools
            .iter()
            .flat_map(|s| s.allowed_tools.iter().map(move |t| format!("mcp__{}__{}", s.name, t)))
            .collect();

        let prompt = match resume {
            Some(_) => req.prompt.clone(),
            None => with_replayed_history(&req.history, &req.prompt),
        };

        let mut cmd = Command::new("claude");
        cmd.current_dir(&self.work_dir)
            .arg("-p")
            .arg(&prompt)
            .args(["--output-format", "stream-json", "--verbose"])
            // No built-in tools: no shell, no file access, no web.
            .args(["--tools", ""])
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
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        if let Some(id) = resume {
            cmd.args(["--resume", id]);
        }
        if let Some(m) = &self.model {
            cmd.args(["--model", m]);
        }

        let start = Instant::now();
        let mut child = cmd.spawn().context("cannot start `claude`. Is Claude Code installed?")?;
        let stdout = child.stdout.take().context("no stdout")?;
        let mut lines = BufReader::new(stdout).lines();

        let mut stats = AskStats::default();
        let mut result: Option<Value> = None;
        while let Some(line) = lines.next_line().await? {
            if stats.startup_ms == 0 {
                stats.startup_ms = start.elapsed().as_millis();
            }
            let Ok(ev) = serde_json::from_str::<Value>(&line) else { continue };
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
                            on_event(Event::ToolCall { server, tool, input: block["input"].clone() });
                            stats.tool_calls.push(name);
                        }
                    }
                }
                Some("result") => result = Some(ev),
                _ => {}
            }
        }

        let output = child.wait_with_output().await?;
        stats.total_ms = start.elapsed().as_millis();
        let Some(res) = result else {
            let err = String::from_utf8_lossy(&output.stderr);
            bail!("claude ended without an answer (exit {}): {}", output.status, err.trim());
        };
        if res["is_error"].as_bool().unwrap_or(false) {
            bail!("claude reported an error: {}", res["result"].as_str().unwrap_or(&res.to_string()));
        }

        stats.model_ms = res["duration_api_ms"].as_u64();
        stats.cost_usd = res["total_cost_usd"].as_f64();
        Ok(AskResponse {
            text: res["result"].as_str().unwrap_or_default().to_string(),
            session: res["session_id"].as_str().map(String::from),
            stats,
        })
    }
}

fn mcp_config(req: &AskRequest) -> Value {
    let servers: serde_json::Map<String, Value> = req
        .tools
        .iter()
        .map(|s| {
            let env: serde_json::Map<String, Value> =
                s.env.iter().map(|(k, v)| (k.clone(), Value::String(v.clone()))).collect();
            (s.name.clone(), json!({ "command": s.command, "args": s.args, "env": env }))
        })
        .collect();
    json!({ "mcpServers": servers })
}

fn with_replayed_history(history: &[Turn], prompt: &str) -> String {
    let recent = &history[history.len().saturating_sub(REPLAY_TURNS)..];
    if recent.is_empty() {
        return prompt.to_string();
    }
    let mut s = String::from("Earlier conversation (for context):\n");
    for t in recent {
        s.push_str(&format!("\n[{}]\n{}\n", t.role, t.text));
    }
    s.push_str("\n---\nNew message:\n");
    s.push_str(prompt);
    s
}
