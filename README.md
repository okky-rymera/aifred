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
- The box on the home page has two modes. Press Tab in the box, or click the chip under it, to change the mode. Shift+Tab still moves the focus out of the box. The mode stays when you go to another page.
  - "Ask AIfred" (the start mode): the message goes to AIfred. AIfred decides what to do:
    - a short to-do line, for example "pay rent friday high #home", becomes a task with its due day, priority, and tags,
    - a question gets an answer,
    - "remind me…" or "check … later" becomes one of AIfred's jobs,
    - only "plan my day" works with no AI decision: it opens the Day plan.
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

AIfred keeps its own list of jobs, apart from your to-do list. It does them by itself while it runs, also when the window is closed to the tray.

- Ask in the chat. Examples:
  - "Remind me every Monday at 9 to send my timesheet."
  - "Remind me on the 3rd and 17th to pay the invoices."
  - "In 2 hours, check if PR #123 is merged and tell me."
- AIfred says the time back with the next run dates. Check them.
- A job is one of these:
  - a reminder: a desktop notification, with no AI call,
  - an AI job: AIfred runs at that time, looks things up, and sends a notification with the result. If it has nothing to tell, it sends nothing.
- An AI job can only read. It cannot change tasks, the plan, memory, GitHub, or ClickUp. It cannot make new jobs.
- Each AI job run is one more AI call on your Claude login (Sonnet).
- A job runs once, at a time and repeat (for example "every other Thursday", "the last Friday of the month"), or after an event. The repeat rule is an RRULE (RFC 5545). The time zone is the zone of your computer when you make the job.
- AIfred has one job from the start: "Update the brief when you finish a task". It waits 90 seconds for more finished tasks, so many tasks give one update (at most 10 minutes after the first one). It only updates a brief that already exists for today.
- "AIfred's jobs" (the clock in the left bar) shows the jobs, the next run, and the latest runs. You can pause, start, and delete a job there. To change a job, ask in the chat.
- At most 20 jobs that are active or paused. A job can repeat at most once an hour.
- If AIfred was not running at a job time (closed, or the computer was asleep):
  - a one-time job runs late, and the notification says the planned time,
  - a repeating job runs one time for the newest missed time, if it is at most 12 hours old. Older times are skipped.
- A job that fails tries again after 1, 2, 4, and 8 minutes. After 5 failures in a row it turns off, and AIfred shows a notification.
- A run that was busy when AIfred stopped does not run again.
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

## One piece of work in many places

A ClickUp task, its GitHub pull request, and an email about it can be one piece of work. Link them, and the home list shows one row for them (`crates/aifred-core/src/links.rs`).

- The row is the main item. The other items show under it after "Also in". Click one to open it.
- When the main item is finished or closed, the next open item of the group becomes the row.
- The row shows the earliest due day and the highest priority of its open items. So a GitHub row with a late ClickUp task in it shows under "Needs you now".
- The tick, "Start", and "+ Plan" of the row act on the main item.
- To link items yourself, open an item and click "+ Link another item" under "Same work". Type words of the other item's title.
- "Unlink" under "Same work" shows an item as its own row again. When you unlink the main item, the next item becomes the main item.
- Each link and unlink has an Undo.
- One piece of work can have at most 10 items.

AIfred suggests links in the "Same work?" card on the home page. A suggestion needs no AI call. It comes from these rules, only for items from different sources:

- One item names the other: a GitHub link, "acme/shop#7", a ClickUp link, a ClickUp id like "DEV-42", or "CU-86a1b2".
- The titles have mostly the same words: at least 2 words in common, and at least 60% of all their words. Short and common words ("fix", "the", "add") do not count.

"Link" accepts a suggestion. "Not the same" rejects it, and AIfred does not suggest that pair again. The main item of a suggestion is your own task first, then the ClickUp task, then the GitHub item, then the email.

The brief and the chat get the linked items as one piece of work (`same_work` in the overview).

## Suggested work

- For each item that the brief names, the brief also gives the next action, for example "Next: Reply to Ana with the price." It shows under the reason in the home list and in the item panel. It costs no extra AI call: the brief call writes it.
- A label says who does the next action:
  - "You do this": only you can do it, for example a decision, a call, code, or a review.
  - "AIfred can write a reply to (name)": the item is an email from that person. Use "Write a reply" in the item panel (see "Reply drafts" below). An email to another person, you write yourself.
  - "AIfred can draft the email later" (or the Sheet change): AIfred cannot do this yet. When it can, it makes a draft for you to check. It sends nothing by itself.
  - "AIfred can draft the Doc text later": use "Suggest changes" under "Doc" in the item panel (see "Doc suggestions" below).
- "✦ Suggest steps" in the item panel splits an item into 2 to 8 steps. It is one AI call (Sonnet) on your Claude login. It saves nothing until you pick:
  - Clear the box of a step that you do not want, then click "Add N steps".
  - For your own task, the steps go at the end of its checklist. A step that the checklist has already is not added again.
  - For a GitHub, ClickUp, or email item, the steps go on your linked task of the same work. When there is none, AIfred makes a new task with the item's title and links it under the item.
  - The message after the add has an Undo. It takes out the added steps that are not done, or deletes the new task.

## Google (calendar and Gmail)

- AIfred connects to Google itself. Each user makes their own Google registration once:
  1. In Google Cloud Console, make a project.
  2. Turn on the Google Calendar API and the Gmail API. For Docs suggestions, also turn on the Google Docs API.
  3. Set up Google Auth Platform. Pick the audience "Internal" for a company Google Workspace. Else pick "External" and publish the app: in "Testing", Google ends the login every 7 days. Google then shows "Google hasn't verified this app" at the login. It is your own registration: click "Advanced" and continue.
  4. Create a client of the type "Desktop app".
- Paste the client ID and the client secret on the Settings page, then click "Connect Google". Google's login page opens in the browser.
- AIfred keeps the registration and the login key in `google.json` in the data folder. Only you can read that file.
- AIfred reads the next 7 days of the primary calendar. It cannot change the calendar. The day plan goes around busy events, and the brief and the chats see the meetings.
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
- An email that you sent, with no answer yet, shows under "No answer yet to your emails" in the "Waiting for" card. "Track" puts it on the list. "Hide" removes the suggestion.
- AIfred reads again only the threads that changed since the last download. The first download can take some seconds.

### Reply drafts

AIfred can write a reply to an email that waits for your answer (`crates/aifred-core/src/drafts.rs`). You check it. Only your click saves it, as a draft in the Gmail thread. AIfred never sends.

- Turn it on once: tick "Use Gmail drafts" in Settings > Google, then click "Connect again" and allow it. It is off until you turn it on. It needs "Use Gmail": when Gmail is off, drafts are off too, and the login does not ask for the drafts access. Google has no access for "drafts only", so this access could also send email. AIfred has no code that sends.
- Open an email item. The "What to say" box has the next action from the brief. Change it or clear it. Then click "✦ Write a reply". It is one AI call (Sonnet) on your Claude login.
- The AI call gets only this email (the subject, the sender, and Google's short preview) and your note. It gets no memory facts and no other items, and it has no tools. So the text of an email cannot make it collect your other data.
- The reply shows with its address and subject. The address comes from the email (Reply-To, else From), not from the AI. Change the text, then click "Save as Gmail draft". "Copy text" copies it.
- The save fails when a new message came in the thread after AIfred wrote the reply. Then write the reply again.
- Each saved draft goes in the action log: under "Reply" in the item panel, and under "AIfred's actions" on the Settings page (`crates/aifred-core/src/actions.rs`).
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
- The next briefs and the plan chat get your corrections of the last 30 days.
- Three "Wrong priority" corrections of different items from one place in 60 days make one memory fact. A place is a GitHub repository, a ClickUp list, the first person of an email, or the first tag of a local task. An item with no place teaches nothing. Example: "GitHub acme/docs: these items are less important to the user than they look. Rank them lower."
- The fact is a core fact, so it goes into every chat and the brief. You can change or forget it under "What AIfred knows about you". AIfred does not learn a forgotten fact again.
- The undo of a correction also forgets the fact when fewer than three corrections are left. A fact with a text that you changed stays.
- The brief gets only your core memory facts. Chats get all facts when the memory is small, and only the core facts when it is large.

### Real work times

- "Start" on an item starts a timer. The card "Working on" shows the time, and the time guess when the item has one. You work on one item at a time: "Start" on another item stops the first one.
- "Stop" stops the timer. The item stays open. You can start it again later. All sessions count.
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
