# Plan: AIfred's own status for every item

Date: 2026-11-10.

## Goal

Every item in AIfred gets one status from a small, fixed list. The status means the same thing for local tasks, ClickUp tasks, and GitHub items. A status name in ClickUp or GitHub can mean something else in each list, so AIfred must not show the source name as its own status.

Example: in the Rocks list, "Delayed" means "we did not do it this quarter, and we made a copy in the next quarter". It is not "done", "to do", or "in progress".

## The statuses

| Status | Open or closed | Meaning |
| --- | --- | --- |
| To do | open | Nobody started the work. |
| In progress | open | The user started the work. |
| Waiting | open | The work waits for another item or another person. |
| Done | closed | The work is finished. |
| Moved | closed | Nobody worked on it in its time. Usually a copy of it continues somewhere else (for example, in the next quarter). |
| Cancelled | closed | The user decided not to do the work at all. |

"Moved" and "Cancelled" are not finished work. AIfred must never count them as done.

## What exists today

- ClickUp: AIfred reads only the group of a status, not its name. A task in a "done" or "closed" status is dropped from the list (`crates/aifred-core/src/clickup.rs:215`). A task in any other status is open. AIfred gives the status name to the AI with no meaning (`crates/aifred-core/src/overview.rs:219`). The brief instructions say nothing about ClickUp statuses (`crates/aifred-core/src/brief.rs:59`).
- ClickUp sends a type with each status: `open` (the first status of a list), `custom` (the statuses in the middle), `done`, and `closed` (test data in `crates/aifred-core/src/clickup.rs:775-820`). The Rocks list also sends `unstarted` (see Decisions). AIfred saves only the name and the color, not the type (`crates/aifred-core/src/clickup.rs:238`).
- GitHub: AIfred downloads only open items (`crates/aifred-core/src/github.rs:129`). An issue that is closed as "completed" and one that is closed as "not planned" both just leave the list.
- Local tasks: only done or not done (`crates/aifred-core/src/db.rs:58`).
- An item that leaves the download stays in the database with `in_list = 0` (`crates/aifred-core/src/db.rs:75`). AIfred does not know why it left.
- The work timer: the user can start and stop work on any item, also ClickUp and GitHub items (`crates/aifred-core/src/worktime.rs:116`). When the timer stops, nothing marks the item as started.
- The weekly review already has two picks that are statuses in all but name (`crates/aifred-core/src/review.rs:10-26`):
  - "drop" is Cancelled. A dropped local task closes, but it is not finished work.
  - "delegate" is Waiting. The item goes on the "waiting for" list (`crates/aifred-core/src/waiting.rs`).
- Blockers: an item can wait for other items (`crates/aifred-core/src/blockers.rs`). This is Waiting.

## Keep these

- [x] Use one mechanism. The review picks, the "waiting for" list, blockers, and the work timer feed the new status. Do not build a second copy of them.
- [x] Every status is open or closed. The check for open items (`crates/aifred-core/src/items.rs:35`) must use the status, not only `done` and `in_list`. Reason: a Rocks task in "delayed" is still open in ClickUp, so it stays in the download (`in_list = 1`). In AIfred it is Moved, so it is closed.
- [x] Keep the status that the user sets in its own table. A download replaces the saved item data, so a status in that data is lost at the next refresh.
- [x] AIfred shows its guess, and the user changes it. AIfred does not ask a question first. Every change has an undo.
- [x] The status is only in AIfred. AIfred does not change ClickUp or GitHub when the user sets a status. Only "mark done" writes to ClickUp, as today (`crates/aifred-core/src/clickup.rs:489`).

## Decisions

1. The status is only in AIfred. Only "mark done" changes ClickUp (user, 2026-11-10).
2. The Rocks list. The user gave "🎯 Rocks (Q4 2026)" in the "Company Wide" space as the example. Its statuses, from ClickUp on 2026-11-10:

   | ClickUp status | ClickUp type | AIfred today | AIfred status in this plan |
   | --- | --- | --- | --- |
   | planned | `open` | open | To do |
   | delayed | `unstarted` | open, shown as active work | Moved (rule) |
   | in progress | `custom` | open | In progress |
   | partially complete | `done` | dropped, same as done | Done, shown as "partly done" (decision 3) |
   | complete | `closed` | dropped, same as done | Done |

   What this shows:
   - "delayed" is an open status in ClickUp. So today AIfred ranks delayed rocks as open work. A rule fixes this (step 2). Step 3 is not needed for it.
   - ClickUp has an `unstarted` type. AIfred's code does not know it (no match for "unstarted" in the code). It reads it as open.
   - The list has the quarter in its name. Each quarter has a new list with a new ID. So a rule for one list ID stops working at the next quarter (step 2 fixes this).

3. "partially complete" in Rocks is Done (user, 2026-11-10). The team worked on the task but did not finish it. Then the team either continues it in a copy next quarter, or accepts it as partly done. In both cases the work happened. So:
   - It is Done, not Moved. Moved means "nobody worked on it", and AIfred keeps no work time for Moved items (step 5). That is wrong for real work.
   - AIfred keeps its work time, like any Done item.
   - The app and the AI show "partly done" next to Done, from the ClickUp status name. AIfred adds no seventh status for it.
   - If a copy continues next quarter, the copy is a new open item. The link between the two is in "Later".

## Which status wins

When two sources disagree, AIfred uses the first one in this list:

1. The status that the user set in AIfred.
2. The reason why the item closed in ClickUp or GitHub (step 3).
3. A rule for the ClickUp status name (step 2).
4. Waiting: the item has an open blocker, or it is on the "waiting for" list.
5. In progress: the user started the work timer on the item. The item stays In progress after the timer stops, until it closes.
6. The default for the ClickUp status group, or To do.

## Checklist

### 1. Status storage

Why: the status must live outside the downloaded data, and it must cover all three sources with one key (the item key, for example `clickup:86abc`).

- [x] Add a table for the status that the user sets: item key, status, time. No row: AIfred works out the status (see "Which status wins").
- [x] Add a column to local tasks for how a task closed: done, moved, or cancelled. A done task with no value is "done", so old tasks keep their meaning.
- [x] Add one function that returns the status of any item key. All other code reads the status through it.
- [x] A weekly review "drop" sets Cancelled (`crates/aifred-core/src/review.rs:338`). Its undo removes it.

### 2. ClickUp status rules

Why: each ClickUp list can have its own status names. Many lists share the same names, so a rule for a name fixes all of them at once. Only real exceptions need a rule for one list.

- [x] Save the status type, the list ID, the folder ID, and the space ID with each ClickUp item (`crates/aifred-core/src/clickup.rs:238`).
- [x] Default for each status type:
  - `open` and `unstarted`: To do. The Rocks list sends `unstarted` (see Decisions).
  - `custom` (and `active`, if ClickUp sends it): In progress. Assumption: ClickUp can send `active`, because the ClickUp search tool has this filter value. I did not see it in a real list.
  - `done` and `closed`: Done.
- [x] Add rules for one status name. A rule has a scope. The narrowest scope wins:
  - one list,
  - one folder,
  - one space,
  - everywhere.
- [x] A rule for a folder or a space also covers new lists in it. This is needed for lists that change each quarter. Example: "delayed" in the "Company Wide" space is Moved, so the Q1 2027 Rocks list needs no new rule.
- [x] Example rule for everywhere: "blocked" is Waiting.
- [x] AIfred guesses a rule for each new status name from the name (for example, "on hold" is Waiting, "won't do" is Cancelled). The guess is used at once and marked as a guess.
- [x] Add a Settings section that lists each ClickUp status name that AIfred saw, with its list and its AIfred status. The user changes a row with one click. New guesses show first.

### 3. Find out why an item closed

Why: ClickUp done and closed tasks never reach AIfred (`include_closed=false`, `crates/aifred-core/src/clickup.rs:667`). GitHub items are downloaded only when open. So today AIfred cannot tell Done from Moved or Cancelled.

- [x] When a ClickUp item leaves the list, read that one task again. Use its status and the rules of step 2.
- [x] When a GitHub item leaves the list, read why it closed. `gh search` cannot send this (its JSON fields have no close reason). Use the GraphQL query that AIfred already sends (`crates/aifred-core/src/github.rs:191`) and add:
  - issue `stateReason`: `COMPLETED` is Done, `NOT_PLANNED` is Cancelled, `DUPLICATE` is Cancelled.
  - pull request `merged`: merged is Done, closed and not merged is Cancelled.
- [x] Read only items that AIfred already knows, and read each one only once. Save the result with the item.
- [x] If the read fails, the item is Done, as today. Try again at the next download.

### 4. In progress and Waiting

Why: the user wants to see what they started. The data exists, but no status uses it.

- [x] An item with at least one work session, and not closed, is In progress (`crates/aifred-core/src/worktime.rs`).
- [x] An item with an open blocker, or on the "waiting for" list, is Waiting (`crates/aifred-core/src/blockers.rs`, `crates/aifred-core/src/waiting.rs`).
- [x] This also works for ClickUp and GitHub items. Example: the user starts the timer on a ClickUp task that is "to do" in ClickUp. AIfred shows In progress. ClickUp does not change.

### 5. Moved and Cancelled are not finished work

Why: these items must not make the user look more productive than they were, and they must not start "task done" jobs. `crates/aifred-core/src/review.rs:486` already has this rule for dropped tasks.

- [x] AIfred keeps no real work time for them (`crates/aifred-core/src/worktime.rs:153`).
- [x] No "task done" jobs start for them.
- [x] They are not in the end-of-day "finished today" list, and not in the goal counts of the weekly review.

### 6. The AI

Why: the brief and the plan chat must use AIfred's status. The source name alone can mislead them.

- [x] Give the AIfred status to the AI in the overview. Keep the source status name next to it as extra detail (`crates/aifred-core/src/overview.rs:219`).
- [x] Add one line about the statuses to the brief instructions (`crates/aifred-core/src/brief.rs:59`). Moved and Cancelled items are never ranked.
- [x] The chat can set a status on an item, with an undo, like "mark done" today.

### 7. The app

- [x] Home page: show the AIfred status. Show the ClickUp status name only as a tooltip (`apps/desktop/src/Home.tsx:59`).
- [x] Item detail: change the done button into a status picker with the six statuses (`apps/desktop/src/Detail.tsx:261`).
- [x] Tasks page: the "Done" group shows Done, Moved, and Cancelled items, each with its label (`apps/desktop/src/items.ts:358`).
- [x] Add the status to the item type in the app (`apps/desktop/src/items.ts:26`).

### 8. Tests

- [ ] Each rule in "Which status wins", with one test for each pair that can disagree. Done for some pairs only: pick over rule, rule over guess, guess over timer, pick over timer.
- [x] A ClickUp task in "delayed" gets Moved with a space rule, also in a new list of the same space. With no rule, it gets To do (`unstarted`).
- [x] A Moved task that is still open in ClickUp is not open in AIfred. The brief does not rank it.
- [x] A GitHub issue closed as "not planned" is Cancelled. A closed pull request that is not merged is Cancelled.
- [x] A Moved or Cancelled item keeps no work time and starts no "task done" job.
- [x] A download keeps the status that the user set.

## Result

Built on 2026-11-10. The code is in `crates/aifred-core/src/status.rs`, with the app parts in `apps/desktop/src/Status.tsx`.

What differs from the plan:

- AIfred's guess from a status name uses a word list, not an AI call. It costs nothing and is the same each time. It matches whole words only. A guess of Moved or Cancelled closes an item with no action of the user, so those lists have only words with one meaning: "delayed" and "carried over" are Moved, "won't do" and "not planned" are Cancelled. "on hold" and "blocked" are Waiting. Words that also fit open work give no guess, for example "postponed", "moved to QA", and "rejected".
- "partially complete" is Done, as decided. AIfred keeps the ClickUp name ("closed as \"partially complete\""), but it does not add the text "partly done". Such a task leaves the ClickUp list, so the app does not show it in a row.
- A weekly review "drop" of a GitHub or ClickUp item is now Cancelled, so the item is closed in AIfred. Before, it stayed open and only the brief hid it. It still opens again when the item changes after the drop.
- A local task that the user gives away in the weekly review keeps "Done" as its closed status. The "waiting for" list has it, and the reviews still do not count it as finished.
- The ClickUp list downloads again once after the update (saved list version 3), so the saved tasks get their status group, list, folder, and space.

Checked:

- Rust tests: 434 in the core, including 10 new tests (9 for statuses, 1 for the GitHub close reason).
- App tests: 248, including 7 new tests for the status menu, the rows, the Settings rules, and no ClickUp change for an item that is closed only in AIfred.
- The database upgrade on a copy of the real data works. With the final word list, 1 ClickUp task is Moved: "Automating tests with Playwright" in "Rocks (Q1 2026)" ("delayed").
- One read-only ClickUp download with the real token: all 11 tasks have their list, space, space name, and status group.
- The app starts on a hidden screen with the upgraded data.

Not checked: the real ClickUp and GitHub calls that read why an item closed. They run after each download, and a failed read only counts a try.

## Later

- Link a Moved item to its copy (for example, the same rock in the next quarter). AIfred can suggest the link from the title and the user can change it.
- If the user wants it (open question 1): write the status back to ClickUp or GitHub.

## Changes after the first review (2026-11-10)

The user found the first Settings screen confusing. These changes replace parts of the plan above.

- [x] "Moved" is "Unfinished" now. Meaning: the team worked on it, but did not finish it in its time. Database version 36 renames the old values.
- [x] An Unfinished item can link to the item that continues it (table `status_next`). With no link, the user accepts it as it is. The item panel shows "Continued in" and "Continues". The chat sets it with `set_item_status` (`continued_in`). This replaces the "Later" item about links to a copy.
- [x] AIfred does not guess from the status name any more. Only the ClickUp group counts: Not started (`open`, `unstarted`) is To do, Active (`custom`) is In progress, Done and Closed are Done. The user adds overrides for other meanings. With no override, "delayed" in Rocks is To do.
- [x] The Settings page has 5 tabs: General, Connections, ClickUp statuses, Activity, About.
- [x] The ClickUp statuses tab follows the "group defaults plus overrides" pattern (Atlassian status categories as the fallback, Mosaic's mapping editor with a default for unmapped statuses). It shows a fixed table of the 4 groups, and a table of overrides: Space, List (or "Any list in the space"), ClickUp status, AIfred status.
- [x] ClickUp keeps statuses on spaces, folders, and lists (ClickUp Help Center). Checked with the real data: the "Company Wide" space has Open, in progress, review, and Closed. The Rocks lists have their own statuses (planned, delayed, in progress, partially complete, complete). So "Any list in the space" offers the statuses of the space and of each list in it.
- [x] The app makes only "list" and "space" overrides. "folder" and "all" rules still work in the backend.

## Changes after the code review (2026-11-10)

A review of the uncommitted work found 10 problems. All are fixed, with a test for each in `crates/aifred-core/src/status.rs` unless said otherwise.

- [x] Unfinished or Cancelled on a GitHub or ClickUp item now forgets the real time of an earlier finish, like a local task does. The undo of a status change puts the real time from before back (`status::Before.work`). Test: `cancelled_forgets_the_real_time_and_the_undo_puts_it_back`.
- [x] Done on a GitHub item ends its work and keeps the real time in the backend, so the chat and the item panel do the same. The "task done" job stays for local tasks only, as before. Test: `done_on_a_github_item_ends_the_work_and_keeps_the_time`.
- [x] "Mark done" in ClickUp is AIfred's own close, so it saves the reason (Done) and when the task left the list (`work_item.closed_at`, database version 37). A Cancelled or Unfinished pick from before that time, also a weekly review drop, does not win over a close that AIfred knows. The pick stays, so the undo of "mark done" brings it back when the task is in the list again. Tests: `a_close_in_the_source_wins_over_an_older_pick`, `a_dropped_item_opens_again`.
- [x] Done in the status menu of a ClickUp task that is closed only in AIfred (for example Unfinished by a rule) closes the task in ClickUp. The tick and the timer still never change ClickUp. Test in `apps/desktop/src/App.test.tsx`: "Done in the status menu of a ClickUp task that is Unfinished in AIfred closes it in ClickUp".
- [x] A pick that gives an item the status it has anyway (from its rule, a blocker, or the timer) is not saved, so the undo of a chat status change does not pin the status. The chat answer has `before_by`. The "Open again" tick takes the pick away (`status::open_again`), and sets To do only when a rule still closes the item. Tests: `a_pick_that_the_item_already_has_is_not_saved`, and "the tick of a ClickUp task that you closed only takes your pick away" in the app.
- [x] An item that left the list while still open in its source (an issue that is not yours any more, a ClickUp task with another assignee) is saved as "still open", not Done. It reads To do, and AIfred reads it again once a day for 30 days after it left, so a later close as "not planned" is found. This is at most one extra API call per such item per day, within the 20 reads per download. Test: `an_item_that_is_still_open_in_its_source_is_read_again`.
- [x] A deleted task takes its pick and its "continued in" links with it. Test: `a_deleted_task_leaves_no_status_rows`.
- [x] The status of one item comes from the rows of that item (`status::of_in`), not from a scan of every item. The refresh after a pick or a rule change covers only the items in the list, and a pick change refreshes one row. Test: `the_status_of_one_item_is_the_same_as_in_the_status_of_all`.
- [x] The closed statuses are in one SQL constant (`status::CLOSED_SQL`) and one app function (`api.isClosed`), and the ClickUp group defaults are in `api.groupDefault`. Test: `closed_sql_agrees_with_is_closed`.

Not checked: the real ClickUp and GitHub calls that read why an item closed, as before.
