# Plan: make AIfred a real assistant

Date: 2026-11-05. The research sources were checked on this date.

## Goal

AIfred is the user's assistant. It must:

- know the user's tasks,
- put them in order of importance,
- suggest the work that each task needs,
- later, do the work through integrations (for example, update a Google Sheet or Doc).

## The main gap

AIfred knows what is open and when it is due. It does not know:

- why an item matters (the user's goals),
- how much time the user has (the calendar),
- who waits for the user, and who the user waits for (email and Slack).

These three things change a list into priorities.

## Keep these

- [x] Data stays on the computer. The AI service can be replaced. Hosted memory can disappear: the Rewind desktop app shut down after Meta bought the company ([9to5mac](https://9to5mac.com/2025/12/05/rewind-limitless-meta-acquisition/)).
- [x] A short daily brief that ends, with a reason for each item. ChatGPT Pulse and Google CC use the same idea. Too many alerts is the top reason why people stop using an assistant.
- [x] Jobs that run with nobody watching can only read. A job with nothing to say sends no notification.
- [x] AIfred says each job time back before it saves the job.

## Checklist

### 1. Goals (do this first)

Why: the brief ranks items only by due day, priority, and review requests (`crates/aifred-core/src/brief.rs`). Urgency can come from dates. Importance must come from the user's goals ([Asana: Eisenhower matrix](https://asana.com/resources/eisenhower-matrix)).

- [x] Let the user keep 3 to 5 current goals (for example, "ship StoreAgent 2.0 this month").
- [x] Give the goals to the brief and the day plan.
- [x] For each ranked item, say which goal it serves, or "no goal".
- [x] Ask the user to check the goals when they are older than 30 days.

Done: the goal card on the home screen, the goal tools in the chat, and the goal of each item in the brief and the day plan (`crates/aifred-core/src/goals.rs`). Not done yet: check with real use that the brief ranks better.

### 2. Google Calendar (read only)

Why: the day plan does not know about meetings. It can plan 6 hours of work on a day with 2 free hours. Sunsama warns when a day is too full ([Sunsama](https://help.sunsama.com/docs/daily-planning)). Reclaim protects focus time ([Reclaim](https://help.reclaim.ai/en/articles/11325700-habits-vs-tasks-vs-focus-time-when-to-use-each-in-reclaim)).

- [x] Read today's and this week's events.
- [x] Compare the time guesses in the day plan with the free hours. Warn when the plan is too full.
- [x] Put plan items into the free blocks, not over meetings.
- [x] Write a short prep note before each meeting: who attends, the goal, the open items that are linked to the meeting.

Done: AIfred reads the primary calendar for the next 7 days through its own Google connection (`crates/aifred-core/src/google.rs`, `calendar.rs`). Each user makes their own Google registration. Later Google features use the same connection. The plan goes around busy events. The brief and the plan chat get the free minutes. The "Prep" button on a meeting asks the plan chat for the note. Not done yet:
- Other calendars (the shared calendars) do not count as busy.
- AIfred writes the prep note only when the user clicks "Prep". It does not write it by itself before the meeting.

### 3. Gmail (read only) and a "waiting for" list

Why: email is one of the largest sources of tasks. Superhuman and Copilot find threads with no reply ([Superhuman](https://help.superhuman.com/hc/en-us/articles/45270478397203-Reminders-on-Autopilot), [Thurrott](https://www.thurrott.com/cloud/335438/copilot-in-outlook-can-now-proactively-manage-your-inbox-and-calendar)).

- [x] Find emails that wait for the user's reply.
- [x] Find emails where the user waits for a reply from another person.
- [x] Add a "waiting for" list: the item, the person, and the date.
- [x] Follow-up jobs: "tell me if Ana has not replied by Friday" (step 2 in `docs/research-agent-jobs.md`).
- [ ] Later: Slack mentions and meeting next steps.

Done: AIfred reads the last 14 days of Gmail through the same Google connection, with read-only access (`crates/aifred-core/src/gmail.rs`). It reads only the headers and Google's short preview. Emails that wait for the user's answer are items with the key `gmail:<thread>`, so the brief and the plan rank them. Emails that the user sent with no answer are suggestions for the "waiting for" list (`crates/aifred-core/src/waiting.rs`). An email item closes when the person answers. A check day sends a notification with no AI call. It is not a general job: the scheduler checks the list on each tick. Not done yet:
- Check with real use that the rules find the right emails (people, not machines; not too many).
- The rules see only the headers and the preview. An email that says "no need to answer" still counts.
- Only the newest 100 inbox threads and 50 sent threads of the last 14 days.
- The user must connect Google again once to allow Gmail.

### 4. Learn from the user's corrections

Why: Google CC learns when the user replies to its daily email ([TechCrunch](https://www.techcrunch.com/2025/12/16/google-tests-an-email-based-productivity-assistant/)). Pulse lets the user rate each card. AIfred learns nothing when the user changes the plan.

- [x] Add "not today" and "wrong priority" to the items in the brief.
- [x] Save a pattern from repeated corrections as a memory fact (for example, "low priority: the docs repo").
- [x] Record when the user starts and finishes an item. Compare the real time with the time guess.
- [x] Use the real times to make the next time guesses better.

Done: each reason in the brief has "Not today" and "Wrong priority" (`crates/aifred-core/src/learn.rs`). The item loses its reason that day. The brief and the plan chat get the corrections of the last 30 days. Three "wrong priority" corrections of different items from one place (a repository, a ClickUp list, a sender, a tag) in 60 days make one core memory fact. The brief now gets the memory facts. A timer ("Start", "Stop", "Done") records the real time of each item (`crates/aifred-core/src/worktime.rs`). The brief and the plan chat get the last 20 real times and the middle value of real time divided by guess. Not done yet:
- Check with real use that 3 corrections is the right number, and that the facts are useful.
- A correction does not change the brief text. Only the reason of the item goes away until AIfred writes the brief again.
- Only "too high" is a correction. There is no "too low" button for an item that the brief did not name.
- AIfred does not change a time guess that exists. Only new guesses use the real times.
- The timer does not stop by itself when an email item closes because the person answered.

### 5. Reviews

Why: in GTD (Getting Things Done), the weekly review keeps the list trustworthy. Most people skip it ([Thomas Frank](https://thomasjfrank.com/productivity/the-5-minute-guide-to-gtd-getting-things-done/)).

- [x] End-of-day check: what is done, what moves to tomorrow.
- [x] Weekly review: a list of items that did not move in 3 weeks. For each item, the user picks one: do, plan, give to someone, or drop.
- [x] Ask about goals in the weekly review.

Done: the "End of the day" card and the "Weekly review" card on the home page (`crates/aifred-core/src/review.rs`, `apps/desktop/src/Review.tsx`). Both need no AI call. The end-of-day check puts the ticked open items in tomorrow's plan. The weekly review asks about the goals, with the finished items of each goal in the last 7 days, and takes one pick for each item with no change for 21 days. Each pick has an Undo. The brief and the plan chat get the picks. Not done yet:
- Check with real use that 21 days is the right age, and that the cards come at good times.
- No notification for the reviews. The cards show only when the user opens the home page.
- When tomorrow has no plan, the moved items wait in a plan with the mode "review", and the app still asks how to plan the day. "Plan it for me" asks the AI to keep them, but the AI can still drop them.
- Each edit of a task counts as a change, also a new due day. So a task that the user moves again and again is never in the weekly review.
- A dropped GitHub or ClickUp item stays in the home list. Only the brief and the reviews stop counting it.
- The weekly review day is 7 days after the last review. The user cannot pick a day of the week.

### 6. One task for one piece of work

Why: a ClickUp task, its GitHub issue, and the pull request for the same work show as 2 or 3 rows. The research found no standard method to merge them.

- [ ] Let one task link to many source items (ClickUp task, GitHub issue, pull request, email thread).
- [ ] Suggest links when titles or descriptions match. The user accepts or rejects each link.
- [ ] Show one row for the linked items, with the sources under it.

### 7. Suggest the work

- [ ] For an item, suggest the next action in one line ("reply to Ana with the price", "review PR #12").
- [ ] Split a large item into steps (the task checklist already exists).
- [ ] Say what AIfred can do for the user later, and what the user must do.

### 8. Actions in other apps (do this last)

Warning: when AIfred reads email and can also change things, one bad email can make the AI leak data or make changes. Simon Willison calls this the "lethal trifecta": private data, text from strangers, and a way to send data out ([his post](https://simonwillison.net/2025/Jun/16/the-lethal-trifecta/)).

- [ ] Make drafts first, in the app's own review place: Gmail drafts, Google Docs suggestions, Sheets comments. The Docs API can add suggestions and comments since September 2026 ([Google](https://workspaceupdates.googleblog.com/2026/09/programmatic-comment-and-suggestion.html)). Google's Gmail tool for AI can make drafts, but it cannot send ([Google](https://developers.google.com/workspace/guides/configure-mcp-servers)).
- [ ] Set approval by the damage that an action can do. Reading and drafting run freely. Sending, deleting, and changes that other people can see need a clear "yes".
- [ ] Do not accept a vague reply as a "yes".
- [ ] Keep a log of each action, with Undo.
- [ ] An AI run that read email, Slack, or other text from strangers cannot write to an app outside the computer. Use the same split as the read-only jobs.
- [ ] Show each change before it is made (before and after).

## Not checked

The research could not check these items. This plan does not use them:

- An HBR number: 8% more output with an executive assistant.
- How Claude Cowork's memory works.
- The exact request format for Google Docs suggestions.
