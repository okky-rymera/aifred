#!/usr/bin/env bash
# Start the desktop app on a hidden virtual screen (Xvfb), so no window
# opens on your real screen. Use it to check a change in the real app.
#
#   scripts/run-hidden.sh [max-seconds] [screenshot.png]
#
# - max-seconds: the longest time to wait for the app to show. Default: 300.
#   A first Rust build can take some minutes.
# - screenshot.png: where to save a picture of the hidden screen.
#   Default: no picture.
#
# The script waits until the page shows content, waits 5 more seconds, takes
# the picture, and stops the app. It exits with 1 if the app does not show.
#
# The hidden copy does not touch your own copy of the app:
# - It uses a free web port (1430 or higher). Your copy uses 1420.
# - It does not read or write the saved window size (AIFRED_NO_WINDOW_STATE).
# - It uses a temporary copy of your data folder, which is deleted at the end.
# - It uses its own message bus (dbus-run-session), so its notifications do
#   not show on your desktop.
#
# AIFRED_HOLD_SECONDS: keep the app running this many more seconds after it
# shows, for example to let AIfred's jobs run. Default: 0.
set -euo pipefail

MAX_SECONDS="${1:-300}"
SHOT="${2:-}"
SETTLE=5
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
DATA="${AIFRED_HOME:-$HOME/.aifred}"

for tool in Xvfb magick setsid ss; do
  command -v "$tool" >/dev/null || { echo "Missing program: $tool" >&2; exit 1; }
done

WORK="$(mktemp -d)"
LOG="$WORK/app.log"
XVFB_PID=""
PGID=""

stop_all() {
  if [ -n "$PGID" ]; then
    kill -TERM -- "-$PGID" 2>/dev/null || true
    # Wait up to 10 seconds for the app to exit, then force it.
    for _ in $(seq 1 20); do
      kill -0 -- "-$PGID" 2>/dev/null || break
      sleep 0.5
    done
    kill -KILL -- "-$PGID" 2>/dev/null || true
  fi
  [ -n "$XVFB_PID" ] && kill "$XVFB_PID" 2>/dev/null || true
  rm -rf "$WORK"
}
trap stop_all EXIT
trap 'exit 130' INT TERM HUP

# Temporary copy of the data folder. Leave out the lock files.
mkdir -p "$WORK/data"
if [ -d "$DATA" ]; then
  cp -a "$DATA/." "$WORK/data/"
  rm -f "$WORK/data/"*.lock
fi

# Xvfb picks a free display number itself and writes it when it is ready.
setsid Xvfb -displayfd 3 -screen 0 1920x1080x24 -nolisten tcp 3>"$WORK/display" >/dev/null 2>&1 &
XVFB_PID=$!
for _ in $(seq 1 50); do
  [ -s "$WORK/display" ] && break
  kill -0 "$XVFB_PID" 2>/dev/null || { echo "Xvfb did not start." >&2; exit 1; }
  sleep 0.1
done
[ -s "$WORK/display" ] || { echo "Xvfb did not start in 5 seconds." >&2; exit 1; }
DISP=":$(tr -d '[:space:]' <"$WORK/display")"

# A free web port. Save the output first: "grep -q" can end the pipe early.
PORT=1430
while [ -n "$(ss -Hltn "sport = :$PORT")" ]; do PORT=$((PORT + 1)); done
CONFIG="{\"build\":{\"devUrl\":\"http://localhost:$PORT\",\"beforeDevCommand\":\"npm run dev -- --port $PORT\"}}"

# A private message bus when the tool is there. Without it, notifications
# from the hidden copy show on your desktop.
BUS=""
command -v dbus-run-session >/dev/null && BUS="dbus-run-session --"

# Start the app in its own process group. The inner shell writes its PID,
# which is the group ID, and then becomes tauri. Remove every link to the
# real screen, so GTK uses the hidden X screen.
cd "$ROOT/apps/desktop"
setsid bash -c 'echo $$ >"$1"; shift; exec "$@"' _ "$WORK/pgid" \
  env -u WAYLAND_DISPLAY \
  DISPLAY="$DISP" GDK_BACKEND=x11 \
  WEBKIT_DISABLE_DMABUF_RENDERER=1 LIBGL_ALWAYS_SOFTWARE=1 \
  AIFRED_HOME="$WORK/data" AIFRED_NO_WINDOW_STATE=1 \
  $BUS npx tauri dev --config "$CONFIG" >"$LOG" 2>&1 &
for _ in $(seq 1 50); do
  [ -s "$WORK/pgid" ] && break
  sleep 0.1
done
PGID="$(cat "$WORK/pgid")"

# If this script is killed hard (for example by a tool time limit), the
# EXIT trap does not run. This watcher then stops the app and Xvfb.
setsid bash -c '
  while kill -0 "$1" 2>/dev/null; do sleep 1; done
  kill -TERM -- "-$2" 2>/dev/null; sleep 5; kill -KILL -- "-$2" 2>/dev/null
  kill "$3" 2>/dev/null; rm -rf "$4"
' _ $$ "$PGID" "$XVFB_PID" "$WORK" >/dev/null 2>&1 &

# Wait until the screen shows content. An empty screen has one color only.
shown=""
end=$((SECONDS + MAX_SECONDS))
while [ "$SECONDS" -lt "$end" ]; do
  if ! kill -0 -- "-$PGID" 2>/dev/null; then
    echo "The app stopped. Last lines of its output:" >&2
    tail -20 "$LOG" >&2
    exit 1
  fi
  if magick import -display "$DISP" -window root "$WORK/check.png" 2>/dev/null &&
    [ "$(magick "$WORK/check.png" -format '%[fx:standard_deviation>0.01]' info:)" = 1 ]; then
    shown=1
    break
  fi
  sleep 3
done

if [ -z "$shown" ]; then
  echo "The app did not show in $MAX_SECONDS seconds. Last lines of its output:" >&2
  tail -20 "$LOG" >&2
  exit 1
fi

sleep "$SETTLE"
HOLD="${AIFRED_HOLD_SECONDS:-0}"
if [ "$HOLD" -gt 0 ]; then
  echo "Holding the app for $HOLD more seconds."
  sleep "$HOLD"
  # AIFRED_KEEP_DB: save a copy of the database here (it needs sqlite3).
  if [ -n "${AIFRED_KEEP_DB:-}" ]; then
    sqlite3 "$WORK/data/aifred.db" ".backup '$AIFRED_KEEP_DB'" && echo "Copied the database to $AIFRED_KEEP_DB"
  fi
  grep -i "aifred:" "$LOG" | tail -20 || true
fi
echo "The app is shown on the hidden screen $DISP (web port $PORT)."
if [ -n "$SHOT" ]; then
  magick import -display "$DISP" -window root "$SHOT"
  echo "Screenshot: $SHOT"
fi
