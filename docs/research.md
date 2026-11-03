# aifred research notes

Date: 2026-11-03. Sources were checked on this date. Items marked "unverified" need a check before you depend on them.

## Goal

aifred is a personal assistant. It is not a workflow orchestrator (that is Troupe).

- It syncs with ClickUp, GitHub, and other platforms in two directions.
- It keeps memory about the user and their work.
- It knows where to look. It has limited access to other places.
- It gives a "what do I need to do" list and short summaries (TL;DR).
- It must be fast and light.
- The AI backend must be swappable. Claude first. Grok, DeepSeek, and others later.

## Decision

Build from scratch in Rust. Borrow ideas from existing projects. Do not fork.

Design rule: aifred owns the memory, the history, the tools, and the access rules. The AI backend is a part that you can replace.

```
           aifred (memory, history, task list, access rules)
                    │
        ┌───────────┴─────────────┐
  Claude backend               API backend (future)
  (claude -p, own login)       (rig: Grok, DeepSeek, Anthropic API, Ollama)
                    │
   Tools as MCP servers (GitHub, ClickUp, files). All backends use the same tools.
```

## 1. Existing projects

No project meets all the requirements. These are useful as references:

| Project | Language | What to borrow | Why not fork |
|---|---|---|---|
| [anthropics/knowledge-work-plugins](https://github.com/anthropics/knowledge-work-plugins) ("productivity") | Skills | Task list and summary skills over ClickUp and Slack | Skills only, not an app |
| [moazbuilds/claudeclaw](https://github.com/moazbuilds/claudeclaw) | TS | Personal assistant on `claude`, scheduled check-ins, 4 access levels | TS, no task list across platforms |
| [qwibitai/nanoclaw](https://github.com/qwibitai/nanoclaw) | TS | One container per agent, keys kept outside | Docker, chat-first |
| [zeroclaw-labs/zeroclaw](https://github.com/zeroclaw-labs/zeroclaw) | Rust | SQLite memory with search, supervised mode, Landlock and Bubblewrap sandbox | Uses a copied Claude login token (terms risk) |
| [moltis-org/moltis](https://github.com/moltis-org/moltis) | Rust | Memory per agent, hooks that block tool calls | Not published as a library |

Poor fit: OpenClaw (too large, chat-first), Khoj (AGPL, heavy), Leon (preview), Letta and mem0 (memory parts only), Goose (coding agent, libraries are alpha).

## 2. Claude headless from Rust

- No official Rust SDK. The Agent SDK is TypeScript and Python only ([overview](https://code.claude.com/docs/en/agent-sdk/overview.md)).
- Run the `claude` program as a child process: `claude -p --output-format stream-json --verbose`.
- Conversations: save the `session_id` from the `result` event. Pass `--resume <id>` on the next turn.
- Own tools: an MCP server written with `rmcp`. Pass it with `--mcp-config` and `--strict-mcp-config`.
- Access limits:
  - `--tools ""` removes all built-in tools (file, shell, web).
  - `--allowedTools mcp__<server>__<tool>` allows only named tools.
  - `--permission-mode dontAsk` denies all tools that are not allowed.
  - A `PreToolUse` hook deny holds in all permission modes ([permissions](https://code.claude.com/docs/en/agent-sdk/permissions.md)).
- Community Rust crates exist (`claude-codes`, `claude-agent-sdk-rs`). aifred uses its own small adapter instead, because the stream format is simple.

## 3. Subscription rules

- Allowed: an end user runs the unmodified `claude` program with their own login ([legal](https://code.claude.com/docs/en/legal-and-compliance)).
- Not allowed: an app that reads, stores, or passes on Claude.ai login tokens.
- `claude -p` uses subscription limits today ([support article, 2026-10-07](https://support.claude.com/en/articles/15036540-use-the-claude-agent-sdk-with-your-claude-plan)). Anthropic changed this policy several times in 2026.
- If aifred is shared with other people, it must use API keys.
- Do not set `ANTHROPIC_API_KEY` in the environment. If it is set, `claude -p` uses it instead of the login.

## 4. Rust libraries

| Crate | Use | Note |
|---|---|---|
| [rmcp](https://github.com/modelcontextprotocol/rust-sdk) 3.8.0 | MCP client and server, official | Many breaking releases |
| [rig-core](https://github.com/0xPlaygrounds/rig) 0.46.0 + `rig-rmcp` | Future API backend: Grok, DeepSeek, Anthropic, Ollama, agent loop, MCP bridge | 25 releases in 12 months |
| [genai](https://github.com/jeremychone/rust-genai) 0.7.2 | Alternative to rig if aifred owns the agent loop | No MCP bridge |
| [agent-client-protocol](https://github.com/agentclientprotocol/rust-sdk) 5.0.0 | Drive Codex or Gemini CLI later | Agent owns memory and loop. Not now. |

Rule: pin exact versions. Keep each backend behind the aifred `Backend` trait.

## 5. Future backends (not in the first version)

| | Grok (xAI) | DeepSeek |
|---|---|---|
| Price per 1M tokens, input / output | grok-4.3: $1.25 / $2.50 | deepseek-flash: $0.30 / $1.20, 50% off-peak |
| Tool calls | Yes, parallel | Yes, strict mode is beta |
| Privacy | 30-day retention, no training, US-only option for +10% | Data stored in China, may train on inputs, opt out by email |
| Gotcha | New features come to the Responses API first | Thinking mode: send `reasoning_content` back each turn |

Sources: [xAI models](https://docs.x.ai/docs/models), [xAI security](https://docs.x.ai/developers/faq/security), [DeepSeek pricing](https://api-docs.deepseek.com/quick_start/pricing), [DeepSeek privacy](https://cdn.deepseek.com/policies/en-US/deepseek-privacy-policy.html), [DeepSeek thinking mode](https://api-docs.deepseek.com/guides/thinking_mode).

Caution: work data (ClickUp, GitHub) sent to DeepSeek goes to servers in China. Get approval before you do this.

OpenRouter gives one OpenAI-format API for Grok, DeepSeek, and Claude. Credit purchase fee is about 5.5% (unverified).

## 6. Open questions

- Startup time of each `claude -p` call. The prototype measures it.
- Whether a long-lived `--input-format stream-json` process is worth it.
- Which API format the rig xAI provider uses (Chat Completions or Responses).
