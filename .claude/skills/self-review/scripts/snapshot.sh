#!/usr/bin/env bash
# snapshot.sh: make the fixes of a self-review reversible.
#
# Usage:
#   bash snapshot.sh save <file>...   before the fixes: copy each file
#   bash snapshot.sh done             after the fixes: write fixes.patch and
#                                     print the undo command
#   bash snapshot.sh undo             put the saved copies back
#
# The copies are in <git dir>/self-review/fix/. A file that the fix made new is
# removed by undo. Undo skips a file that changed again after the fix, and
# says so. The user's own edits from before the fix are never touched.
set -euo pipefail
cd "$(git rev-parse --show-toplevel)"
STATE="$(git rev-parse --path-format=absolute --git-dir)/self-review"
FIX="$STATE/fix"
case "${1:-}" in
  save)
    shift
    rm -rf "$FIX"
    mkdir -p "$FIX/before" "$FIX/after"
    : > "$FIX/files.txt"
    for f in "$@"; do
      echo "$f" >> "$FIX/files.txt"
      if [ -e "$f" ]; then
        mkdir -p "$FIX/before/$(dirname "$f")"
        cp -p "$f" "$FIX/before/$f"
      fi
    done
    echo "saved $(wc -l < "$FIX/files.txt") file(s) in $FIX/before"
    ;;
  done)
    : > "$FIX/fixes.patch"
    while read -r f; do
      [ -z "$f" ] && continue
      if [ -e "$f" ]; then
        mkdir -p "$FIX/after/$(dirname "$f")"
        cp -p "$f" "$FIX/after/$f"
      fi
      b="$FIX/before/$f"; [ -e "$b" ] || b=/dev/null
      a="$f"; [ -e "$a" ] || a=/dev/null
      git diff --no-index -- "$b" "$a" >> "$FIX/fixes.patch" || true
    done < "$FIX/files.txt"
    echo "the fixes are in $FIX/fixes.patch"
    echo "undo: bash .claude/skills/self-review/scripts/snapshot.sh undo"
    ;;
  undo)
    [ -s "$FIX/files.txt" ] || { echo "no saved fixes"; exit 0; }
    while read -r f; do
      [ -z "$f" ] && continue
      if [ -e "$f" ] && [ -e "$FIX/after/$f" ] && ! cmp -s "$f" "$FIX/after/$f"; then
        echo "skipped $f: it changed after the fix"
        continue
      fi
      if [ -e "$FIX/before/$f" ]; then
        cp -p "$FIX/before/$f" "$f"
        echo "restored $f"
      elif [ -e "$f" ]; then
        rm -f "$f"
        echo "removed $f (the fix made it)"
      fi
    done < "$FIX/files.txt"
    ;;
  *)
    echo "usage: snapshot.sh save <file>... | done | undo" >&2
    exit 2
    ;;
esac
