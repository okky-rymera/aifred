# Plan: tasks page, missions, task context, and calendar

Date: 2026-11-07. Status: agreed. The user made the decisions on 2026-11-07 (see "Decisions").

## Goal

The home page lists all open items now. The user must read the full list each day to find the next work. AIfred must do that work:

- read all raw items,
- join the items that have one goal,
- rank them,
- show only a short list of work on the home page.

## Words in this plan

- **Raw item**: one item from one source. A local task, a GitHub item, a ClickUp task, an email, or an AIfred job.
- **Mission**: the next small piece of work to do. Usually one subtask or one item, sometimes two small items. Never a full big task. Only missions show on the home page.
- **Subtask**: an item under a parent item, in AIfred only. Any item can be a parent or a subtask, with many levels.
- **Doer**: who does the work. "You" or "AIfred".
- **Task context**: hidden notes about one item. AIfred writes them from the user's answers. The AI reads them. The page does not show them by default.

## What exists now

These parts already do some of the work. The plan uses them. It does not make copies.

- Local tasks: the `task` table (`crates/aifred-core/src/tasks.rs`). Due day, start day, priority, tags, checklist, files.
- AIfred's jobs: the `job` table (`crates/aifred-core/src/jobs.rs`). A trigger (once, repeat with RRULE, or event), a status, a run history (`job_run`), failure retries.
- One piece of work in many places: the `item_link` table (`crates/aifred-core/src/links.rs`). It groups raw items under one main item.
- The brief: the `brief` table. It keeps a reason (`why`), a next action (`next`), and a draft kind for each item key.
- Goals, time guesses (`estimate`), real work times, corrections, and review picks.
- The overview (`crates/aifred-core/src/overview.rs`). It gives all of the above to the AI in one call.
- The day plan: the `day_plan` table and `apps/desktop/src/Plan.tsx`. The timeline already puts plan items around busy calendar events.
- Google Calendar: read only. The login asks only for `calendar.readonly` (`crates/aifred-core/src/google.rs:31`).

Note: the `task` table has `added_by = 'aifred'`. That means "AIfred added this task for you". It does not mean "AIfred does this work". The new doer field is a different field.

## Part 1: Tasks page

- [x] Add a "Tasks" page to the left bar (`apps/desktop/src/App.tsx`, the `Page` type).
- [x] Show the full raw list on this page. The home page keeps its list until Part 3 adds the missions. Then the home page shows only the missions.
- [x] Add a filter: "Mine", "AIfred's", "Both". The start value is "Mine".
- [x] Keep the current groups, the row actions, and the "Later" list.
- [x] Hide the system job "Update the brief when you finish a task" (action `brief`). Show it only on the Settings page.

## Part 2: One task type for the user and for AIfred

The user's view: a job is a task with a trigger. The other fields are the same. AIfred must read both kinds together to plan well.

Decision A: one table.

Do this in two steps. About 40 SQL reads of the `task` table are in 12 files. If the job rows move first, a missed read shows a reminder as an open task of the user.

1. Step 1 (done): one list in the code. The overview gives the AI the user's tasks (`doer: "you"`) and AIfred's active jobs (`doer: "aifred"`, with no item key) in one `tasks` list. The Tasks page shows the two lists with one filter.
2. Step 2: the data move. Do these before the move:
   - Mark each read of `task` as "user tasks only" or "both". The default read must return only `doer = 'you'`.
   - A repeating user task makes a new task row at each time. A reopened row loses the earlier `done_at`, and the day review and the work times need it.
   - Finishing an AIfred task must not send the `task_done` event. Else the brief job starts itself again.
   - SQLite cannot change a foreign key. Make `job_run` again with `REFERENCES task(id)`, as in SCHEMA_6. Keep `tz` and `cursor`.
   - The `task` triggers fill the word search, the meaning search, and the summaries. Keep the AI instructions of a job out of `notes`, or the migration sends each job to the summary model.
   - Add a test that opens a version 18 database with a job, a run, and the brief job.

- [x] Add `doer`, `trigger`, `status`, `next_run`, and `failures` to the `task` table (db.rs SCHEMA_19). Also `action`, `message`, `tz`, and `cursor`.
- [x] Move the rows of the `job` table into the `task` table. Keep `job_run` and `job_event`, and point them at the task id.
- [x] The scheduler reads the tasks with `doer = 'aifred'` and the status "active". The user's task reads filter on `doer = 'you'`. A test checks each read.
- [x] A task with `doer = 'aifred'` runs as a reminder or an AI job, as now.
- [x] A repeating user task makes a new task at each time. For example, "pay rent on the 1st". The user picked this over one task that opens again. The rule is an AIfred task (`doer = 'aifred'`, action `task`), so it is not in the user's list. Each new task has `doer = 'you'` and the local day of the time as its due day. A missed time has no 12-hour limit.
- [x] Give the overview one `tasks` list with a `doer` field, not two lists.
- [x] Let the AI give a task to AIfred, for example "check PR #123 at 15:00". The chat tool `schedule_job` already does this with action "agent". Unattended AIfred work stays read only (no change to this rule).

## Part 3: Missions on the home page

- [x] Add a `mission` table. Each row has: an id, a title, the raw item keys, a goal id, a progress value, a checklist, a next action, a doer, and a rank.
- [x] The brief call makes and updates the missions. It uses the overview, `item_link`, goals, and task context.
- [x] The brief keeps a mission id when the same raw items stay in it. So the checklist and the progress do not reset each day.
- [x] A raw item is in at most one mission. The AI can suggest a new link. The user accepts it, as with "Same work?" now.
- [x] Progress: done checklist steps divided by all steps, and closed raw items divided by all raw items. AIfred shows how it calculated the value.
- [x] The home page shows only the top missions for today (at most 7). "See all tasks" opens the Tasks page.
- [x] A mission opens a panel with its raw items, its checklist, and "Start", "Done", "Not today", and "Wrong priority".
- [x] Ticking a checklist step does not change the raw items. Finishing a mission asks which raw items to close.

Decision C: the home page shows the other raw items as one closed line, "Other open items (N)". A click opens them, or opens the Tasks page. Exception (2026-11-08): the "Needs you now" items that are in no mission always show, under the missions.

Decision D (2026-11-08): AIfred groups items into a mission at once. It does not wait for the user's OK. Replaced by Part 3b on the same day: the "Same work?" card goes away, and nothing blocks the brief for good.

How part 3 works now (2026-11-08):

- Tables: `mission` and `mission_item` (SCHEMA_20). The key of `mission_item` is the item key, so an item is in at most one mission.
- The mission table has no reason, next action, or goal. A mission shows those of its first item from the brief. So the brief does not write them twice.
- The progress is not saved. The app calculates it when it reads the mission.
- The mission panel opens inside the mission card on the home page, not in the side panel.
- "Finish mission" closes the picked items first. If one does not close, the mission stays open. A done mission keeps its items and checklist, so its Undo opens it again.
- The brief prompt has no task context yet. Part 4 adds it.

## Part 3b: Subtasks, and the AI sorts the tasks (2026-11-08)

Why: the "Same work?" card asked about pairs one at a time. The user found this annoying, and a "Not the same" answer could never be undone. Research agrees: let the AI act, show what it did, and make each change easy to undo (Microsoft guidelines for human-AI interaction, 8 and 9).

Rules:

- The AI acts without asking. The brief tells the user what changed, in plain text. Example: "I made X a subtask of Y, because ...". For many changes: "I reorganized several tasks:" and one bullet for each change, with the reason.
- The user can undo each change, or move an item by hand or through chat. The AI never moves an item that the user placed.
- All parent links are in AIfred only. ClickUp and GitHub do not change.

Steps:

- [x] Step 1, subtasks: each item can have a parent item, with many levels. This replaces `item_link`. The Tasks page and the item panel show the tree. The user can move an item under another item, or out, with Undo.
- [x] Step 2, the AI sorts: when new items come in (ClickUp, GitHub, Gmail, or a new task), one small AI call puts them in the tree. Clues: one item names the other, the titles match, a ClickUp task is usually the parent of its GitHub issue, and an issue is the parent of its pull request. Each move keeps its reason. The next brief reports the moves since the last brief.
- [x] Step 3, smaller missions: the brief picks the next small subtask or item for each mission. The parent shows as context ("part of DEV-123"). The user can remove an item from a mission or add an item, with Undo. A removed item comes back only when the user adds it.
- [x] Step 4 (done first): remove the "Same work?" card and its code. The `link_no` table goes (SCHEMA_21). The brief gets no `not_same_work` list.

How step 1 works (2026-11-08):

- `item_link` is the tree: each row puts `key` under its parent `main`. SCHEMA_22 adds `placed_by` ("you" or "aifred") and `why`.
- A move takes the subtasks of the item with it. "Move out" too. When an item is gone, its subtasks move up to its parent.
- A mission keeps only the items that the brief named. It does not pull in the full tree.
- The home list shows one row for each tree, with the subtasks under it. The item panel shows the tree with each level indented. A tree view on the Tasks page is not done yet.

How step 2 works (2026-11-08):

- `sort.rs` finds the open items with no row in `item_sort` (SCHEMA_23). One AI call (sonnet) gets up to 20 new items, the other open items with their parents, and the clues. The call answers with moves: subtask, parent, reason.
- The app checks each move again before it saves it: a new item is in it, both items are open, the subtask is not placed by the user, and no loop or full tree. Then the new items are marked as seen.
- "Placed by the user": `item_sort.fixed`. A move by the user, "Move out", a chat move (`move_item`), and an Undo of an AI move set it. An Undo of the user's own move puts the marks back as they were (`links::restore`). "Suggest steps" marks only its new task, not the item. The links from before SCHEMA_23 count as placed by the user.
- `link_move` logs each move of the AI with the old parent. The brief shows the moves after the brief of the day before that are still in place. Undo puts the item back under its old parent, or at the top.
- The app writes the brief note from the log, not the AI.

How step 3 works (2026-11-08):

- The overview has no `same_work` list now. Each subtask has a `parent`: the nearest open item above it.
- The brief prompt asks for the next small subtask or item as a mission, not a full tree. A parent and its subtasks do not go in one mission.
- The mission card shows the parent of its first item: "Part of X". The app finds it from the tree. The AI does not write it.
- SCHEMA_24: `mission_item.placed_by` and the `mission_out` table. An item that the user added stays in its mission. An item that the user took out never goes back in that mission by the brief. It can get a mission of its own.
- SCHEMA_25: the parent of each link that the user made is fixed too (`item_sort.fixed`). So the AI sort never moves the top item of a tree that the user built. The parents of the AI's links stay free.
- The last open item cannot go out of a mission. "Done" and "Not today" are for that.
- Each add and each take-out has an undo. The undo puts the item back in its old mission, when that mission is still open.
- "Needs you now" on the home page hides only the rows whose own item is in a mission. So an urgent subtask stays in view when its sister subtask is a mission.
- The chat can add an item to a mission or take one out (`add_to_mission`, `remove_from_mission`). The home page loads the brief again after each chat answer.

Fixes after the review (2026-11-08):

- A tree is not always one piece of work: the AI builds it. "Suggest steps" uses the item, the items above it, and its direct subtasks. The steps go on the item when it is an open task, else on an open task directly under it that the user placed, else on a new task.
- The Doc and Sheet links for a draft come from the item, else from the nearest item above it. Never from a sister item.
- On the home page, a tree row stays in "other open items" while one of its items is in no mission. The items in a mission leave the row.
- "Not today" and "Wrong priority" on a tree row correct the item whose reason the row shows. Only that item leaves the missions.
- When the brief names an item that the user added to a mission, that mission is ranked today.
- A deleted item removes only its own mission when that mission is empty. Other empty missions stay for their undo.
- The brief of a day shows the AI moves between the brief before it and the brief after it. It shows the newest 30 and says how many older moves it does not show.
- After the update to SCHEMA_23, the first sorts put the open backlog into trees one time.

Decision E: the sort runs in its own small AI call when new items come in, not in the brief call. So the tree is ready within minutes, not the next day.

## Part 4: Task context (hidden memory for each item)

- [x] Add a `task_context` table: item key or mission id, the text, the time of the last change.
- [x] AIfred finds items that it cannot rank: a short title, no description, no goal, no due day.
- [x] A home card "Help AIfred understand" asks about at most 3 items each day. For example: "What is 'Fix sync'? Is it linked to other work? Is it urgent?"
- [x] The user answers in a short chat (the item chat thread `item:<key>` already exists). AIfred writes a short summary into the task context.
- [x] The overview, the brief, and the item chat get the task context.
- [x] The item panel shows "What AIfred knows" as a closed section. The user can open it, change it, or clear it.

How part 4 works (2026-11-08):

- SCHEMA_27: the table is `item_context` (key, text, by, updated). It has item keys only, no mission ids: a mission uses the notes of its items, as it uses their reason and next action. `by` says who wrote the notes last ("you" or "aifred").
- SCHEMA_27 also adds `context_skip` (the items that the user skipped) and `brief.ask` (the items that the brief of the day asks about).
- A fixed rule picks the items, not the AI (`context::to_ask`). So the brief call does not change, and the pick costs nothing. The rule: a title of 4 words or fewer, no summary, no due day, no parent, no notes, no goal in the brief, not skipped. Only the user's tasks, GitHub items, and ClickUp tasks. The items in the brief's `why` come first. At most 3.
- `brief::get` drops an item from `ask` when it has notes now, when the user skipped or corrected it, or when it is closed.
- The question is one fixed text (`context::QUESTION`). The card does not show it in the chat. The item chat gets it in its first note when today's brief asks about the item, so the AI understands the answer.
- The chat tool `set_item_context` saves the notes ("aifred"). It is not in the read-only tools of AIfred's jobs. The notes are cut at 600 characters.
- The user's changes in the item panel save as "you". Each save and each clear gives the notes from before, and the Undo puts them back (`restore_item_context`).

## Part 5: Calendar page

- [x] Rename "Plan" to "Calendar".
- [x] Show a day view and a week view, as in Google Calendar. Show the Google events and the planned missions as blocks.
- [x] Let the user drag a block to a new time. The plan chat can also move blocks.
- [x] AIfred plans missions into free time, around the events. It warns when a day is too full.

Decision B: view-only first. Google Calendar does not change. Writing blocks into Google can come later, with a new Google access and an Undo log.

How part 5 works (2026-11-09):

- The plan still holds item keys, not mission ids. The brief, the reviews, and "Next:" on the home page use item keys. A block of an item in an open mission shows "Mission: <title>".
- Each plan item can have a fixed start time, `at`, in clock minutes after midnight. It is in the JSON of `day_plan.items`, so no new schema is necessary.
- The window places an item with a fixed time at that time. It places the other items in order, in the free time around lunch, the meetings, and the fixed items (`schedule` in `items.ts`).
- The AI sends the full plan again for each change. When it leaves out a time, the item keeps its old time (`plan::set_ai`). So a block that the user dragged does not move. To remove a fixed time, the AI sends `at: ""`. The AI can move an item only to today or one of the next 6 days (`plan::check_shown_day`), because the app shows only these days. The window always sends the times, so it can remove one.
- The AI tool `set_day_plan` takes `at` as "HH:MM", with two digits for the minutes. `day_plan` shows it, and tells the AI to say when a day is too full.
- A local task cannot go on a day before its start day. `move_plan_item` refuses the move, and the window refuses a drag or an add to such a day.
- The week view is today and the next 6 days, not Monday to Sunday. AIfred reads only 7 days of events from the start of today (`calendar::DAYS`).
- `day_plans` reads the plans of the 7 days in one call. `move_plan_item` moves an item to another day in one change, so it cannot get lost between two saves. Its Undo puts the item back at its old place and time.
- The AI moves an item to another day with the tool `move_to_day`, which calls `plan::move_item`. `set_day_plan` replaces the full plan of a day, so the AI must not use it for a move: it would delete the other items of that day.
- When a move empties a plan that a review made, the plan goes away, so the day shows "No plan" again. A "Tomorrow" click on a planned task also moves it to the plan of tomorrow.
- An all-day event is never busy time. It shows in the "All day" row, and the plan does not place items around it.
- Each day has its own plan chat (`plan:<day>`). The chat on the screen is the chat of the day on the page. Each day keeps its messages and its running answer when you change the day. Only one plan chat answers at a time.
- "Too full": the planned minutes are more than the free minutes of the work day. An item with a fixed time counts only its minutes in the work day that lunch and the meetings do not take: an item at 19:00 uses no work time, and an item on lunch does not count lunch two times. The app (`schedule` in `items.ts`) and the AI (`calendar::fixed_work`) count the same way. A work-day setting that AIfred cannot read gives 09:00 to 17:30 in each count. The day view shows a warning with "What can wait?". The week view shows it on each day and in one line above the grid.

## Decisions

- **A**: your own tasks can repeat. AIfred's jobs move into the `task` table (A1).
- **B**: the calendar is view-only in Google for now.
- **C**: other raw items show on home as one closed line.
- **Name**: "Mission".

## Order of work

1. Part 1 and Part 2. The data change first, because the other parts read it.
2. Part 3. The largest part. It changes the brief call.
3. Part 4.
4. Part 5.

## For each part

- [x] A new table continues the schema list after `SCHEMA_18` (`crates/aifred-core/src/db.rs`).
- [x] A new or renamed app command also goes into the fake backend (`apps/desktop/src/test/fakeBackend.ts`).
- [x] Update the README sections that the part changes.
