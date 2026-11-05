# AIfred's own jobs: research notes

Date: 2026-11-05. Sources were checked on this date. Items marked "unverified" need a check before you depend on them.

## Goal

AIfred must follow through on actions, as a human assistant does. It keeps its own list of jobs, apart from the user's to-do list. Examples:

- "Remind me every Monday at 9 to submit my timesheet."
- "Remind me on the 3rd and 17th."
- "Each time I finish a task, update the brief."
- "Check PR #123 tomorrow and tell me if it merged."

## Summary

Most products use the same basic design:

- Each job has a trigger (when it runs), an action, and a destination (where the result goes).
- A small limit on active jobs.
- Pause, edit, and delete controls, and a run history.
- After a missed run, one late run at most, never one run for each missed time.
- Before the job is saved, the assistant says the time that it understood back to the user.

## 1. How other products do it

| Product | What to copy |
|---|---|
| ChatGPT Tasks ([help](https://help.openai.com/en/articles/10291617), unverified: page blocked, numbers from search snippets) | 5 to 15 active tasks by plan. One run each hour at most. Tasks nobody looks at can pause. |
| Gemini scheduled actions ([help](https://support.google.com/gemini/answer/16316416)) | Daily, weekly, monthly. Pause, resume, delete. 10 active actions (unverified). |
| Claude Code `/loop` ([docs](https://code.claude.com/docs/en/scheduled-tasks)) | 50 jobs per session. Recurring jobs end after 7 days. A late job runs once, not once per missed time. |
| Claude Code Desktop local tasks ([docs](https://code.claude.com/docs/en/desktop-scheduled-tasks)) | Checks every minute while open. After sleep: one late run for the newest missed time, within 7 days. Shows skipped runs with the reason. A permission level for each job. |
| Claude Code cloud tasks ([docs](https://code.claude.com/docs/en/routines)) | Says the exact time back before it saves a one-time job. 1 hour minimum gap. |
| OpenClaw ([docs](https://docs.openclaw.ai/automation/cron-jobs/how-it-works)) | Jobs and run history in SQLite. Retries with growing waits. Turns a job off after repeated errors. Quiet hours. A run with nothing new answers "no reply" and sends no notification. |
| Letta sleep-time agents ([docs](https://docs.letta.com/guides/agents/sleep-time-agents)) | A job that starts after N events, not on a clock. |
| Apple Reminders ([help](https://support.apple.com/en-afri/guide/iphone/iphec7a1de82/ios)) | Time, place, or event triggers ("when I message this person"). |

## 2. Data model

- **Two kinds of action.** A reminder only shows a notification. It costs no AI call. An agent job runs Claude with a saved prompt.
- **Trigger types:**
  - once: one local time,
  - repeat: a calendar rule,
  - event: for example "a task is done", with a wait,
  - watch: check a condition on an interval, until a set end (later),
  - deadline with a cancel rule (later).
- **Repeat rules use RRULE** ([RFC 5545](https://icalendar.org/iCalendar-RFC-5545/3-8-5-3-recurrence-rule.html)), not cron. RRULE can say "every other week" (`INTERVAL=2`), "last Friday" (`BYDAY=-1FR`), "last day of the month" (`BYMONTHDAY=-1`), and an end (`COUNT`, `UNTIL`). Cron cannot.
- **Time zones:** store the local wall time and the zone name (for example `Asia/Jakarta`). Do not store only UTC: zone rules can change ([Jon Skeet](https://codeblog.jonskeet.uk/2019/03/27/storing-utc-is-not-a/)).
- **Next run:** each job keeps its next run time in UTC, with an index. AIfred calculates it again after each run, after each change, and at each start.
- **States:** active, paused, done, off (after errors).
- **Run history:** one row for each run, with the planned time, the result, the reason for a skip, the cost, and the error. The pair (job, planned time) is unique.

## 3. Scheduler

- **A loop in the app.** It ticks every 30 seconds. AIfred already stays in the tray when the window closes. OS timers (systemd, launchd) can come later.
- **Sleep:** if the clock jumped much more than one tick, the computer was asleep. Then the loop checks for missed runs.
- **Missed runs:** run at most one, the newest, marked as late. Record the older ones as skipped. Give the planned time and the real time to the prompt.
- **No double runs:** in one database transaction, add the run row and move the next run time forward. Start the job only after that.
- **One run of each job at a time.** If a run is still busy, skip the next time.
- **Errors:** retry with growing waits. Turn the job off after 5 failures in a row and tell the user.

## 4. Event triggers

- The tool servers run in other processes. So a task change writes an event row in the database. The scheduler reads the events.
- **Wait for a quiet moment:** run 90 seconds after the last event, but at most 10 minutes after the first one. 8 tasks done in one minute give one brief refresh.

## 5. Safety

- A job that runs with nobody watching can only read, notify, and write a text. It cannot change tasks, GitHub, or ClickUp ([OWASP LLM06](https://genai.owasp.org/llmrisk/llm062025-excessive-agency/)). AIfred enforces this with the tool list of the Claude call, not with the prompt.
- A job run gets no job tools. So a job cannot make new jobs.
- At most 20 active jobs.
- AIfred says each new job back to the user with its next run times, for example "Mondays 09:00 (Asia/Jakarta). Next: Mon 9 Nov, Mon 16 Nov".
- An agent job that has nothing to report answers with a fixed word, and the user gets no notification.

## 6. Pitfalls in plain-word times

- "Next Friday": people do not agree on the day. Always say the full date back.
- "The 31st": RRULE skips months with no 31st. Ask, or use the last day of the month (`BYMONTHDAY=-1`).
- "Every other week" needs a start date. Without it, it is not clear which week counts.
- "In 2 hours" is real time (UTC + 2 hours), not wall time + 2.
- A time in the spring-forward gap does not exist. Reject it.
- "Tomorrow" with no time: use 09:00 and say it back.
- The AI gives the local time, the zone, and the rule. Code checks them and calculates the next runs. The AI never calculates UTC offsets.

## 7. Cases to test

1. "Every Monday at 9": a weekly rule, a reminder only, no AI call.
2. "The 3rd and 17th": `BYMONTHDAY=3,17`.
3. "The 31st": ask, or the last day. Check February.
4. "Every other Thursday from next week": `INTERVAL=2` with the right start.
5. "Last Friday of the month": `BYDAY=-1FR`.
6. "Tomorrow" sent at 23:50 and at 00:10.
7. "In 2 hours" across a summer time change.
8. The computer sleeps through 3 daily runs: one late run, 2 skipped rows.
9. The app is closed when a one-time reminder is due: a late notification at start.
10. The app crashes during a run: no second run for the same time.
11. The app and the terminal command use the same database: one run.
12. A run takes longer than its gap: the next time is skipped.
13. 10 tasks done in 60 seconds: one brief refresh.
14. Change "Monday" to "Tuesday": the next run moves, no double run.
15. Pause, then resume after missed times: no burst of runs.
16. "Stop the timesheet reminder" when 2 jobs match: AIfred asks which one.
17. The limit of 20 active jobs: a clear refusal.
18. 5 failures in a row: the job turns off and AIfred tells the user.
19. An agent job with nothing new: no notification.
20. A recurring job with `COUNT=5` or `UNTIL`: it ends as done.

## Decisions

These are the defaults until the user decides otherwise:

- AIfred's jobs are a separate list, not a view of the user's tasks.
- A job that runs with nobody watching only reads and notifies. It does not change anything.
- At most 20 active jobs.

## Plan

- **Step 1 (done, 2026-11-05):** jobs table and run history, reminders (once and repeat), the event "a task is done" with the brief refresh, agent jobs with read-only tools, the job tools for the chat, desktop notifications, and a jobs list in the window.
- **Step 2:** watches with an end ("tell me when PR #123 merges"), and deadlines with a cancel rule ("follow up if Ana has not replied by Friday").
- **Step 3:** quiet hours, cost limits, and OS timers that work when the app is closed.
