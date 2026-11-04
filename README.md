# AIfred

AIfred is a personal assistant. This is a prototype.

AIfred owns the memory, the conversation history, the tools, and the access rules. The AI service is a part that you can replace. The first service is Claude Code in headless mode (`claude -p`) with your own login. See `docs/research.md` for the decisions.

## Parts

- `crates/aifred-core`: the shared core. It contains memory, history, the `Backend` trait (`src/backend/mod.rs`), and the Claude backend (`src/backend/claude.rs`).
- `crates/aifred`: the terminal commands.
- `apps/desktop`: the desktop app (Tauri 2, React, Tailwind).
- `crates/aifred-github-mcp`: a read-only GitHub tool server (MCP, stdio). It uses your `gh` login.
- `crates/aifred-tasks-mcp`: a local task tool server. AIfred uses it to add, list, and finish tasks. It can only change `tasks.json` in the data folder.

## Desktop app

Run these commands one time:

```bash
cargo build                    # builds the GitHub and task tool servers
cd apps/desktop && npm install
```

Start the app:

```bash
cd apps/desktop && npx tauri dev
```

The app looks for the tool servers (`aifred-github-mcp`, `aifred-tasks-mcp`) in the same folder as the app program (`target/debug`). If one is missing, the chat still works without those tools, and the chat panel shows a notice. An installed app package does not include the tool servers yet.

## Tests

The tests do not call Claude or GitHub. They cost nothing and open no windows.

```bash
cargo test --workspace             # Rust: tasks, GitHub parsing, task tool, app commands
cd apps/desktop && npm test        # window: home page, chat panel, to-do list
```

- The app command tests use the Tauri fake runtime (the `test` feature of `tauri`).
- The window tests use Vitest in a fake browser (jsdom). A fake backend in `apps/desktop/src/test/fakeBackend.ts` replaces the Rust app.
- If you add or rename a command, update the fake backend too.

## Build

```bash
cargo build --release
```

## Use

```bash
aifred remember "I work on the StoreAgent plugin."
aifred ask "What GitHub items need my attention?"
aifred ask "Summarize item 2."     # continues the same conversation
aifred chat                        # talk in a loop
aifred new                         # start a new conversation
```

## Settings (environment variables)

- `AIFRED_HOME`: data folder. Default: `~/.aifred`.
- `AIFRED_GITHUB_OWNERS`: comma-separated GitHub owners. GitHub tools refuse all other owners.
- `AIFRED_MODEL`: Claude model name. Default: the Claude Code default.

Do not set `ANTHROPIC_API_KEY`. If it is set, `claude` uses the API key and not your login.

## Access limits

- Claude gets no built-in tools: no shell, no file access, no web.
- Claude gets only the MCP servers and tools that AIfred names. All other tools are denied.
- The GitHub tools that Claude uses can only read.
- The desktop app also only reads GitHub. It never changes or closes an issue or pull request.

## GitHub list

- AIfred saves the last GitHub list in `github-cache.json` in the data folder. When the window opens, it shows the saved list.
- AIfred loads the list from GitHub again on a timer. Set the time on the Settings page (default: 15 minutes). The reload button always loads from GitHub.
- GitHub items have no checkbox. An item leaves the list when it is closed on GitHub, at the next reload.
- When your pull request closes one of your issues, the list shows one row for the issue. The pull request shows as a badge under the issue title.

## Check the app without a window on your screen

```bash
scripts/run-hidden.sh 300 /tmp/app.png
```

This starts the app on a hidden screen (Xvfb). It waits up to 300 seconds until the page shows, saves a picture, and stops the app. It does not change your own copy of the app:

- It uses a different web port, so it works while your copy runs.
- It does not read or write your saved window size. The setting `AIFRED_NO_WINDOW_STATE=1` turns that off.
- It uses a temporary copy of your data folder.
