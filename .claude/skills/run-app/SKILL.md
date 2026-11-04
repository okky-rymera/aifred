---
name: run-app
description: Start the AIfred desktop app to check a change in the real app, or to take a screenshot. Use this every time you start the desktop app. It runs the app on a hidden screen, so no window opens on the user's screen.
---

# Run the desktop app on a hidden screen

Do not run `npx tauri dev` directly. A window opens on the user's screen and interrupts their work.

Use this command from the repository root:

```bash
scripts/run-hidden.sh 300 <scratchpad>/app.png
```

- The first value is the longest time to wait for the app, in seconds. A first Rust build can take some minutes.
- The second value is optional. It is where the script saves a picture of the hidden screen.
- Set the Bash tool timeout to at least 60 seconds more than the first value. For 300, use 360000 ms.
- Read the picture to see the result.

What the script does:

1. It starts Xvfb (a virtual screen that nobody sees). It removes the link to the real Wayland screen.
2. It waits until the page shows content, and then 5 seconds more.
3. It takes the picture and stops the app.
4. It exits with 1 and shows the last lines of the app output if the app does not show.

The hidden copy does not touch the user's own copy of the app:

- It uses a free web port (1430 or higher). The user's copy uses 1420.
- It sets `AIFRED_NO_WINDOW_STATE=1`, so the app does not read or write the saved window size.
- It uses a temporary copy of the data folder (`~/.aifred`), and deletes it at the end.
- If the script is killed, a watcher process stops the app and Xvfb.

Limits:

- You cannot click in the hidden window. To check clicks and typing, use the window tests (`cd apps/desktop && npm test`).
- The user's own copy restarts when you edit files in `apps/desktop/src-tauri`, because `tauri dev` watches them. Tell the user before such an edit if their copy runs (`pgrep -af "tauri dev"`).
