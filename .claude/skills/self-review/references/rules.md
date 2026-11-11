# Review rules

This is the closed list of what a self-review of AIfred reports. A problem that is not under one of the rules R1 to R9 is not a finding. It is not reported, not as a note and not as a nit.

The list comes from the fixes that past reviews of this repo made (see the "Fixes from the code review" lists in `git log`). Each rule names a wrong result that the user of AIfred can see.

## What a finding needs

Every finding has these five parts. A finding that misses one is dropped.

1. **Rule**: one of R1 to R9.
2. **Place**: `file:line`. The line is in a hunk of the diff under review (`hunks.txt`). A removed line gets the line where it was. On a second run, the hunk has a line in `new-lines.txt`. For R4 and R7, the place is the changed code line, not the README or comment line: a README line outside the diff is not a place.
3. **Trigger**: the input, click, or state that starts the problem. Concrete: "the user adds a task with the due day 'friday' on a Friday", not "an edge case".
4. **Wrong result**: what the user sees, loses, or gets. Concrete: "the task gets today as the due day", not "incorrect behavior".
5. **Fix**: the change, as code (2 to 10 lines) or as one sentence that names the function and the change.

Confirm the trigger in the code before you report. Read the full function, and the callers when the trigger depends on them. A guess is not a finding. When you cannot confirm the trigger, drop the finding.

## Tiers

- **Must fix**: the trigger is a normal use of the app, or the rule is R2, R3, R4, or R5.
- **Should fix**: the trigger is rare, or the rule is R6, R7, R8, or R9.

The verdict is "Not ready: N must fix" when a Must fix finding is open. Else it is "Ready to commit", also when Should fix findings are open.

## The rules

### R1. Wrong result

A path that exists now gives a wrong answer, shows a wrong screen, loses a change, or stops (a panic, an unhandled promise rejection, a frozen page). Examples from past reviews:

- a limit counts the wrong things ("the limit of 10 blockers counted closed blockers too"),
- a date, time zone, or rounding error,
- an undo that does not put everything back,
- a double click, a second save, or two parallel runs that do the work two times,
- a failed save that shows success, or that saves again,
- a screen that keeps old data after a change,
- a new path that makes an old defect reachable (then the trigger names the new path).

### R2. Data

- The diff changes the SQL schema (a table, a column, an index) without a new `VERSION` in `crates/aifred-core/src/db.rs` and a migration step.
- A migration loses rows or converts a value wrongly.
- Two or more writes that must stay together are not in one transaction, so a failure between them leaves half of the change.
- The diff changes the fields that a saved list keeps (GitHub, ClickUp, Gmail) without a new list version, so old rows never get the new fields.
- The diff adds a user action that removes or overwrites something and has no undo. AIfred's rule: every user change can be reversed.

### R3. Outside writes and access

The README sections "Access limits" and the tool descriptions are the contract.

- A new way for Claude, a tool server, or an AI job to write to GitHub, ClickUp, Google, or Gmail without the guards of the existing writes: the user asked for it in this chat, the text is the user's own words, and an undo exists. An AI job can only read. A new write that has the guards, but a README that still says "read only", is R7, not R3: the README is stale, the access rule is kept.
- Text from other people (item titles and descriptions, comments, emails, documents, custom fields) reaches an outside write, a shell command, a file path, or an instruction position in a prompt, without the guard that the repo uses for it (the text is marked as data; a comment must be in the user's own words).
- A token, a password, or an access key goes to a log, to a database text column, to the AI prompt, or to a tool answer.
- A wider Tauri capability, a new environment variable, or a new owner outside `AIFRED_GITHUB_OWNERS` changes what the app or the AI can reach.

### R4. Contract between the parts

- A Tauri command is added, renamed, or gets a new argument or result shape, and `apps/desktop/src/api.ts` or `apps/desktop/src/test/fakeBackend.ts` does not follow. The names are checked by `checks.sh`. The shapes (argument names, result fields) are checked by the reviewer.
- A struct field changes its name or type on the Rust side or on the TypeScript side only (serde and `api.ts` types).
- An MCP tool changes its name, its arguments, or its answer, and the text that tells the AI how to use it (the tool description, the brief or chat instructions) does not follow.
- A terminal command (`aifred ask`, `chat`, ...) changes and the README section "Use" does not follow.

### R5. Checks

Every FAIL line from `checks.sh` is a finding: rustfmt, clippy, cargo test, typecheck, the window tests, the command names, the schema version. Report it as it is, with the file and line from the check output. When the cause is a problem that another finding names (for example a test that fails because of an R1 bug), do not make a second finding: add one line "Check: cargo test fails on <test> because of this" to that finding.

### R6. One rule in two places

The diff adds a second copy of a limit, a format, a parse, or a rule that must agree with a copy that already exists, for example the 600-character limit of the notes box in both the window and the Rust side. The next change updates one copy only. Report this only when both copies must agree. Other repeated code is not a finding.

### R7. Untrue text

- A README line, a tool description, or a code comment that the diff makes untrue.
- A new control, command, setting, or limit that the user can see, and the README has no line for it.

Wording, length, and style of the text are not findings.

### R8. Repeated work that grows with the data

A database query, a network call, or a full list read inside a loop over items, facts, messages, or days, where the user's data makes the loop long (hundreds of items). Name the loop and the path that reaches it. A loop over a small fixed list is not a finding.

### R9. Missing test

The diff adds a rule with a concrete expected value (a limit, a date computation, a parse, a status pick, a sort order) in a Rust file that already has a `#[cfg(test)]` module, and no test exercises the new rule. Name the input and the expected value that the test needs. Window code, wiring, and UI layout are not under this rule.

## What is never a finding

- A line that the diff did not change. On a second run: a line that the last run reviewed. The one exception is R1 for an old defect that the new code makes reachable.
- Names, the wording of comments, messages, and logs, formatting that rustfmt accepts, the order of items, imports, file length, function length.
- "Could be simpler", "extract a helper", "consider", and repeated code that R6 does not cover.
- Speed, unless R8 applies with a named loop and path.
- A future change ("if someone later adds...").
- `unwrap`, `expect`, and `as` casts, unless R1 applies with a reachable trigger.
- Test code style, test names, and missing tests that R9 does not cover.
- Accessibility, translations, and browser compatibility. AIfred is a one-user desktop prototype in English.
- Files under `docs/`, `Cargo.lock`, `package-lock.json`, `apps/desktop/dist/`, `apps/desktop/src-tauri/gen/`.
- A finding that the last run reported. It goes under "Still open from the last run", one line, no new analysis.

## Finding format

```
R1 Must fix — crates/aifred-core/src/tasks.rs:120
Trigger: the user adds a task with the due day "friday" on a Friday.
Wrong result: the task gets today as the due day. The README says "friday" is the next Friday.
Fix: in due_day(), when the weekday is today, add 7 days.
```

## Report format

```
Self-review: <scope> (<N> files)
Checks: rustfmt ok · clippy ok · cargo test ok · typecheck ok · window tests ok · command names ok · schema version ok
Findings: <N> (<M> must fix)

<findings, Must fix first>

Fixed since the last run: <one line per old finding whose trigger is gone>     (omit when none)
Still open from the last run: <one line per old finding that still applies>   (omit when none)

Verdict: Ready to commit | Not ready: <M> must fix
```

When there are no findings, the report says "Findings: 0" and "No findings." and the verdict "Ready to commit". That result is final. Do not look again.
