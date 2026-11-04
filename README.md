# AIfred

AIfred is a personal assistant. This is a prototype.

AIfred owns the memory, the conversation history, the tools, and the access rules. The AI service is a part that you can replace. The first service is Claude Code in headless mode (`claude -p`) with your own login. See `docs/research.md` for the decisions.

## Parts

AIfred is one program file, `aifred`. The first word after the name selects the part:

- `aifred`: opens the window.
- `aifred ask`, `chat`, `remember`, `new`, `index`: the terminal commands (see "Use").
- `aifred tool <name>`: one AI tool server. Claude Code starts it for each message. You do not run it yourself.
- `aifred embed-helper <data folder> <socket>`: the meaning search helper. AIfred starts it when it needs it.

The tool servers and the helper run as their own processes, but they use the same file.

The source code:

- `crates/aifred-core`: the shared core. It contains memory, history, the `Backend` trait (`src/backend/mod.rs`), and the Claude backend (`src/backend/claude.rs`).
- `apps/desktop`: the desktop app (Tauri 2, React, Tailwind). Its Rust part (`src-tauri`) makes the `aifred` program. The terminal commands are in `src-tauri/src/cli.rs`.
- `crates/aifred-tools`: the AI tool servers (MCP, stdio):
  - `github`: reads GitHub with your `gh` login.
  - `clickup`: reads ClickUp with the token in the data folder.
  - `tasks`: adds, lists, searches, and finishes local tasks. It can only change the tasks in the data folder.
  - `memory`: searches, adds, and forgets facts about you, and searches old chats.
- `crates/aifred-embed`: the meaning search helper. It is the only part that loads the meaning search model.

## Data

- AIfred keeps memory, chat history, local tasks, and the saved GitHub and ClickUp lists in one SQLite file: `aifred.db` in the data folder (`~/.aifred`, or `$AIFRED_HOME`).
- The settings and the ClickUp token stay in their own files.
- On the first start after the update, AIfred moves the old files into the database: `memory.md`, `history.jsonl`, `history-<time>.jsonl`, and `tasks.json`. Each old file gets the name `<name>.imported`. AIfred does not delete it.
- If an old file cannot be read, AIfred stops and shows the error. It saves nothing until you repair the file.
- "New conversation" keeps the old messages in the database. AIfred can search them.

## Memory

- "What AIfred knows about you" in the sidebar is a list of facts. You can add, edit, and forget a fact. A forgotten fact goes to the "Forgotten" list, where you can restore it.
- While all facts together are 2,000 tokens or less (about 8,000 characters), all of them go into every message.
- When memory is larger, only the facts with "Always" on go into every message. For each question, the app also adds up to 8 other facts that match the question. Under the answer, "Used N facts from memory" shows them. AIfred can also search for more facts.
- The facts that you add have "Always" on. AIfred can add facts in the chat. Those have "Always" off, unless AIfred decides that the fact matters in most chats.
- AIfred can also search your old chats, your tasks, and the saved GitHub and ClickUp lists. A GitHub item or ClickUp task that left your list stays searchable.
- Search finds words and meaning. For example, "book my flight" finds the fact "I prefer window seats".
- Meaning search uses a small model that runs on your computer (all-MiniLM-L6-v2, about 90 MB). It uses the processor and RAM, not the GPU. The first start downloads it into `models` in the data folder. If the download fails, word search still works, and AIfred tries the download again after 1 minute. The sidebar shows the state under "Meaning search".
- Only the meaning search helper loads the model. The app, the terminal commands, and the tool servers send their texts to it, so only one copy of the model is in memory (about 150 MB while it runs).
  - A program starts the helper when it needs it. The start takes about 0.2 s.
  - The helper stops after 5 minutes with no requests. Then the model uses no memory.
  - The helper's messages go to `embed.log` in the data folder.
- AIfred writes short summaries of chats. Each summary is one more AI call on your Claude login:
  - When you start a new conversation, the old one gets a summary, if it has at least 2,000 characters.
  - A long conversation gets a new summary each time 20,000 tokens (about 80,000 characters) of new messages are not in the summary yet.
  - AIfred searches the summaries together with the messages. When Claude cannot continue a session, AIfred sends the summary and the messages after it (at most 100,000 characters, newest first).
- AIfred saves the description of each GitHub item and ClickUp task when it downloads the list. A local task can have a description too: click the task to change it.
- Click a local task to change it. A local task can have these fields:
  - a description,
  - a due day,
  - a start day: before this day, the task waits under "Later" in the list,
  - a priority: urgent, high, normal, or low,
  - tags, for example "work" or "travel",
  - a checklist of steps. You can tick a step in the list without opening the task.
- Open tasks show the nearest due day first. Tasks with the same due day show the highest priority first.
- AIfred can set these fields when you give them in the chat, for example "remind me to pay rent by Friday, high priority".
- A long description (more than 300 characters) gets a short summary. A shorter description is its own summary.
  - The fast model (Haiku) writes the summaries in the background, 15 items in one AI call on your Claude login.
  - It runs after a list download, after an answer, and after you add or change a task. It only writes the summaries that are missing.
  - A summary is made again when its title or description changes.
- When you ask what to do or what is open, the AI calls one overview tool. It gets all open tasks and items with their summaries, and it opens the full details only of the items that need it.
- `aifred index` makes the missing meaning search data and item summaries now, for example after a first start with no network.

## Chat model

- Pick the model in the chat box, next to Send. AIfred remembers the last pick, also for `aifred ask` and `aifred chat`.
- Fable is the most careful and the slowest. Haiku is the fastest. "Default" uses `AIFRED_MODEL`, or else the Claude Code default.
- The pick is in the database (`aifred.db`), not in `settings.json`.

## Desktop app

Run these commands one time:

```bash
cd apps/desktop && npm install
```

Start the app:

```bash
cd apps/desktop && npx tauri dev
```

The tool servers and the meaning search helper are in the same program, so you do not build them separately.

### Tray

- AIfred shows an icon in the system tray. Closing the window does not quit AIfred: the window hides and AIfred keeps running.
- To show the window again, click the tray icon and choose "Show AIfred". To quit, choose "Quit" in the tray menu.
- On Linux, a click on the tray icon opens the menu.
- On Linux, the tray needs libayatana-appindicator (or libappindicator) and a desktop that shows tray icons. GNOME needs the AppIndicator extension.
- If AIfred cannot make the tray icon, closing the window quits AIfred.

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

Make the release program (one file, `target/release/aifred`):

```bash
cd apps/desktop && npx tauri build --no-bundle
```

- Use the Tauri command, not `cargo build --release`. Only the Tauri command puts the window files into the program.
- Release builds remove the debug symbols (`[profile.release]` in `Cargo.toml`).
- The program needs WebKitGTK 4.1 on the computer.
- The tray icon needs libayatana-appindicator at run time.

## Use

```bash
aifred remember "I work on the StoreAgent plugin."
aifred ask "What GitHub items need my attention?"
aifred ask "Summarize item 2."     # continues the same conversation
aifred chat                        # talk in a loop
aifred new                         # start a new conversation
aifred version                     # show the version
```

## Updates

The release program updates itself from the GitHub releases of this repository.

1. 20 seconds after the window opens, and then every 6 hours, AIfred asks GitHub for a newer version. Pre-releases count. "Check now" on the Settings page asks at once.
2. When it finds one, it downloads the file for your computer (for example `aifred-0.1.2-linux-x86_64`) next to the program, under a temporary name.
3. It compares the file's SHA-256 sum with the sum from GitHub. Then it runs `<file> version` one time, to see that the file starts on your computer. Only then does it rename the file to `.aifred-update`. If a step fails, it removes the new file, keeps an earlier checked file, and shows the error on the Settings page. A download stops when no data comes for 60 seconds, or after 30 minutes.
4. The sidebar shows "AIfred <version> is ready" and a "Restart now" button. The button puts the new file in place of the program, starts it, and closes this window. The chat, the tasks, and the window size stay.
5. If you quit AIfred (tray menu "Quit") without a restart, AIfred puts the new file in place when it quits. The next start uses it.

- AIfred replaces the program file only at a restart or a quit. Before that, the AI tool servers that Claude Code starts use the same version as the window.
- Only one AIfred window at a time checks for updates. A second window shows "Another AIfred window checks for updates."
- If a newer release is removed from GitHub, the next check removes its downloaded file, so AIfred does not install it.
- AIfred must be able to write to the folder of the program file. If it cannot, the Settings page shows the error.
- Development builds (`npx tauri dev`) do not update themselves.
- `aifred version` shows the version.
- A release must have its program file with the name `aifred-<version>-linux-x86_64`. Versions 0.1.1 and older cannot update themselves. Install the first version with updates by hand.

## Settings (environment variables)

- `AIFRED_HOME`: data folder. Default: `~/.aifred`.
- `AIFRED_GITHUB_OWNERS`: comma-separated GitHub owners. GitHub tools refuse all other owners.
- `AIFRED_CLICKUP_TOKEN`: ClickUp personal token. It overrides the token from the Settings page.
- `AIFRED_MODEL`: Claude model name for "Default" in the chat box. Default: the Claude Code default.

Do not set `ANTHROPIC_API_KEY`. If it is set, `claude` uses the API key and not your login.

## Access limits

- Claude gets no built-in tools: no shell, no file access, no web.
- Claude gets only the MCP servers and tools that AIfred names. All other tools are denied.
- The GitHub and ClickUp tools that Claude uses can only read.
- The desktop app also only reads GitHub. It never changes or closes an issue or pull request.

## GitHub list

- AIfred saves the last GitHub list in the database (`aifred.db`) in the data folder. When the window opens, it shows the saved list.
- AIfred loads the list from GitHub again on a timer. Set the time on the Settings page (default: 15 minutes). The reload button always loads from GitHub.
- GitHub items have no checkbox. An item leaves the list when it is closed on GitHub, at the next reload.
- When your pull request closes one of your issues, the list shows one row for the issue. The pull request shows as a badge under the issue title.

## ClickUp list

- Connect ClickUp on the Settings page. Paste your personal API token (ClickUp: Settings > Apps > API Token). AIfred checks the token with ClickUp, then saves it in `clickup-token` in the data folder. Only you can read that file.
- The list shows the open ClickUp tasks assigned to you, in all your workspaces. Tasks in a "done" or "closed" status do not show. It uses the same database and timer as the GitHub list.
- Tick a ClickUp task to close it in ClickUp. AIfred moves the task to the "closed" status of its list, like the ClickUp "mark complete" button. An Undo puts the old status back.
- This tick is the only change AIfred makes in ClickUp. The chat can only read ClickUp.
- The token never goes into the tool settings file that Claude reads. The ClickUp tool server reads the token itself.

## Check the app without a window on your screen

```bash
scripts/run-hidden.sh 300 /tmp/app.png
```

This starts the app on a hidden screen (Xvfb). It waits up to 300 seconds until the page shows, saves a picture, and stops the app. It does not change your own copy of the app:

- It uses a different web port, so it works while your copy runs.
- It does not read or write your saved window size. The setting `AIFRED_NO_WINDOW_STATE=1` turns that off.
- It uses a temporary copy of your data folder.
