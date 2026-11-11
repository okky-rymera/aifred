#!/usr/bin/env bash
# checks.sh: run the mechanical checks of the AIfred repo.
#
# Usage: bash checks.sh [--diff <patch file>]
#
# Each check prints "<name>: ok" or "<name>: FAIL" with the details below it.
# The last line is "result: all ok" or "result: N checks failed".
# The exit code is always 0. The report is the result.
#
# The checks: rustfmt, clippy (with tests), cargo test, the window typecheck,
# the window tests, the command names (Rust, api.ts, fake backend), and the
# database schema version when a diff is given.
#
# On a clean trunk every check is ok (checked on 2026-11-11), so a FAIL comes
# from the change under review. No check starts the app or opens a window.
# No check needs sudo.
set -uo pipefail
cd "$(git rev-parse --show-toplevel)"
export CARGO_TERM_COLOR=never NO_COLOR=1 CI=1
DIFF=""
if [ "${1:-}" = "--diff" ]; then DIFF="${2:-}"; fi
fail=0

ok() { echo "$1: ok"; }
bad() { # name, details, max lines
  fail=$((fail + 1))
  echo "$1: FAIL"
  printf '%s\n' "$2" | head -"${3:-40}" | sed 's/^/    /'
}

# 1. rustfmt
out=$(cargo fmt --all --check 2>&1)
if [ $? -eq 0 ]; then ok rustfmt; else
  bad rustfmt "$(printf '%s\n' "$out" | grep -E '^Diff in' | sed -E 's/^Diff in (.*) at line ([0-9]+):.*/\1:\2/' | sort -u)"
fi

# 2. clippy, also on the tests. Trunk has zero warnings, so any warning is new.
out=$(cargo clippy --workspace --all-targets --quiet 2>&1)
n=$(printf '%s\n' "$out" | grep -cE '^(warning|error)(\[|:)' || true)
if [ "$n" -eq 0 ]; then ok clippy; else
  bad clippy "$(printf '%s\n' "$out" | grep -E -A3 '^(warning|error)(\[|:)' | grep -vE '^(warning|error): [0-9]+ (warning|error)s? emitted')" 80
fi

# 3. cargo test. A test that fails once and passes on a second run is flaky
# (the embed helper tests depend on timing). It is noted, not a FAIL.
out=$(cargo test --workspace --quiet 2>&1)
if [ $? -eq 0 ]; then ok "cargo test"; else
  failed=$(printf '%s\n' "$out" | grep -oE '^---- [a-zA-Z0-9_:]+ ' | sed -E 's/^---- //; s/ $//' | sort -u)
  again_failed=""
  for t in $failed; do
    name=${t##*::}
    if ! cargo test --workspace --quiet "$name" > /dev/null 2>&1; then again_failed+="$t "; fi
  done
  if [ -z "$again_failed" ] && [ -n "$failed" ]; then
    echo "cargo test: ok (flaky: $(echo $failed) failed once and passed on a second run)"
  else
    bad "cargo test" "$(printf '%s\n' "$out" | grep -E '^test .* FAILED|^---- |panicked at|^error(\[|:)|assertion' | head -40)"
  fi
fi

# 4 and 5. The window: typecheck and tests.
if [ -d apps/desktop/node_modules ]; then
  out=$(cd apps/desktop && npm run -s typecheck 2>&1)
  if [ $? -eq 0 ]; then ok typecheck; else
    bad typecheck "$(printf '%s\n' "$out" | grep -E 'error TS' | head -40)"
  fi
  out=$(cd apps/desktop && npx vitest run 2>&1)
  if [ $? -eq 0 ]; then ok "window tests"; else
    bad "window tests" "$(printf '%s\n' "$out" | grep -E '^ *(FAIL|×|✗)|AssertionError|Error:|Tests  ' | head -40)"
  fi
else
  bad typecheck "apps/desktop/node_modules is missing. Run: cd apps/desktop && npm install"
  bad "window tests" "apps/desktop/node_modules is missing. Run: cd apps/desktop && npm install"
fi

# 6. Command names. README: "If you add or rename a command, update the fake backend too."
cmds=$(grep -A3 '#\[tauri::command\]' apps/desktop/src-tauri/src/main.rs | grep -oE 'fn [a-z_0-9]+' | sed 's/fn //' | sort -u)
inv=$(grep -rhoE 'invoke(<[^(]*>)?\(\s*"[a-z_0-9]+"' apps/desktop/src --include='*.ts' --include='*.tsx' \
  | grep -oE '"[a-z_0-9]+"' | tr -d '"' | sort -u)
fake=$(grep -oE 'case "[a-z_0-9]+"' apps/desktop/src/test/fakeBackend.ts | grep -oE '"[a-z_0-9]+"' | tr -d '"' | sort -u)
details=""
a=$(comm -23 <(echo "$cmds") <(echo "$inv")); [ -n "$a" ] && details+="in main.rs but no invoke in the window code: $(echo $a)"$'\n'
a=$(comm -13 <(echo "$cmds") <(echo "$inv")); [ -n "$a" ] && details+="invoked by the window but not a command in main.rs: $(echo $a)"$'\n'
a=$(comm -23 <(echo "$cmds") <(echo "$fake")); [ -n "$a" ] && details+="in main.rs but not in fakeBackend.ts: $(echo $a)"$'\n'
a=$(comm -13 <(echo "$cmds") <(echo "$fake")); [ -n "$a" ] && details+="in fakeBackend.ts but not in main.rs: $(echo $a)"$'\n'
if [ -z "$details" ]; then ok "command names"; else bad "command names" "$details"; fi

# 7. Schema version. A schema change needs a new VERSION in db.rs.
if [ -n "$DIFF" ] && [ -s "$DIFF" ]; then
  # Only added lines in Rust files count. A pattern in a script or a doc does not.
  schema=$(awk '/^\+\+\+ b\//{rs = ($0 ~ /\.rs$/)} /^\+/ && rs && /CREATE TABLE|ALTER TABLE|CREATE (UNIQUE )?INDEX|CREATE VIRTUAL TABLE|DROP TABLE|DROP INDEX/' "$DIFF")
  if [ -n "$schema" ]; then
    if grep -qE '^\+const VERSION: i64 = ' "$DIFF"; then ok "schema version"; else
      bad "schema version" "the diff adds SQL schema statements but does not change VERSION in crates/aifred-core/src/db.rs. Check by hand: a statement in a test or in a temporary table needs no new version."
    fi
  else
    ok "schema version"
  fi
else
  echo "schema version: skipped (no --diff)"
fi

if [ "$fail" -eq 0 ]; then echo "result: all ok"; else echo "result: $fail checks failed"; fi
