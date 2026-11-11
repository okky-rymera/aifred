#!/usr/bin/env bash
# scope.sh: find the changes to review and write them to .git/self-review/run/.
#
# Usage: bash scope.sh [<git range> | <N>] [--full]
#        bash scope.sh --check <file:line>...   (see below)
#   no argument : the uncommitted changes (staged, unstaged, and new files)
#                 when there are any, else the last commit.
#   N           : the last N commits.
#   <range>     : a git range, for example abc123..HEAD.
#   --full      : ignore the last run. Every changed line counts as new.
#
# Output: "key: value" lines, then the changed files, one per line.
#   scope, run_dir, files, changed_lines, first_run, same_as_last, new_lines, last_report
#
# Files in the run folder:
#   diff.patch        the unified diff
#   files.txt         the changed files (deleted files are not listed)
#   changed-lines.txt every changed line as "file:line: text"; a removed line
#                     as "file:line: - text" (line: the hunk's place in the new file)
#   hunks.txt         "file:first-last" of each hunk, in new-file lines
#   new-lines.txt     the changed lines that the last run did not review
#
# It writes nothing outside <git dir>/self-review (`.git/self-review` in the main
# checkout, `.git/worktrees/<name>/self-review` in a worktree). It never changes the index.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
STATE="$(git rev-parse --path-format=absolute --git-dir)/self-review"

# --check <file:line>...: is each place inside a hunk of the current run?
# Prints "<file:line>: in hunk <first-last>" or "<file:line>: not in a hunk".
# On a second run it also says "(new)" when the hunk has a line that no
# earlier run reviewed. Use it in Step 4 of the skill. A path with "-" in
# it breaks a hand-made check, so do not do it by hand.
if [ "${1:-}" = "--check" ]; then
  shift
  for q in "$@"; do
    f=${q%:*}; l=${q##*:}
    hit=$(grep -F -- "$f:" "$STATE/run/hunks.txt" 2>/dev/null | sed "s#^$f:##" \
      | awk -F- -v l="$l" '$1 <= l && $2 >= l { print $0; exit }')
    if [ -z "$hit" ]; then echo "$q: not in a hunk"; continue; fi
    a=${hit%-*}; b=${hit#*-}
    new=$(grep -F -- "$f:" "$STATE/run/new-lines.txt" 2>/dev/null | sed -E "s#^$f:([0-9]+):.*#\1#" \
      | awk -v a="$a" -v b="$b" '$1 >= a && $1 <= b { c++ } END { print c + 0 }')
    if [ "$new" -gt 0 ]; then echo "$q: in hunk $hit (new)"; else echo "$q: in hunk $hit (reviewed before)"; fi
  done
  exit 0
fi
RUN="$STATE/run"
LAST="$STATE/last"
FULL=0
ARG=""
for a in "$@"; do
  case "$a" in
    --full) FULL=1 ;;
    *) ARG="$a" ;;
  esac
done
rm -rf "$RUN"
mkdir -p "$RUN" "$LAST"

# Generated and lock files are never reviewed.
EXCL=(':!Cargo.lock' ':!**/package-lock.json' ':!apps/desktop/dist/**' ':!apps/desktop/src-tauri/gen/**' ':!target/**')
SKIP_RE='(^|/)(Cargo\.lock|package-lock\.json)$|/dist/|/gen/|^target/'

dirty=$(git status --porcelain --untracked-files=all | grep -vE "$SKIP_RE" || true)
if [ -z "$ARG" ] && [ -n "$dirty" ]; then
  scope="uncommitted changes"
  git diff HEAD -- . "${EXCL[@]}" > "$RUN/diff.patch"
  # New files are not in `git diff`. Add each one as a new-file hunk.
  { git ls-files --others --exclude-standard | grep -vE "$SKIP_RE" || true; } | while read -r f; do
    [ -L "$f" ] && continue # a link, for example to a node_modules folder
    git diff --no-index -- /dev/null "$f" >> "$RUN/diff.patch" || true
  done
else
  if [ -z "$ARG" ]; then
    range="HEAD~1..HEAD"
    scope="the last commit ($(git log -1 --format='%h %s'))"
  elif [[ "$ARG" =~ ^[0-9]+$ ]]; then
    range="HEAD~$ARG..HEAD"
    scope="the last $ARG commits"
  else
    range="$ARG"
    scope="commits $ARG"
  fi
  if ! git rev-list -n 1 "$range" > /dev/null 2>&1; then
    echo "error: git does not know the range '$range'" >&2
    exit 2
  fi
  git diff "$range" -- . "${EXCL[@]}" > "$RUN/diff.patch"
  if [ -n "$dirty" ]; then
    echo "note: there are also uncommitted changes. They are not in this scope." >&2
  fi
fi

grep -E '^diff --git a/' "$RUN/diff.patch" | sed -E 's#^diff --git a/.* b/##' | sort -u > "$RUN/files.txt" || true

# Every changed line as "file:line: text". A removed line shows as
# "file:line: - text", where line is the place of the hunk in the new file.
# hunks.txt has "file:first-last" for each hunk, in new-file lines.
awk '
  /^diff --git / { f = $0; sub(/^diff --git a\/.* b\//, "", f); next }
  /^@@/ {
    split($3, a, ","); n = substr(a[1], 2) + 0
    len = (a[2] == "" ? 1 : a[2] + 0); if (len == 0) len = 1
    print f ":" n "-" (n + len - 1) > "'"$RUN/hunks.txt"'"
    next
  }
  /^\+\+\+ / || /^--- / { next }
  /^\+/ { print f ":" n ": " substr($0, 2); n++; next }
  /^-/ { print f ":" n ": - " substr($0, 2); next }
  /^ / { n++ }
' "$RUN/diff.patch" > "$RUN/changed-lines.txt"

files=$(wc -l < "$RUN/files.txt")
changed=$(wc -l < "$RUN/changed-lines.txt")

first_run=yes
same_as_last=no
related=no
if [ -s "$LAST/diff.patch" ] && [ "$FULL" -eq 0 ]; then
  # The last run matters only when it touched one of the same files.
  # Else this is unrelated work, and the old report must not follow it.
  grep -E '^diff --git a/' "$LAST/diff.patch" | sed -E 's#^diff --git a/.* b/##' | sort -u > "$RUN/last-files.txt" || true
  if [ -n "$(comm -12 "$RUN/last-files.txt" "$RUN/files.txt")" ]; then related=yes; fi
  rm -f "$RUN/last-files.txt"
fi
if [ "$related" = yes ]; then
  first_run=no
  if cmp -s "$LAST/diff.patch" "$RUN/diff.patch"; then
    same_as_last=yes
  fi
  # A line is new when the last run did not see the same text in the same file.
  # Line numbers move, so the text is the key.
  awk '
    /^diff --git / { f = $0; sub(/^diff --git a\/.* b\//, "", f); next }
    /^\+\+\+ / || /^--- / { next }
    /^\+/ { print f ": " substr($0, 2) }
    /^-/ { print f ": - " substr($0, 2) }
  ' "$LAST/diff.patch" | sort -u > "$RUN/last-added.txt"
  sed -E 's/^([^:]+):[0-9]+: /\1: /' "$RUN/changed-lines.txt" > "$RUN/now-added.txt"
  # Keep the "file:line: text" form for the new lines.
  paste -d '\t' "$RUN/now-added.txt" "$RUN/changed-lines.txt" \
    | awk -F '\t' 'NR==FNR { seen[$0] = 1; next } !($1 in seen) { print $2 }' "$RUN/last-added.txt" - \
    > "$RUN/new-lines.txt"
  rm -f "$RUN/last-added.txt" "$RUN/now-added.txt"
else
  cp "$RUN/changed-lines.txt" "$RUN/new-lines.txt"
fi
new=$(wc -l < "$RUN/new-lines.txt")

echo "scope: $scope"
echo "run_dir: $RUN"
echo "files: $files"
echo "changed_lines: $changed"
echo "first_run: $first_run"
echo "same_as_last: $same_as_last"
echo "new_lines: $new"
if [ -s "$LAST/report.md" ] && [ "$related" = yes ]; then
  echo "last_report: $LAST/report.md"
else
  echo "last_report: none"
fi
if [ "$files" -eq 0 ]; then
  echo "nothing to review"
fi
cat "$RUN/files.txt"
