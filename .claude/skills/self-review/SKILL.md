---
name: self-review
description: Review the local code changes of the AIfred repository (Rust, Tauri, React) before a commit, with a strict and closed list of what counts as a finding. Use this skill, not wpdev:self-review, whenever the user in this repo asks to review their changes, check their code, do a self-review, a pre-commit review, "review my changes", "check what I did", "review before I commit", or "is this ready to commit". It runs the mechanical checks (rustfmt, clippy, cargo test, typecheck, the window tests, the command names, the schema version), reads the change cold in a fresh subagent, reports only findings under the rules R1 to R9, and applies the fixes with an undo. A second run on the same change reviews only the new lines, so it does not keep finding issues.
argument-hint: "[<git range> | <N commits>] [--report-only] [--full]"
user-invocable: true
---

# Self-review of a change to AIfred

One pass. The reviewer reports only what the rules allow. The fixer fixes only what the report names. Nobody looks again.

Paths in this file are relative to the repository root. The skill folder is `.claude/skills/self-review/`.

## Why the rules are closed

A review that looks for "anything wrong" finds something on every run, also on a change that is fine. The findings get smaller each time, and the author stops reading them. So this skill works the other way round: `references/rules.md` lists the nine kinds of problem that past reviews of this repo fixed (R1 wrong result, R2 data, R3 outside writes and access, R4 contract between the parts, R5 checks, R6 one rule in two places, R7 untrue text, R8 repeated work, R9 missing test). Each finding must name its rule, a line in the diff, a concrete trigger, the wrong result, and the fix. Everything else is not a finding, not even as a note. A clean report is final.

## Arguments

- no argument: the uncommitted changes (staged, unstaged, new files) when there are any, else the last commit.
- `N`: the last N commits. A git range such as `abc123..HEAD` also works. The fixes of such a run are uncommitted, so the next run has no argument.
- `--report-only`: report, do not fix.
- `--full`: forget the last run. Review every changed line again.

## Step 1: Scope

```bash
bash .claude/skills/self-review/scripts/scope.sh [N | range] [--full]
```

It writes the diff and the line lists to the run folder (`run_dir` in its output, `.git/self-review/run` in the main checkout) and prints `key: value` lines. Use the printed paths below where this file writes `<run_dir>` and `<last_dir>` (`<last_dir>` is `run` replaced by `last`). Tell the user the scope and the file count in one line.

- `nothing to review`: say so and stop.
- `same_as_last: yes`: the change did not move since the last run. Show the saved report (`last_report`) and stop. Do not review again.
- `first_run: no` with `new_lines: 0`: every changed line was reviewed already. Show the saved report with one line that says so, and stop.

## Step 2: Mechanical checks

```bash
bash .claude/skills/self-review/scripts/checks.sh --diff <run_dir>/diff.patch
```

Set the Bash timeout to 600000. It takes 1 to 3 minutes. It never opens a window and never needs sudo. Keep the output: it goes into the reviewer prompt and into the report. A FAIL line is a finding under R5. Do not fix anything yet.

## Step 3: The cold read

Read `references/reviewer.md`, fill the `{{ }}` parts, and send the prompt as one `Agent` call with `subagent_type: general-purpose`. Wait for it. Do not review the diff in this session: this conversation wrote the change, so it reads the change with the author's eyes.

## Step 4: Filter and show the report

Check each finding in the returned report against the rules file:

- it names a rule R1 to R9,
- its place is inside a hunk of the diff. Check all places with one call, never by hand (a path with "-" breaks a hand-made check):

  ```bash
  bash .claude/skills/self-review/scripts/scope.sh --check <file:line> <file:line> ...
  ```

  "not in a hunk" fails the finding. On a second run, "(reviewed before)" fails it too. An R5 finding is exempt: it points where the check output points.
- it has a trigger, a wrong result, and a fix.

Drop a finding that fails one of these, and say in one line how many you dropped and why. Do not add findings. Then show the report to the user as it is.

Save it:

```bash
cp <run_dir>/diff.patch <last_dir>/diff.patch
```

and write the report to `<last_dir>/report.md`.

## Step 5: Fix

Skip this step with `--report-only`, or when the report has no findings.

Fixes are the default because every finding carries its fix and every fix can be undone. Before the first edit, save the files that the fixes touch:

```bash
bash .claude/skills/self-review/scripts/snapshot.sh save <file>...
```

Then, for each finding, apply the fix that the report names. Only that. Do not improve code next to it, and do not look for more problems. A fix that needs a design decision, a new dependency, or a change outside the diff is not applied: say so under "Not fixed" with the reason.

Run the scope and the checks once more, so the checks see the diff with the fixes:

```bash
bash .claude/skills/self-review/scripts/scope.sh <the same arguments as in Step 1>
bash .claude/skills/self-review/scripts/checks.sh --diff <run_dir>/diff.patch
```

A check that a fix broke: repair it when the repair is mechanical (a format, an import, a test value), else undo that one fix and list the finding under "Not fixed". One repair round, no more.

Close the snapshot:

```bash
bash .claude/skills/self-review/scripts/snapshot.sh done
```

It prints the undo command. Update `<last_dir>/report.md` with the outcome of each finding (Fixed or Not fixed). Do not copy the diff again: `<last_dir>/diff.patch` stays the diff from before the fixes, so the next run sees the fix lines as the only new lines and reads them cold.

## Step 6: Summary

End with:

```
Verdict: Ready to commit | Not ready: N must fix
Fixed: <one line per finding>            (when fixes ran)
Not fixed: <one line per finding + why>  (when any)
Undo the fixes: bash .claude/skills/self-review/scripts/snapshot.sh undo
Next: run the skill again with no argument to review the fixes, or commit.
```

Never commit. Never run the app. A second run reviews the fix lines only.
