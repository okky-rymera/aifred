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
  - `jobs`: makes, lists, changes, and stops AIfred's own jobs.
  - `memory`: searches, adds, and forgets facts about you, and searches old chats.
- `crates/aifred-embed`: the meaning search helper. It is the only part that loads the meaning search model.

## Data

- AIfred keeps memory, chat history, local tasks, and the saved GitHub and ClickUp lists in one SQLite file: `aifred.db` in the data folder (`~/.aifred`, or `$AIFRED_HOME`).
- The settings, the ClickUp token, and the Google connection (`google.json`) stay in their own files.
- On the first start after the update, AIfred moves the old files into the database: `memory.md`, `history.jsonl`, `history-<time>.jsonl`, and `tasks.json`. Each old file gets the name `<name>.imported`. AIfred does not delete it.
- If an old file cannot be read, AIfred stops and shows the error. It saves nothing until you repair the file.
- "New conversation" keeps the old messages in the database. AIfred can search them.

## Memory

- "What AIfred knows about you" in the sidebar is a list of facts. You can add, edit, and forget a fact. A forgotten fact goes to the "Forgotten" list, where you can restore it.
- In the "Forgotten" list, "Delete" removes a fact for good. Click it two times. AIfred then writes the full database file again (VACUUM), so no old copy of the text stays in the file. This also removes old copies from earlier edits. When another AIfred program uses the database at that time, the clean-up waits until the next delete, and the app tells you. Old chat messages that mention the fact stay.
- While all facts together are 2,000 tokens or less (about 8,000 characters), all of them go into every message.
- When memory is larger, only the facts with "Always" on go into every message. For each question, the app also adds up to 8 other facts that match the question. Under the answer, "Used N facts from memory" shows them. AIfred can also search for more facts.
- A fact can have up to 5 tags, for example `testing` or `people`. AIfred gives each new fact 1 to 3 tags and reuses the tags that exist. Click "Tags" under a fact to change them (commas between tags). Click a tag to show only its facts.
- AIfred can list the tags and read all facts with one tag. The tag names are not in each message: AIfred asks for them when it needs them.
- In the chat, you can ask AIfred to change a fact (`edit_memory`), to turn "Always" on or off (`set_memory_always`), or to restore a forgotten fact (`restore_memory`).
- The facts that you add have "Always" on. AIfred can add facts in the chat. Those have "Always" off, unless AIfred decides that the fact matters in most chats.
- AIfred can also search your old chats, your tasks, and the saved GitHub and ClickUp lists. A GitHub item or ClickUp task that left your list stays searchable.
- Search finds words and meaning. For example, "book my flight" finds the fact "I prefer window seats".
- Meaning search uses a small model that runs on your computer (all-MiniLM-L6-v2, about 90 MB). It uses the processor and RAM, not the GPU. The first start downloads it into `models` in the data folder. If the download fails, word search still works, and AIfred tries the download again after 1 minute. The sidebar shows the state under "Meaning search".
- Only the meaning search helper loads the model. The app, the terminal commands, and the tool servers send their texts to it, so only one copy of the model is in memory (about 150 MB while it runs).
  - A program starts the helper when it needs it. The start takes about 0.2 s.
  - The helper stops after 5 minutes with no requests. Then the model uses no memory.
  - The helper's messages go to `embed.log` in the data folder.
- The chat starts fresh each day, with a new Claude session. The app looks for the day change at start-up and each minute while it runs. The messages of every day stay in the database.
- AIfred writes short summaries of chats. Each summary is one more AI call on your Claude login:
  - Each past day with chats gets a summary: the main chat and the side chats (plan, item, mission) of that day. A day with less than 2,000 characters of messages gets none: a search finds its few messages directly. The day that ended gets its summary at start-up, or at the first minute of the new day while the app runs. After each answer, one more waiting day gets its summary, the newest first. At start-up, at most 5 waiting days get theirs. So the days from before the update get their summaries over a few starts and answers, one AI call each.
  - Each quarter gets a summary from its day summaries, after the quarter ends.
  - A long conversation gets a new summary each time 20,000 tokens (about 80,000 characters) of new messages are not in the summary yet. When Claude cannot continue a session, AIfred sends this summary and the messages after it (at most 100,000 characters, newest first).
  - Each message gets the day and quarter summaries that match it (at most 2), and the summary of the last day goes in as soon as it exists. A summary goes into a session only once. The AI can also search the summaries and the messages (`search_history`), list the summaries of a range of days (`day_summaries`), and read all messages of one day (`day_messages`).
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
- The box on the home page has two modes. Press Tab in the box, or click the chip under it, to change the mode. Shift+Tab still moves the focus out of the box. The mode stays when you go to another page.
  - "Ask AIfred" (the start mode): the message goes to AIfred. AIfred decides what to do:
    - a short to-do line, for example "pay rent friday high #home", becomes a task with its due day, priority, and tags,
    - a question gets an answer,
    - "remind me…" or "check … later" becomes one of AIfred's jobs,
    - only "plan my day" works with no AI decision: it opens the Calendar.
    Each message is one AI call, also a new task. It takes some seconds.
  - "Add task": the box has a green border and no model picker. The line becomes a task at once, with no AI call. AIfred reads the due day ("friday", "tomorrow", "2027-02-01"), the priority at the end ("high", "urgent", "low"), and the tags ("#home") from the words. The line under the box shows what it read. An "Added" message has an Undo. This works while AIfred answers, and with no network.
- A file that you send with a new task stays on the task, in both modes.
- A long description (more than 300 characters) gets a short summary. A shorter description is its own summary.
  - The fast model (Haiku) writes the summaries in the background, 15 items in one AI call on your Claude login.
  - It runs after a list download, after an answer, and after you add or change a task. It only writes the summaries that are missing.
  - A summary is made again when its title or description changes.
- When you ask what to do or what is open, the AI calls one overview tool. It gets all open tasks and items with their summaries, and it opens the full details only of the items that need it.
- `aifred index` makes the missing meaning search data and item summaries now, for example after a first start with no network.

## AIfred's jobs

AIfred's jobs are the tasks that AIfred does by itself. It does them while it runs, also when the window is closed to the tray. The Tasks page shows them next to your tasks.

- Ask in the chat. Examples:
  - "Remind me every Monday at 9 to send my timesheet."
  - "Remind me on the 3rd and 17th to pay the invoices."
  - "In 2 hours, check if PR #123 is merged and tell me."
  - "Add 'pay rent' to my tasks on the 1st of each month."
- AIfred says the time back with the next run dates. Check them.
- A job is one of these:
  - a reminder: a desktop notification, with no AI call,
  - an AI job: AIfred runs at that time, looks things up, and sends a notification with the result. If it has nothing to tell, it sends nothing.
  - a repeating task: at each time, AIfred adds a new task to your list, with that day as the due day, and sends a notification. The rule shows under "AIfred's" on the Tasks page. Each new task shows under "Mine". Each copy keeps its own done date, so the end-of-day review shows each one.
- An AI job can only read. It cannot change tasks, the plan, memory, GitHub, or ClickUp. It cannot make new jobs.
- Each AI job run is one more AI call on your Claude login (Sonnet).
- A job runs once, at a time and repeat (for example "every other Thursday", "the last Friday of the month"), or after an event. The repeat rule is an RRULE (RFC 5545). The time zone is the zone of your computer when you make the job.
- AIfred has one job from the start: "Update the brief when you finish a task". It waits 90 seconds for more finished tasks, so many tasks give one update (at most 10 minutes after the first one). It only updates a brief that already exists for today.
- The Tasks page (the check box in the left bar) shows your tasks and AIfred's jobs. The filter at the top has three values:
  - "Mine" (the start value): your tasks, GitHub items, ClickUp tasks, and emails, in the same groups as on the home page,
  - "AIfred's": the jobs, the next run, and the latest runs,
  - "Both": the two lists, your tasks first.
  The page keeps the last filter. "See all tasks" on the home page opens it.
- On the Tasks page, you can pause, start, and delete a job. To change a job, ask in the chat.
- The job "Update the brief when you finish a task" is not on the Tasks page. It is on the Settings page (General tab), under "Brief". You can pause it there.
- The AI gets your tasks and AIfred's active and paused jobs in one list. Each entry says who does it: "you" or "aifred". So the AI knows what AIfred already checks. A paused job shows only its title and status, not its message. The brief and the day plan do not pick AIfred's jobs.
- At most 20 jobs that are active or paused. A job can repeat at most once an hour.
- If AIfred was not running at a job time (closed, or the computer was asleep):
  - a one-time job runs late, and the notification says the planned time,
  - a repeating job runs one time for the newest missed time, if it is at most 12 hours old. Older times are skipped.
  - a repeating task has no 12-hour limit: AIfred adds one task for each missed time, with the day of that time as the due day. It adds them one after the other, oldest first. After a very long gap, only the newest 20 missed times get a task. The older times are skipped.
- A job that fails tries again after 1, 2, 4, and 8 minutes. After 5 failures in a row it turns off, and AIfred shows a notification.
- A run that was busy when AIfred stopped does not run again. A repeating task is different: AIfred adds the task for that time after it starts again. It never adds the same task two times.
- A repeating task whose add failed tries again also after you pause and start it. If you give it a new time, the failed old time gets no task. The run list shows why.
- See `docs/research-agent-jobs.md` for the research and the next steps.

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

1. 20 seconds after the window opens, and then every 6 hours, AIfred asks GitHub for a newer version. Pre-releases count. "Check now" on the Settings page (About tab) asks at once.
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

## Settings page

The Settings page has 5 tabs:

- **General:** the reload time, your work day, and the brief. "Save" is on this tab. A change on this tab stays when you open another tab, until you save it.
- **Connections:** ClickUp and Google.
- **ClickUp statuses:** what each ClickUp status means in AIfred (see "ClickUp statuses").
- **Activity:** AIfred's actions.
- **About:** updates, and facts about this app.

The arrow keys move between the tabs.

## Settings (environment variables)

- `AIFRED_HOME`: data folder. Default: `~/.aifred`.
- `AIFRED_GITHUB_OWNERS`: comma-separated GitHub owners. GitHub tools refuse all other owners.
- `AIFRED_CLICKUP_TOKEN`: ClickUp personal token. It overrides the token from the Settings page.
- `AIFRED_MODEL`: Claude model name for "Default" in the chat box. Default: the Claude Code default.

Do not set `ANTHROPIC_API_KEY`. If it is set, `claude` uses the API key and not your login.

## Access limits

- Claude gets no built-in tools: no shell, no file access, no web.
- Claude gets only the MCP servers and tools that AIfred names. All other tools are denied.
- The GitHub tools that Claude uses can only read.
- The ClickUp tools can only read, with one exception. The question chat, the chat of a ClickUp task, and a mission chat can mark a ClickUp task done and write a comment on it. Each change has an undo.
- The desktop app also only reads GitHub. It never changes or closes an issue or pull request.

## GitHub list

- AIfred saves the last GitHub list in the database (`aifred.db`) in the data folder. When the window opens, it shows the saved list.
- AIfred loads the list from GitHub again on a timer. Set the time on the Settings page (default: 15 minutes). The reload button always loads from GitHub.
- GitHub items have no checkbox. An item leaves the list when it is closed on GitHub, at the next reload.
- When your pull request closes one of your issues, the list shows one row for the issue. The pull request shows as a badge under the issue title.

## ClickUp list

- Connect ClickUp on the Settings page (Connections tab). Paste your personal API token (ClickUp: Settings > Apps > API Token). AIfred checks the token with ClickUp, then saves it in `clickup-token` in the data folder. Only you can read that file.
- The list shows the open ClickUp tasks assigned to you, in all your workspaces. Tasks in a "done" or "closed" status do not show. It uses the same database and timer as the GitHub list.
- Each row shows AIfred's own status (see "Item status"). The ClickUp status name is in the tooltip and in the item panel.
- Tick a ClickUp task to close it in ClickUp. AIfred moves the task to the "closed" status of its list, like the ClickUp "mark complete" button. An Undo puts the old status back.
- The tick and these chats are the only places where AIfred changes ClickUp: the question chat of the home page, the chat of a ClickUp task, and a mission chat. These chats can mark a ClickUp task done (`mark_task_done`) and write a comment on it (`add_task_comment`), in your words. Ask the chat for the undo (`reopen_task`, `delete_task_comment`).
- The token never goes into the tool settings file that Claude reads. The ClickUp tool server reads the token itself.

## Item status

Each item has AIfred's own status. It is the same for local tasks, GitHub items, and ClickUp tasks (`crates/aifred-core/src/status.rs`). A ClickUp status name can mean another thing in each list, so AIfred does not use the name as its status.

| Status | Open or closed | Meaning |
| --- | --- | --- |
| To do | open | Nobody started the work. |
| In progress | open | You started the work. |
| Waiting | open | The work waits for another item or another person. |
| Done | closed | The work is finished. |
| Unfinished | closed | The team worked on it, but did not finish it in its time. Another task can continue it, for example the copy in the next quarter. With no such task, you accept it as it is. |
| Cancelled | closed | You decided not to do it. |

- Unfinished and Cancelled are not finished work. AIfred keeps no work time for them. No "task done" job starts. They are not in "finished today" or in the goal counts of the weekly review.
- The status is only in AIfred. Only Done of a ClickUp task changes ClickUp: it is the same as the tick.
- Change the status in the item panel. Each change has an Undo. The chat can also change it (`set_item_status`).
- When two sources disagree, AIfred uses the first one in this list:
  1. The status that you picked. A weekly review "drop" is Cancelled.
  2. How the item closed in ClickUp or GitHub. After an item leaves the list, AIfred reads why, one time for each item, for up to 7 days. A GitHub issue closed as "not planned" or as a duplicate, and a pull request closed with no merge, are Cancelled.
  3. Your override for the ClickUp status (see "ClickUp statuses").
  4. Waiting: you gave the item away in the weekly review, or it has an open blocker.
  5. In progress: the item has work time. It stays In progress after the timer stops.
  6. The ClickUp status group: Not started is To do, Active is In progress, Done and Closed are Done. Else To do.
- AIfred does not guess from the status name. Only the group and your overrides count.
- A ClickUp task can be open in ClickUp and closed in AIfred. For example, with an override that makes "delayed" Unfinished, a rock in "delayed" is closed, so the brief does not rank it.

### Continued in

- An Unfinished item can link to the item that continues it. In the item panel, "Continued in" shows the link. "+ Link the task that continues it" finds an open item by its title.
- With no link, the panel says "Accepted as it is".
- The item that continues it shows "Continues" with the Unfinished item. The brief gets this as `continues`.
- Each change has an Undo. The chat can set the link with `set_item_status` (`continued_in`).
- The link counts only while the item is Unfinished.

### ClickUp statuses

Settings > ClickUp statuses follows the "group defaults plus overrides" pattern of status mapping screens.

- **Defaults:** a fixed table. Each ClickUp status group gives one AIfred status:

  | ClickUp group | AIfred status |
  | --- | --- |
  | Not started | To do |
  | Active | In progress |
  | Done | Done |
  | Closed | Done |

- **Overrides:** a table with one row for each override: Space, List, ClickUp status (with its group), AIfred status, and how many of your tasks it changes now.
- "+ Add override" adds a row. Pick the space, then the list or "Any list in the space", then the ClickUp status, then the AIfred status. The AIfred status starts at the group default.
- ClickUp keeps statuses on spaces, folders, and lists. A list can use the statuses of its space, or have its own. So for "Any list in the space", AIfred offers the statuses of the space and of each list in it. AIfred reads them from ClickUp when you pick the space (one call for the space, and one for each list).
- An override for "Any list in the space" also covers the new lists in that space. For example, "delayed" in the "Company Wide" space also counts for the Rocks list of the next quarter.
- An override for one list wins over one for its space.
- In a saved row, change the AIfred status in place. Each change and each "Remove" has an Undo.
- The overrides change only AIfred, not ClickUp.
- In the chat, you can ask for an override, for example "In QA means waiting" (`set_status_rule`, `remove_status_rule`). Only the main chat and the plan chat can change the overrides.

## Subtasks

An item can be a subtask of another item, with many levels. For example, the GitHub issue goes under the ClickUp task DEV-123, and the pull request goes under the issue. The subtasks exist only in AIfred. ClickUp and GitHub do not change (`crates/aifred-core/src/links.rs`).

- The home list shows one row for each tree. The row is the top item. Its open subtasks show under it after "Subtasks". Click one to open it.
- The Tasks page shows the same rows, but the open subtasks of each row show as a tree under it, each level indented. Each subtask has its own tick, its source, and its due day.
- When the top item is finished or closed, the next open item of the tree becomes the row.
- The row shows the earliest due day and the highest priority of its open items. So a GitHub row with a late ClickUp task in it shows under "Needs you now".
- The tick, "Start", and "+ Plan" of the row act on the top item.
- The item panel shows the tree under "Subtasks", each level indented.
- To add a subtask, open the parent item and click "+ Add a subtask". Type words of the other item's title.
- "Move out" takes an item out from under its parent. Its own subtasks go with it.
- An item cannot go under itself or under one of its own subtasks.
- When an item is deleted, or its source is disconnected, its subtasks move up to its parent.
- Each change has an Undo.
- One tree can have at most 10 items.
- Each link keeps who placed it (you or AIfred) and AIfred's reason. The item panel shows AIfred's reason next to the item.
- In the chat, you can ask AIfred to put an item under another item, or to move it out (the `move_item` tool).

### AIfred sorts new items

When new items come in, AIfred puts them into trees by itself. It does not ask first (`crates/aifred-core/src/sort.rs`).

- The sort runs after each GitHub, ClickUp, or Gmail download, after you add a task, after a chat, and when the app starts.
- Only new items cost an AI call. One call sorts up to 20 new items. With only one open item, there is no call.
- Clues help the AI: a pull request closes an issue, or one item names the other (a GitHub link, `owner/repo#7`, a ClickUp link, or `DEV-42`).
- The parent is the item that plans or holds the work. A ClickUp task is the parent of its GitHub issue. An issue is the parent of its pull request.
- Each move has a new item in it. So AIfred does not move old items around again and again.
- AIfred never moves an item that you placed. Your own move, "Move out", and an Undo of AIfred's move all count as your choice.
- An Undo of your own move puts everything back as it was before the move. AIfred can sort the items again, as before.
- The brief tells you what AIfred moved since the brief of the day before, with the reason. For one move: "I made X a subtask of Y." For more: "I reorganized several tasks:" and one line for each parent. The app writes this part from a log, not the AI, so it is always true.
- Each line has an Undo. The item goes back to where it was, and AIfred does not move it again.
- The note shows at most 5 lines first. "Show N more" shows the rest. The first sort after the update can move many old items.
- The screen shows the new trees and the note as soon as a sort moves items. The note needs today's brief: without a brief, open the item to see AIfred's reason.

The brief and the chat see the trees: each subtask has a `parent` in the overview. When the parent is closed, the next open item above it is the parent.

## Blockers

An item can wait until other items are finished. For example, "Deploy the fix" waits for "Review PR #12". Blockers exist only in AIfred. ClickUp and GitHub do not change (`crates/aifred-core/src/blockers.rs`).

- Any item can wait or block: a task, a GitHub item, a ClickUp task, or an email.
- An item can wait for at most 10 open items. Finished blockers, and blockers that are not in your list now, do not count. Many items can wait for one item.
- To add a blocker, open the item that waits and click "+ Add a blocker" under "Blocked by". Type words of the other item's title.
- The item panel shows what the item waits for under "Blocked by", and what waits for it under "This item blocks". "Remove" takes a link away. Each change has an Undo.
- A row that waits for an open item shows "Blocked by" and the names of the open blockers. Click a name to open it. A mission shows the same line for its first open item. A block in the Calendar shows "Blocked by" with the names.
- When a blocker is finished, it does not block any more. The link stays, so it blocks again when the item opens again.
- You can still finish a blocked item. The link only informs.
- An item cannot wait for itself. Two open items cannot wait for each other, also not through other items. Links to finished items do not count for this check. So when a finished item opens again, two items can wait for each other. Both links show in the item panel, and you can remove one.
- When an item is deleted, or its source is disconnected, its links go away.

In the chat, say for example "the deploy task waits for PR #12". AIfred sets the link, and the panel shows "Set by AIfred in a chat". AIfred can also set a link by itself when a plain fact shows the order, for example an issue that says it depends on another issue. Then the panel shows "AIfred:" and the reason. When you remove a link in the item panel, AIfred never adds it again, also not when the chat says that you asked. Only the item panel can add it again. When AIfred removes a link (for example when you ask in the chat), AIfred can add it again later.

The brief and the chat see the links: each open item that waits for open items has `blocked_by` in the overview. The day plan tool gives `blocked_by` for each planned item too. The brief and the day plan put a blocked item after its open blockers. A blocked item stays in the brief and the day plan. A link does not make an item more urgent or more important.

## Missions

A mission is the next small piece of work: usually one open item, sometimes two small items that you do together. For a tree of subtasks, the mission is the next open subtask, not the full tree (`crates/aifred-core/src/missions.rs`).

- The brief call gives at most 7 missions for today, the most important first. It costs no extra AI call.
- The home page shows only these missions. "Needs you now" shows the urgent items that are in no mission. The other open items are behind the line "Show other open items (N)". "See all tasks" opens the Tasks page with all items.
- Before the brief makes missions, the home page shows the list in groups, as before.
- A mission shows the reason, the next action, and the goal of its first item. Its first item is the item of the next action.
- When the first item is a subtask, the mission shows its parent: "Part of DEV-123 Checkout totals". Click the parent to open it.
- An item is in at most one mission. A mission has only the items that the brief named, not their full tree (see "Subtasks"). A mission is the next small piece of work.
- AIfred groups items into a mission by itself. It does not ask you first.
- A mission keeps its checklist from day to day while its items stay. The brief keeps a mission when it gives the same mission id and some items stay, or when half of its open items or more stay.
- A new mission with more than one action gets 2 to 6 steps from the brief.
- Progress is (steps done + items closed) / (all steps + all items). The panel shows the numbers. For example, 1 of 2 steps done and 1 of 3 items closed is (1 + 1) / (2 + 3) = 40%.
- Click a mission to open its panel. The panel shows its items, its checklist, and these actions:
  - "Start": starts the work timer on the first open item.
  - "Done": asks which items are finished too. Then each picked item closes. One Undo opens the mission and its closed tasks and emails again. A closed ClickUp task keeps its own Undo. A GitHub item cannot close here, because AIfred only reads GitHub.
  - "Not today" and "Wrong priority": a correction of each open item of the mission (see "Corrections of the brief"). The mission leaves the home page for today. The message has an undo.
  - "Take out" on an item: the item leaves the mission. The brief never puts it back in this mission. It can still get a mission of its own. The last open item cannot go out: use "Done" or "Not today".
  - "Add an item": pick an open item. It leaves its other mission. The brief does not take it away from this mission.
  - Each "Take out" and "Add an item" message has an undo. The item goes back to where it was.
- In the chat, you can ask AIfred to add an item to a mission or to take one out (the `add_to_mission` and `remove_from_mission` tools). The same rules apply.
- In the chat, you can also ask AIfred to rename a mission (`rename_mission`), change its steps (`set_mission_steps`), tick a step (`tick_mission_step`), finish it (`finish_mission`), open it again (`reopen_mission`), or say "Not today" (`skip_mission_today`).
- The brief marks a task or a mission step done when it is the same work as a ClickUp or GitHub item that closed as Done in the last 7 days. It changes only your own tasks and mission steps, never a ClickUp or GitHub item. The home page shows "What AIfred marked done" with an Undo. After an Undo, the brief does not make that match again. The chat has no undo button: ask the chat to change it back. The home page shows the change when the answer comes.
- A tick in the checklist does not change the items.
- An open mission that did not change for 14 days, and that today's brief did not rank, goes away.

## What AIfred knows about an item

AIfred keeps short notes about an item: what it is, how it links to other work, and how urgent it is. The notes come from your answers (`crates/aifred-core/src/context.rs`).

- Each brief finds the items that AIfred cannot rank. Such an item has a title of 4 words or fewer, no description, no due day, no parent, no goal, and no notes. Your tasks, GitHub items, and ClickUp tasks can be such items. Emails and AIfred's own tasks are never asked about.
- The card "Help AIfred understand" on the home page asks about at most 3 of these items each day. The items of the brief come first. It costs no AI call: a fixed rule picks the items.
- Type your answer in the card. The item panel opens, and the answer goes to the chat of the item. AIfred saves a short summary with the `set_item_context` tool (at most 600 characters). Then the item leaves the card.
- "Skip": AIfred does not ask about the item again. The message has an undo.
- The brief, the overview, and the chat of the item get the notes. In the chat of any item, tell AIfred new facts, and it changes the notes.
- The item panel shows the notes under "What AIfred knows", closed at the start. You can change them or clear them. Each change has an undo.
- AIfred saves only facts that you gave. It never saves text from emails, GitHub, or ClickUp as notes.

## Suggested work

- For each item that the brief names, the brief also gives the next action, for example "Next: Reply to Ana with the price." It shows under the reason in the home list and in the item panel. It costs no extra AI call: the brief call writes it.
- A label says who does the next action:
  - "You do this": only you can do it, for example a decision, a call, code, or a review.
  - "AIfred can write a reply to (name)": the item is an email from that person. Use "Write a reply" in the item panel (see "Reply drafts" below). An email to another person, you write yourself.
  - "AIfred can draft the email later": AIfred cannot do this yet. When it can, it makes a draft for you to check. It sends nothing by itself.
  - "AIfred can draft the Doc text later": use "Suggest changes" under "Doc" in the item panel (see "Doc suggestions" below).
  - "AIfred can comment on the Sheet": use "Suggest comments" under "Sheet" in the item panel (see "Sheet comments" below).
- "✦ Suggest steps" in the item panel splits an item into 2 to 8 steps. It is one AI call (Sonnet) on your Claude login. It saves nothing until you pick:
  - Clear the box of a step that you do not want, then click "Add N steps".
  - For your own task, the steps go at the end of its checklist. A step that the checklist has already is not added again.
  - For a GitHub, ClickUp, or email item, the steps go on your linked task of the same work. When there is none, AIfred makes a new task with the item's title and links it under the item.
  - The message after the add has an Undo. It takes out the added steps that are not done, or deletes the new task.

## Calendar

The Calendar page shows your Google Calendar events and your plan as blocks, as in Google Calendar (`apps/desktop/src/Calendar.tsx`).

- "Day" shows one day. "Week" shows today and the next 6 days, because AIfred reads 7 days of events. Click a day in the week to open it.
- Each day has its own plan and its own plan chat. "Plan it for me" and "I plan it myself" work for each day.
- A plan item with no fixed time goes in the next free time, around lunch, the meetings, and the items with a fixed time.
- Drag a block to a new time. It then has a fixed time. Drag it to another day in the week view. Drag an item from the list under the calendar onto the calendar to add it. Drag a block onto that list to take it out.
- Without a mouse, select a block and use these keys:
  - Alt+Up and Alt+Down move it by 15 minutes.
  - Alt+Left and Alt+Right move it to the day before or after.
- The time button on a block with a fixed time removes the fixed time.
- Each change shows Undo.
- The plan chat can also move blocks. It sets a fixed time only when you ask for a time. When it changes the plan, the blocks that you placed keep their times. It removes a fixed time only when you ask.
- A day with more work than free time shows "Too full". The week view says which days are too full. "What can wait?" asks AIfred which items can move.
- Google Calendar does not change. The blocks are only in AIfred.

## Google (calendar and Gmail)

- AIfred connects to Google itself. Each user makes their own Google registration once:
  1. In Google Cloud Console, make a project.
  2. Turn on the Google Calendar API and the Gmail API. For Docs suggestions, also turn on the Google Docs API. For Sheets comments, also turn on the Google Sheets API.
  3. Set up Google Auth Platform. Pick the audience "Internal" for a company Google Workspace. Else pick "External" and publish the app: in "Testing", Google ends the login every 7 days. Google then shows "Google hasn't verified this app" at the login. It is your own registration: click "Advanced" and continue.
  4. Create a client of the type "Desktop app".
- Paste the client ID and the client secret on the Settings page (Connections tab), then click "Connect Google". Google's login page opens in the browser.
- AIfred keeps the registration and the login key in `google.json` in the data folder. Only you can read that file.
- AIfred reads the next 7 days of the primary calendar. It cannot change the calendar. The Calendar page shows the events, the day plan goes around busy events, and the brief and the chats see the meetings.
- One connection serves all Google features. When a new feature needs more access, Settings asks you to connect again (`crates/aifred-core/src/google.rs`). A login from before Gmail was added does not allow Gmail: connect again once.

### Gmail

AIfred reads Gmail (`crates/aifred-core/src/gmail.rs`). It cannot send or delete email. With "Use Gmail drafts" on, it can save a reply draft when you click (see "Reply drafts" below).

- To not use Gmail, untick "Use Gmail" in Settings > Google. Then the login does not ask for Gmail, AIfred removes the saved emails, and no Gmail error or "connect again" note shows. If the login already allows Gmail, connect again to remove that access at Google. Tick it again to use Gmail: connect again, and allow it.

- AIfred reads the threads of the last 14 days: at most 100 from the inbox and 50 from "Sent". It reads only the headers (sender, people, subject) and Google's short preview of each message, not the full text.
- An email from a person, in the inbox, to you, whose last message is not yours, waits for your answer. It shows in the list as an "Email" item. The brief and the day plan rank it with the other work.
- AIfred leaves out mail from machines: newsletters, notifications, and "no-reply" senders.
- AIfred leaves out Gmail's Promotions, Social, Updates, and Forums tabs.
- AIfred leaves out archived threads, and threads where you are only in Cc and never wrote.
- A tracked email closes when the person answers. AIfred reads each tracked thread again once a day. So an answer in a left-out tab closes the item within one day.
- "No answer needed" (the tick) hides an email in AIfred. Gmail does not change. The email comes back when a new message comes in the thread.
- In the chat, you can say that an email needs no answer (`hide_email`). It is the same as the tick.
- An email that you sent, with no answer yet, shows under "No answer yet to your emails" in the "Waiting for" card. "Track" puts it on the list. "Hide" removes the suggestion.
- AIfred reads again only the threads that changed since the last download. The first download can take some seconds.

### Reply drafts

AIfred can write a reply to an email that waits for your answer (`crates/aifred-core/src/drafts.rs`). You check it. Only your click saves it, as a draft in the Gmail thread. AIfred never sends.

- Turn it on once: tick "Use Gmail drafts" in Settings > Google, then click "Connect again" and allow it. It is off until you turn it on. It needs "Use Gmail": when Gmail is off, drafts are off too, and the login does not ask for the drafts access. Google has no access for "drafts only", so this access could also send email. AIfred has no code that sends.
- Open an email item. The "What to say" box has the next action from the brief. Change it or clear it. Then click "✦ Write a reply". It is one AI call (Sonnet) on your Claude login.
- The AI call gets only this email (the subject, the sender, and Google's short preview) and your note. It gets no memory facts and no other items, and it has no tools. So the text of an email cannot make it collect your other data.
- The reply shows with its address and subject. The address comes from the email (Reply-To, else From), not from the AI. Change the text, then click "Save as Gmail draft". "Copy text" copies it.
- The save fails when a new message came in the thread after AIfred wrote the reply. Then write the reply again.
- Each saved draft goes in the action log: under "Reply" in the item panel, and under "AIfred's actions" on the Settings page (Activity tab, `crates/aifred-core/src/actions.rs`).
- Undo deletes the draft in Gmail. When you changed the draft in Gmail, Undo does not delete it. When the draft is gone (you sent or deleted it), Undo deletes nothing, marks it, and tells you so. Undo works also after you turn Gmail or Gmail drafts off, until you connect Google again: a new login does not ask for drafts. A reply answers the newest message of a person: an out-of-office reply or a bounce does not count. When Gmail gives no clear answer to a save (for example a timeout), AIfred looks in the Gmail drafts of the thread. When it finds the draft with the same text, it logs it, so you do not get a second draft.

### Doc suggestions

AIfred can suggest changes to a Google Doc for an item (`crates/aifred-core/src/docs.rs`). You check each change. Only your click saves them, as suggestions in the Doc. The owner of the Doc accepts or rejects each one in Google Docs. AIfred never edits a Doc directly.

- Turn it on once: turn on the Google Docs API in your Google project. Then tick "Use Docs suggestions" in Settings > Google, click "Connect again", and allow it. It is off until you turn it on. Google has no access for "suggestions only", so this access could read and change all your Docs. AIfred has no code that changes a Doc directly.
- The item panel shows "Doc" when Docs suggestions is on, when the brief says that AIfred can draft Doc text, or when the item (or another item of the same work) has a Doc link in its description. Then AIfred fills in the link. When you ticked "Use Docs suggestions" but did not connect Google again, the "Doc" part tells you to connect again.
- Give the Doc link and a note of what to change. Click "✦ Suggest changes". AIfred reads the tab of the link (or the first tab, when the link names no tab) and makes one AI call (Sonnet) on your Claude login. The AI call gets only the Doc, the title of the item, and your note. It gets no memory facts and no other items, and it has no tools.
- Each change shows the old text (struck through) and the new text. Change the new text, or untick "Keep this change". Then click "Save as suggestions". At most 5 changes.
- AIfred finds the place of each change itself: the old text must be in the Doc once. AIfred suggests only the words that change. It leaves out a change that it cannot place, and tells you.
- The save fails when the Doc changed after AIfred read it. Then suggest the changes again.
- Each save goes in the action log: under "Doc" in the item panel, and under "AIfred's actions" on the Settings page.
- Undo deletes the suggestions that nobody accepted or rejected yet. It works also after you turn Docs suggestions off, until you connect Google again.

### Sheet comments

AIfred can suggest comments on the cells of a Google Sheet for an item (`crates/aifred-core/src/sheets.rs`). Each comment names a cell and says the change. You check each comment. Only your click saves them, as comments on the cells. The owner of the Sheet makes the changes. AIfred never changes a cell.

- Turn it on once: turn on the Google Sheets API in your Google project. Then tick "Use Sheets comments" in Settings > Google, click "Connect again", and allow it. It is off until you turn it on. Google has no access for "comments only", so this access could read and change all your Sheets. AIfred has no code that changes a cell.
- The item panel shows "Sheet" in these cases:
  - Sheets comments is on.
  - The brief says that AIfred can comment on a Sheet.
  - The item, or another item of the same work, has a Sheet link in its description.
- Give the Sheet link and a note of what to change. Click "✦ Suggest comments". AIfred reads the tab of the link (or the first tab with cells), at most the first 200 rows and the columns A to Z. It makes one AI call (Sonnet) on your Claude login. The AI call gets only the Sheet, the title of the item, and your note. It gets no memory facts and no other items, and it has no tools.
- Each comment shows its cell and the value of the cell now. Change the text, or untick the comment. Then click "Save as comments". At most 5 comments, one for each cell.
- AIfred never gives a comment to a person. A comment cannot have an email address: in Sheets, an address with "@" or "+" tells that person.
- The save fails when the tab changed after AIfred read it. Then suggest the comments again. These changes stop the save:
  - A new or deleted row or column.
  - A new text or formula in the part that AIfred read.
  - A new value in a commented cell with no formula.

  A formula that changes by itself, like =NOW(), does not stop the save.
- The save fails when a cell has an open comment with the same text already. An earlier save with no clear answer from Google can have added it. Look in the Sheet.
- Each save goes in the action log: under "Sheet" in the item panel, and under "AIfred's actions" on the Settings page.
- Sometimes Google gives no id for a comment. Then AIfred looks for the comment in the Sheet. The log title counts only the comments that Google confirmed. It names the cells that AIfred did not find. Undo cannot remove those comments, so look for them in the Sheet.
- When AIfred finds none of the comments, the log says that Google did not confirm them, and the save has no Undo. Google can have saved them. Look in the Sheet before you save again.
- Undo deletes the comments that nobody resolved or answered yet. A comment with an answer of another person stays, because a delete would also delete that answer. Undo works also after you turn Sheets comments off, until you connect Google again.

### "Waiting for" list

The "Waiting for" card on the home page lists what you wait for from other people (`crates/aifred-core/src/waiting.rs`).

- An item has what you wait for, who, and since when. Add one with "Add", with "Track" on an email, or in the chat ("I wait for the contract from Ana").
- An item from an email closes by itself when the person answers in the thread. It shows "Ana Lee answered on Thu 5 Nov" for 3 days.
- "Check on…" sets a check day. On that day, at the start of your work day, AIfred sends a notification when the item is still open. For an email item, AIfred reads Gmail first, so an answer from that morning counts. This needs no AI call. In the chat, "tell me if Ana has not answered by Friday" sets it.
- "Got it" closes an item. "Open again" opens it.
- At most 50 open items.

## Learning from your corrections

AIfred learns from what you change (`crates/aifred-core/src/learn.rs`, `worktime.rs`). All of it stays in the database.

### Corrections of the brief

- An item with a reason from the brief has two buttons under the reason:
  - "Not today": the item does not need you today.
  - "Wrong priority": the brief ranked the item too high.
- The item loses its reason at once. The brief text does not change until AIfred writes it again. Each button has an undo.
- In the chat, you can give the same correction (`correct_brief`).
- The next briefs and the plan chat get your corrections of the last 30 days.
- Three "Wrong priority" corrections of different items from one place in 60 days make one memory fact. A place is a GitHub repository, a ClickUp list, the first person of an email, or the first tag of a local task. An item with no place teaches nothing. Example: "GitHub acme/docs: these items are less important to the user than they look. Rank them lower."
- The fact is a core fact, so it goes into every chat and the brief. You can change or forget it under "What AIfred knows about you". AIfred does not learn a forgotten fact again.
- The undo of a correction also forgets the fact when fewer than three corrections are left. A fact with a text that you changed stays.
- The brief gets only your core memory facts. Chats get all facts when the memory is small, and only the core facts when it is large.

### Real work times

- "Start" on an item starts a timer. The card "Working on" shows the time, and the time guess when the item has one. You work on one item at a time: "Start" on another item stops the first one.
- "Stop" stops the timer. The item stays open. You can start it again later. All sessions count.
- In the chat, you can start and stop the timer (`start_work`, `stop_work`), and give the time guess of an item (`set_time_guess`). A guess that you give is yours: the brief and the plan do not change it.
- "Done" on the card, or the tick of the item, finishes the item. The message says the real time and the guess, for example "It took 45 m, the guess was 30 m".
- A GitHub item stays in the list until it closes on GitHub. "Done" only keeps its time.
- A session longer than 10 hours does not count. AIfred thinks that you forgot the timer.
- The brief and the plan chat get the real times of the last 20 finished items, and the middle value of real time divided by guess. They use them for new time guesses. They do not change a guess that exists.

## Reviews

Two reviews keep the list right (`crates/aifred-core/src/review.rs`). They need no AI call and send no notification.

### End-of-day check

- The "End of the day" card shows on the home page from 30 minutes before your work day ends (Settings). "End the day" under the list opens it at any time.
- It shows what you finished today, with the real time from the timer.
- It shows what is still open: the items of today's plan, and your tasks due today.
- Each open item has a tick. "Finish the day" puts the ticked items in tomorrow's plan. A task due today gets tomorrow as its due day.
- When tomorrow has no plan, these items wait in it. The app still asks how to plan the day: "Plan it for me" or "I plan it myself". Both keep the items.
- After "Finish the day", the card does not show again that day.

### Weekly review

- The "Weekly review" card shows under the brief 7 days after the last review. The first one comes on a Friday, Saturday, or Sunday. "Weekly review" under the list opens it at any time. "Later" hides it until tomorrow.
- First it asks if your goals are still right. Each goal shows how many items that served it you finished in the last 7 days. The goal of an item comes from the briefs and plans of the last 14 days.
- Then it lists the open items with no change for 3 weeks (at most 30, oldest first). A change is an edit in AIfred, GitHub, or ClickUp, work on the timer, or an earlier pick. Emails and tasks that start later are not in the list.
- For each item, pick one. Each pick has an Undo.
  - "Do": the item goes into today's plan.
  - "Plan": pick a later day. The item goes into that day's plan. Your own task also waits until that day (its start day).
  - "Give to…": type who does it. The item goes on the "Waiting for" list. Your own task closes.
  - "Drop": your own task closes. A GitHub or ClickUp item does not change there: AIfred only stops counting it in the brief and the reviews.
  - In the chat, you can give the same pick (`review_pick`).
  - A task that closes with "Give to…" or "Drop" is not finished work. No "task done" job starts, and its timer time is not kept.
  - An item that you gave away or dropped counts again when it changes after the pick. For example, you open the task again, or the GitHub item has new activity.
- The brief and the plan chat get your "Do" and "Plan" picks of the last 30 days. They also get each "Give to…" and "Drop" pick that still hides an open item.

## Check the app without a window on your screen

```bash
scripts/run-hidden.sh 300 /tmp/app.png
```

This starts the app on a hidden screen (Xvfb). It waits up to 300 seconds until the page shows, saves a picture, and stops the app. It does not change your own copy of the app:

- It uses a different web port, so it works while your copy runs.
- It does not read or write your saved window size. The setting `AIFRED_NO_WINDOW_STATE=1` turns that off.
- It uses a temporary copy of your data folder.
- It uses its own message bus (`dbus-run-session`), so its notifications do not show on your desktop.

To let AIfred's jobs run in the hidden copy, keep it running longer, and save its database to read the runs:

```bash
AIFRED_HOLD_SECONDS=150 AIFRED_KEEP_DB=/tmp/after.db scripts/run-hidden.sh 300 /tmp/app.png
```
