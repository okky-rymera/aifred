# Plan: let the AI change everything that the user can change

## Goal

The user can ask the AI in any chat to change a thing in AIfred. The AI does not say "I have no tool for it".

The trigger: in a mission chat, the user asked the AI to change the mission checklist. The AI had no tool for it. It put the steps on the tasks and told the user to finish the mission or press "Not today".

## How we found the gaps

We compared two lists:

- The actions that the app buttons can do (the Tauri commands in `apps/desktop/src-tauri/src/main.rs`).
- The AI tools (`crates/aifred-tools/src/*.rs`) and the tools that each chat may use (`tool_servers_for` in `crates/aifred-core/src/assistant.rs:847`).

## Gaps

### Missions (the cause of the screenshot)

| What the user wants | App button | AI tool now | Core function |
|---|---|---|---|
| Tick or untick a mission step | `set_mission_step` | none | `missions::set_step` exists |
| Add, change, remove, or sort mission steps | none | none | none: new work |
| Change the mission title | none | none | none: new work |
| Finish the mission | `finish_mission` | none | `missions::finish` exists |
| Open a finished mission again (undo) | `reopen_mission` | none | `missions::reopen` exists |
| "Not today" for the mission | `correct_mission` | none | `missions::correct` exists |
| Undo of "Not today" | `undo_correction` | none | exists in `learn.rs` |

Two facts about the brief (the daily AI call that makes the missions) change the design:

- The brief writes the mission title again on each run (`missions.rs:277`). An AI title change goes away at the next brief. A title change needs a "set by the user" mark that the brief obeys.
- The brief fills the checklist only when it is empty (`missions.rs:282`). A checklist change stays. No guard is necessary.

### Other areas

| Area | What is missing for the AI | App button |
|---|---|---|
| Memory | Change the text of a fact | `edit_fact` |
| Memory | Mark a fact as "core" (always known) | `set_fact_core` |
| Memory | Bring back a forgotten fact (undo) | `restore_fact` |
| Local tasks | Delete a task | `delete_task` |
| Day plan | Set the time guess of one item alone | `set_estimate` |
| Email | Hide an email ("no answer needed"), show it again | `hide_email` |
| Brief | "Not today" or "wrong priority" for one item | `correct_brief` |
| Brief | Make the brief again | `write_brief` |
| Work timer | Start, stop, finish, or unfinish work on an item | `start_work`, `stop_work`, `finish_work`, `unfinish_work` |
| Status | Add, change, or remove a ClickUp status rule | `set_status_rule`, `delete_status_rule` |
| Reviews | Pick do, plan, delegate, or drop in the week review | `review_pick` |
| ClickUp | Mark done, reopen, comment: only the question chat on the home page has these tools. The item chat and the mission chat do not. | `clickup_mark_done` |

### Already covered (no work)

Goals, the waiting list, blockers, subtask links (`move_item`), item status, item notes and questions, the day plan, jobs.

### Keep out of the AI (decided)

- Tokens, Google connect, and disconnect.
- App settings, the chat model, and app updates.
- Gmail drafts, Google Doc suggestions, and Google Sheet comments. The app shows these to the user in a window before it sends them.

Reason: text from other people (emails, issues, ClickUp tasks) goes into these chats. That text must not be able to change a login or send content out.

## Rules for each new tool

Each new tool must do all of these steps:

1. Add or reuse a function in `crates/aifred-core`.
2. Add the tool in `crates/aifred-tools`.
3. In an item chat or a mission chat, let the tool change only the items or the mission of that chat. Use `check_item` and `check_mission` (`crates/aifred-tools/src/tasks.rs:313`, `:345`).
4. Give back the old value as `before`, so the AI can undo the change.
5. Add the tool name to the allow list in `tool_servers_for`. Do not add a write tool to the read-only list for jobs.
6. Add tests: the tool test, and the allow-list tests in `assistant.rs`.
7. Write the tool text so the AI uses the tool only when the user's own words ask for the change.
8. Make sure that the page shows the change after the answer, with no manual reload.

## Steps

### Step 1. Missions (fixes the screenshot) — done, except the optional rename

1. Add `missions::set_checklist(dir, id, steps, now)`. It replaces all steps and keeps the tick of a step with the same text. It gives back the old list.
2. Set the limits for a change by the user: at most 12 steps, at most 100 characters each. The brief keeps its limit of 6 for a new mission.
3. Add the tools `set_mission_steps` (the full list) and `tick_mission_step` (one step, done or not done).
4. Add the tools `finish_mission` and `reopen_mission`. `finish_mission` does not close the items: it only finishes the mission.
5. Add the tool `skip_mission_today` and its undo.
6. Optional: add `rename_mission`. It needs a new column `title_by` and a new schema version. The brief must keep a title that the user set.

### Step 2. Small tools on existing functions — done

1. Memory: `edit_memory`, `set_memory_always`, `restore_memory`.
2. Day plan: `set_time_guess`.
3. Email: `hide_email` (with `hide: false` as the undo).
4. Brief: `correct_brief` and `undo_brief_correction`.
5. Local tasks: `delete_task` is not added. A delete has no undo: it also removes the links, blockers, mission place, and status of the task. The AI can set the status "cancelled" instead, and that has an undo.

### Step 3. ClickUp in the item chat and the mission chat — done

Give `mark_task_done`, `reopen_task`, `add_task_comment`, and `delete_task_comment` to the item chat and the mission chat. Limit them to the ClickUp tasks of that chat. Keep the rule that a comment uses only the user's own words.

### Step 4. Less common actions — done, except "make the brief again"

1. Work timer: `start_work`, `stop_work`. "Finish" needs no tool: the done and status tools keep the real time.
2. Status rules: `status_rules`, `set_status_rule` (from a ClickUp task that has the status now), `restore_status_rule`, `remove_status_rule`. Only the main chat and the plan chat can change them: one rule changes many tasks.
3. Week review: `review_pick`, `undo_review_pick`.
4. Brief: `write_brief` is not added. The brief is a second AI call. The app runs it with a lock, so that two briefs are not written at the same time. A tool server is a separate program and cannot use that lock. The user can make the brief again on the home page.

### Still open

None. On 2026-11-12 the user decided:

- Rename a mission: done. The tool `rename_mission` sets a "set by the user" mark (schema version 40). The brief keeps a title with this mark. The undo gives the old title and the old mark back.
- Make the brief again from a chat: not added. The "Write again" button on the home page does this.

## Decision

The user decided on 2026-11-12: tokens, settings, and the outside sends (Gmail drafts, Doc suggestions, Sheet comments) stay out of the AI.
