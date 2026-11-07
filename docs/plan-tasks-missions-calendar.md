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
- **Mission**: one unit of work that AIfred makes from one or more raw items. Only missions show on the home page.
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

- [ ] Add a `mission` table. Each row has: an id, a title, the raw item keys, a goal id, a progress value, a checklist, a next action, a doer, and a rank.
- [ ] The brief call makes and updates the missions. It uses the overview, `item_link`, goals, and task context.
- [ ] The brief keeps a mission id when the same raw items stay in it. So the checklist and the progress do not reset each day.
- [ ] A raw item is in at most one mission. The AI can suggest a new link. The user accepts it, as with "Same work?" now.
- [ ] Progress: done checklist steps divided by all steps, and closed raw items divided by all raw items. AIfred shows how it calculated the value.
- [ ] The home page shows only the top missions for today (at most 7). "See all tasks" opens the Tasks page.
- [ ] A mission opens a panel with its raw items, its checklist, and "Start", "Done", "Not today", and "Wrong priority".
- [ ] Ticking a checklist step does not change the raw items. Finishing a mission asks which raw items to close.

Decision C: the home page shows the other raw items as one closed line, "Other open items (N)". A click opens them, or opens the Tasks page.

## Part 4: Task context (hidden memory for each item)

- [ ] Add a `task_context` table: item key or mission id, the text, the time of the last change.
- [ ] AIfred finds items that it cannot rank: a short title, no description, no goal, no due day.
- [ ] A home card "Help AIfred understand" asks about at most 3 items each day. For example: "What is 'Fix sync'? Is it linked to other work? Is it urgent?"
- [ ] The user answers in a short chat (the item chat thread `item:<key>` already exists). AIfred writes a short summary into the task context.
- [ ] The overview, the brief, and the item chat get the task context.
- [ ] The item panel shows "What AIfred knows" as a closed section. The user can open it, change it, or clear it.

## Part 5: Calendar page

- [ ] Rename "Plan" to "Calendar".
- [ ] Show a day view and a week view, as in Google Calendar. Show the Google events and the planned missions as blocks.
- [ ] Let the user drag a block to a new time. The plan chat can also move blocks.
- [ ] AIfred plans missions into free time, around the events. It warns when a day is too full.

Decision B: view-only first. Google Calendar does not change. Writing blocks into Google can come later, with a new Google access and an Undo log.

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

- [ ] A new table continues the schema list after `SCHEMA_18` (`crates/aifred-core/src/db.rs`).
- [ ] A new or renamed app command also goes into the fake backend (`apps/desktop/src/test/fakeBackend.ts`).
- [ ] Update the README sections that the part changes.
