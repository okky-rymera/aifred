# Plan: memory, chat history, tasks, GitHub, and ClickUp in SQLite, with word and meaning search

Date: 2026-11-04. Status: phases 1 to 5 done (branch `feat/sqlite-storage`, not committed).

## Goal

- The AI gets the context it needs without the full memory in every message.
- The AI can find facts, old chats, local tasks, GitHub items, and ClickUp tasks by words and by meaning.
- The AI can add facts to memory. Today only the command-line program can (`crates/aifred/src/main.rs:59`).
- The design must work when memory and history get large.

## Current state

- The AI has no file access. It uses only AIfred tool servers (`crates/aifred-core/src/backend/claude.rs:77`).
- Memory is one text file, `memory.md`. The app puts all of it into the AI instructions on each message (`crates/aifred-core/src/assistant.rs:111`).
- Chat history is `history.jsonl`. "New conversation" renames it to `history-<time>.jsonl`. Nothing reads the old files again (`crates/aifred-core/src/history.rs:50`).
- When Claude cannot continue a session, the app sends only the last 10 messages (`claude.rs:18`).
- The desktop app edits memory as one text box (`apps/desktop/src/App.tsx`).
- Local tasks are in `tasks.json` (`crates/aifred-core/src/tasks.rs:40`). The AI can only list all of them, add one, or mark one done (`crates/aifred-tasks-mcp/src/main.rs:61-74`). It cannot search them.
- GitHub items are saved in `github-cache.json` (`crates/aifred-core/src/github_list.rs:41`). Only the desktop app uses this copy. The AI's GitHub tools ask GitHub each time and do not read the copy (`crates/aifred-github-mcp/src/main.rs:69`).
- ClickUp tasks are saved in `clickup-cache.json` (`crates/aifred-core/src/clickup_list.rs:55`). The AI's `my_tasks` tool uses the copy while it is new enough, or downloads a new list (`crates/aifred-clickup-mcp/src/main.rs:76`).
- Both copies hold only the open items for the user. They hold the title, the place (repository, or ClickUp list and workspace), and the status. They do not hold the description (`crates/aifred-core/src/github.rs:14`, `crates/aifred-core/src/clickup.rs:113`).

## Decisions

| Item | Choice | Reason |
|---|---|---|
| Database | `rusqlite` 0.41, feature `bundled` | Builds its own SQLite. The system SQLite version does not matter. |
| Vector search | `sqlite-vec` 0.1.9, pinned with `=0.1.9` | Stable release. 0.1.10 is still alpha. It has no runtime dependencies. It loads into rusqlite with `sqlite3_auto_extension`. |
| Word search | SQLite FTS5 | Built into SQLite. Gives best match first (BM25). |
| Meaning model | fastembed 7.5, model `AllMiniLML6V2Q` | English only, as the user confirmed. Small quantized model, 384 numbers for each text. Runs on the computer. Phase 3 changed it from `BGESmallENV15Q`: see "Measurements in phase 3". |
| Database file | `~/.aifred/aifred.db` | Same data folder as today. |
| Not moved now | settings, ClickUp token | Small files. Search does not help. The token file must stay readable only by the user. |
| Items that leave the list | Keep them, marked "not in the list" | The AI can then answer "what was that ClickUp task about the invoice last month?". |

Versions were checked with `cargo info` on 2026-11-04.

### Vectors are derived data

The app can make all vectors again from the text. A change of model or a `sqlite-vec` format change does not lose data. The app deletes the vector table and fills it again.

## Database layout

```sql
-- One row for each memory fact.
CREATE TABLE memory (
  id        INTEGER PRIMARY KEY,
  text      TEXT NOT NULL,
  style     TEXT NOT NULL,               -- 'item' = a '- ' line in the memory text, 'text' = other text
  pos       INTEGER NOT NULL,            -- the order in the memory text
  core      INTEGER NOT NULL DEFAULT 1,  -- 1 = always in the AI instructions
  added_by  TEXT NOT NULL,               -- 'you' or 'aifred'
  created   INTEGER NOT NULL,
  updated   INTEGER NOT NULL,
  forgotten INTEGER                      -- time of soft delete, or NULL
);
CREATE VIRTUAL TABLE memory_fts USING fts5(text, content='memory', content_rowid='id');
CREATE VIRTUAL TABLE memory_vec USING vec0(embedding float[384] distance_metric=cosine);

CREATE TABLE conversation (
  id      INTEGER PRIMARY KEY,
  started INTEGER NOT NULL,
  ended   INTEGER,                       -- NULL = the current conversation
  summary TEXT
);
CREATE TABLE turn (
  id              INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversation(id),
  ts              INTEGER NOT NULL,
  role            TEXT NOT NULL,         -- 'user' or 'assistant'
  text            TEXT NOT NULL,
  backend         TEXT NOT NULL
);
CREATE VIRTUAL TABLE turn_fts USING fts5(text, content='turn', content_rowid='id');
CREATE VIRTUAL TABLE turn_vec USING vec0(embedding float[384] distance_metric=cosine);

-- Local tasks. The text id stays the same as in tasks.json, so the tools
-- and the desktop app do not change. The hidden rowid links the search tables.
CREATE TABLE task (
  id       TEXT PRIMARY KEY,             -- 't<nanoseconds>', as today
  title    TEXT NOT NULL,
  notes    TEXT NOT NULL DEFAULT '',
  done     INTEGER NOT NULL DEFAULT 0,
  added_by TEXT NOT NULL,                -- 'you' or 'aifred'
  created  INTEGER NOT NULL,
  done_at  INTEGER
);
CREATE VIRTUAL TABLE task_fts USING fts5(title, notes, content='task', content_rowid='rowid');
CREATE VIRTUAL TABLE task_vec USING vec0(embedding float[384] distance_metric=cosine);

-- GitHub items and ClickUp tasks, from the downloaded lists.
CREATE TABLE work_item (
  source     TEXT NOT NULL,              -- 'github' or 'clickup'
  item_id    TEXT NOT NULL,              -- the id that GitHub or ClickUp gives
  account    TEXT NOT NULL DEFAULT '',   -- ClickUp: clickup::account_key of the token
  title      TEXT NOT NULL,
  place      TEXT NOT NULL,              -- 'owner/repo', or 'workspace / list'
  url        TEXT NOT NULL,
  details    TEXT NOT NULL DEFAULT '',   -- description, saved when the AI reads it
  data       TEXT NOT NULL,              -- the full item as JSON, as the app uses it today
  in_list    INTEGER NOT NULL,           -- 1 = in the last download
  pos        INTEGER NOT NULL,           -- the order in the last download
  first_seen INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL,
  PRIMARY KEY (source, item_id)
);
CREATE VIRTUAL TABLE work_item_fts USING fts5(title, place, details, content='work_item', content_rowid='rowid');
CREATE VIRTUAL TABLE work_item_vec USING vec0(embedding float[384] distance_metric=cosine);

-- One row for each source: when the last download was, and its notes.
CREATE TABLE work_fetch (
  source     TEXT PRIMARY KEY,
  account    TEXT NOT NULL DEFAULT '',
  fetched_at INTEGER NOT NULL,
  version    INTEGER NOT NULL,           -- the VERSION in clickup_list.rs
  cut        TEXT NOT NULL DEFAULT '[]'  -- ClickUp workspaces with too many tasks
);

-- Schema version and the name of the model that made the vectors.
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
```

Phase 1 makes only the main tables (schema version 1, `crates/aifred-core/src/db.rs`). Phase 2 adds the FTS tables, and phase 3 adds the vector tables. Triggers keep the FTS tables the same as the main tables. The vector tables use the same id as the main row.

### Many programs at the same time

The desktop app, the command-line program, and the tool servers open the same file. Each connection must:

1. Set `journal_mode=WAL`. Readers then do not stop writers.
2. Set `busy_timeout=5000`. A program waits up to 5 seconds for a lock, and does not fail at once.
3. Use one transaction for each change.

A test must start two processes that write at the same time. Both writes must be in the database after the test.

## Phases

Each phase is one pull request. Each phase must pass all tests before the next one starts.

### Phase 1: SQLite for memory, history, tasks, GitHub, and ClickUp

1. Add a `db` module in `aifred-core`. It opens the database, sets the options above, and runs schema upgrades by version number.
2. Move `History` to the `turn` and `conversation` tables. Keep the same public functions: `load`, `append`, `archive`.
3. Move memory to the `memory` table. Keep `memory()`, `set_memory()`, and `remember()` for now.
4. Move `TaskStore` to the `task` table. Keep the same public functions: `load`, `add`, `set_done`, `remove`. The tasks tool server and the desktop app then need no change.
5. Move `GithubStore` and `ClickupStore` to the `work_item` and `work_fetch` tables. Keep the same public functions: `cached`, `save_fetch`, `save`, `remove`, `clear`. A new download does this:
   1. Add new items, with `in_list = 1`.
   2. Update the items that are already there, and set `last_seen`.
   3. Set `in_list = 0` for items that are not in the download. Do not delete them.
   4. The lists for the window and for `my_tasks` show only rows with `in_list = 1`. They look the same as today.
6. When the ClickUp token changes, delete all ClickUp rows. The old rows can belong to another ClickUp account. This is the same rule as `ClickupStore::clear` today.
7. Import the old files on the first start:
   - Each line in `memory.md` that starts with `- ` becomes one fact with `core = 1`. Other text becomes one fact.
   - `history.jsonl` becomes the current conversation.
   - Each `history-<time>.jsonl` becomes one ended conversation.
   - Each task in `tasks.json` becomes one row in `task`, with the same id.
   - Rename each imported file to `<name>.imported`. Do not delete it. Delete `tasks.lock`.
   - Do not import `github-cache.json` and `clickup-cache.json`. Delete them and their `.lock` files. The app downloads the lists again on the next start.
8. If the import fails, stop and show the error. Do not write a part of the data.

Result: no change for the user or the AI. All existing tests still pass.

### Phase 2: memory and history tools, word search only

1. Add a tool server, `aifred-memory-mcp`, with these tools:
   - `search_memory(query, limit)`: facts that match, best first.
   - `add_memory(text)`: adds a fact with `added_by = 'aifred'`.
   - `forget_memory(id)`: soft delete. The user can see and restore it in the app.
   - `search_history(query, limit)`: matching messages from all conversations, with the time and one message before and after each one.
2. Register the server in `tool_servers()` (`assistant.rs:134`), with the same `AIFRED_HOME` setting as the tasks server.
3. Add `search_tasks(query, include_done, limit)` to the tasks tool server (`aifred-tasks-mcp`). It searches the title and the notes, and returns matching tasks, best first. Done tasks are included only when `include_done` is true. Add the name to the allowed tools of the tasks server (`assistant.rs:151`).
4. Add `search_my_items(query, include_old, limit)` to the GitHub tool server, and `search_my_tasks(query, include_old, limit)` to the ClickUp tool server:
   - They search the title, the place, and the saved description, and return matches, best first.
   - Items that are no longer in the list show only when `include_old` is true. Each result says if it is in the list, and when AIfred last saw it.
   - If the saved list is older than the refresh time in the settings, the tool downloads a new list first. This is the same rule as `my_tasks` today.
   - The GitHub tool server then needs the data folder. Add `AIFRED_HOME` to its settings in `tool_servers()`, as for the tasks server.
   - Add the names to the allowed tools of each server.
5. When the AI reads an item with `item_details` or `task_details`, save the description in `details`. Search then gets better for the items that the AI reads.
6. Add rules to `BASE_RULES` (`assistant.rs:13`):
   - Search memory before you say that you do not know something about the user.
   - Use `search_tasks`, `search_my_items`, or `search_my_tasks` when the user asks about a topic. Use the list tools when the user asks for all open items.
   - Add a fact only when the user tells you something lasting about themselves or their work.
   - Search old chats when the user refers to an earlier conversation.
7. Change the memory editor in the desktop app from one text box to a list. Each fact has edit, delete, and a "core" switch. Forgotten facts show in a separate list with "restore".

Result: the AI can save and find facts, and find old chats, tasks, GitHub items, and ClickUp tasks.

Decisions made during phase 2:

- `add_memory` has an `always` value. The default is false: the fact is found by search, and is not in every message. Facts that the user adds in the app are always true.
- The AI instructions contain only the facts with `core = 1`. They do not contain a count of the other facts, because a count would change the instructions and stop the prompt cache.
- Word search drops common words ("the", "what", and others), and finds the start of each word ("inv" finds "invoice"). A text matches when it has any of the words. BM25 puts texts with more and rarer words first.
- In a work item search, the title counts 3 times more than the place and the description. In a task search, the title counts 2 times more than the notes.
- `search_history` cuts each message to 1,000 characters.

### Phase 3: meaning search

1. Do a short test first. Measure how long fastembed takes to load the model in a new process. The tool servers start again for each message, so this time adds to each search.
   - If the load time is less than 500 ms, the tool servers load the model when a search needs it.
   - If it is more, only the desktop app and the command-line program load the model. The tool servers use word search only.
2. Add an `embed` module in `aifred-core`. It loads the model once and makes vectors.
3. Save the model files in `~/.aifred/models/`. The first start downloads the model.
4. If the model is not available (for example, no network on the first start), use word search only. Show one notice. Do not stop the chat.
5. When a tool server adds a fact or a task, it saves it without a vector. After each answer, and after each task change in the app, the app makes the missing vectors.
   - For a task, the vector comes from the title and the notes together.
   - When a task title or notes change, delete its old vector, so the app makes a new one.
   - For a GitHub item or a ClickUp task, the vector comes from the title, the place, and the saved description.
   - After each download, the app makes vectors only for new items and for items whose title or description changed. A normal download then makes few new vectors.
6. Make the search hybrid:
   1. Get the top 20 from word search.
   2. Get the top 20 from meaning search.
   3. Merge the two lists with reciprocal rank fusion (score = sum of 1 / (60 + rank)).
   4. Return the top `limit` results.
7. For long messages, make the vector from the first 2,000 characters. The model reads at most 512 tokens.
8. Save the model name in `meta`. If the name changes, delete the vector tables and fill them again.

Measurements in phase 3 (2026-11-04, this computer):

- Model load in a new program, after the download: about 200 ms (debug build) and 150 ms (release build). One search in a new tool server program, with the load: about 150 ms. So the tool servers load the model when a search needs it.
- A test with 12 texts and 6 questions with no shared words. Correct matches in the top results: MiniLM quantized 8 of 11, BGE small (quantized or not) 7 of 11, BGE base quantized 7 of 11. The BGE search prefix made BGE worse (6 of 11). MiniLM is also the smallest download: 23 MB, against 65 MB for BGE small quantized.
- The scores of this model are low and close together. A correct match can score 0.17 and a wrong one 0.30. So there is no minimum score. The word search and the AI filter the results.
- Program size: the ONNX runtime is in each program. The release build of `aifred-tasks-mcp` went from 4.5 MB to 45 MB.

Decisions made during phase 3:

- A search never makes vectors for the saved texts. Only the app (in the background) and `aifred ask` and `aifred index` make them. So the tool servers load the model only to turn the question into a vector, and only when vectors exist.
- Triggers delete a vector when its text changes or its row is deleted. The triggers check that the text really changed, because each download writes all items again.
- The tests share one model folder, `target/test-models`. The first test run needs the network.
- WAL mode: SQLite does not wait when it turns on WAL mode and another program holds the new file. The two-process test found this. The first open now tries again for up to 5 seconds.

Changes after phase 3 (2026-11-04, after the code review):

- One meaning search helper, `aifred-embed`, now holds the model. The app, the terminal commands, and the tool servers send texts to it over a local socket. Before, each tool server loaded its own copy for each message (about 62 MB each).
- The helper stops after 5 minutes with no requests, so an idle app keeps no model in memory. A start takes about 0.2 s.
- The model is now the full-size all-MiniLM-L6-v2 (`Qdrant/all-MiniLM-L6-v2-onnx`, about 90 MB). The compressed model failed when one call had more than 32 texts ("Dynamic quantization cannot be used with batching"). So a fill of more than 32 texts failed each time. The 8 of 11 test above used the compressed model. The full-size model was not measured with that test.
- The helper reads one text at a time. 64 long texts: 4.3 s and 175 MB at the highest point, against 6.0 s and 602 MB in groups of 32.
- Program size: the release build of `aifred-tasks-mcp` is now 7.0 MB, because only `aifred-embed` (37 MB) has the ONNX runtime.

Result: "book my flight" finds "I prefer window seats". "What do I have about money?" finds the task "Send the invoice to Acme", a GitHub issue "Fix the price rounding", and a ClickUp task "Q4 budget".

### Phase 4: the app adds relevant memory to each message

1. Count the size of all facts that are not forgotten. Use 4 characters as 1 token.
2. If the total is 2,000 tokens or less, send all facts in the AI instructions, as today.
3. If the total is more:
   - The AI instructions contain the core facts only.
   - The app runs the hybrid search with the user's message and adds the top 8 other facts to the message.
4. Put the added facts in the message, after the `[Now: ...]` line. Do not put them in the AI instructions. The instructions must stay the same between messages, so Claude can reuse its saved copy (see the comment at `assistant.rs:108`).
5. Show the added facts in the desktop app, so the user can see what the AI got.

Result: memory size does not change the cost of each message much.

Decisions made during phase 4:

- "Core" is the "Always" switch from phase 2. In a large memory, the instructions hold the core facts, and the search adds only facts that are not core.
- The added facts are not saved in the chat history. The history keeps the text of the user only.
- If the search fails, the message goes without added facts, and the chat shows a notice. The AI can still search memory itself.
- The system prompt changes one time when the memory grows over the limit. This costs one message without the prompt cache.

### Phase 5: chat summaries

1. When the user starts a new conversation, make a summary of the old one. Save it in `conversation.summary`, and make a vector for it.
2. When Claude cannot continue a session, send the summary of the current conversation and the last 10 messages.
3. Make the current summary when the conversation is longer than 20,000 tokens, and again for each next 20,000 tokens.
4. `search_history` also searches summaries.

Cost: one extra AI call for each summary. A summary uses the same `claude -p` path and counts against the plan limits.

Result: a lost session keeps the main points. Old chats are easy to find.

Decisions made during phase 5:

- An ended conversation with less than 2,000 characters gets no summary. A search finds its few messages directly, and the summary would cost an AI call.
- A new summary uses the old summary and only the messages after it (`conversation.summary_upto`). Each message goes in with at most 4,000 characters.
- The summary call uses `claude -p` with no tools, no MCP servers, its own system prompt, its own folder, and `--no-session-persistence`.
- The summary runs in the background in the app, and after the answer in the command-line program. One summary runs at a time.
- `search_history` returns up to 3 matching summaries, and then the matching messages.
- Not checked with a real AI call: the tests use a fake backend.

## Later

- **Items that AIfred never downloaded.** The search finds only items that were in your list at some time. To find other items, for example closed issues from before AIfred, the tool must ask GitHub or ClickUp directly.
- **Descriptions for all items.** Today only the items that the AI reads get a description. To save all descriptions, AIfred must ask GitHub or ClickUp once for each item. Measure the time and the API limits first.
- **Other AI gaps.** These do not need SQLite: delete a task, undo "done", and mark a ClickUp task done from the AI.

## Risks

| Risk | Result | Action |
|---|---|---|
| `sqlite-vec` is before version 1.0 | A future version can change the format | Pin the version. Vectors can be made again from the text. |
| Model download fails | No meaning search | Use word search only and show a notice. |
| The model and ONNX runtime make the app bigger | Longer download for the app | Measured in phase 3: about 40 MB more for each program (release build). |
| Two programs write at the same time | "Database is locked" error | WAL, 5 second wait, and the two-process test. |
| The import from old files is wrong | Lost facts or messages | Keep the old files as `.imported`. Test with a copy of real data. |
| The AI adds too many or wrong facts | Bad memory | Rules in `BASE_RULES`. The user sees, edits, and restores facts in the app. |
| Old GitHub and ClickUp items stay forever | The database gets bigger, and old items show in search | They show only with `include_old`. Delete items that AIfred has not seen for 365 days. |
| The search uses a saved list | A result can be a little old | Each result has the time it was last seen. The tool downloads a new list when the saved one is too old. |

## Tests

- Each phase adds unit tests in `aifred-core`. Each test uses its own temporary data folder, as the current tests do.
- The import test uses example `memory.md`, `history*.jsonl`, and `tasks.json` files. Task ids must be the same after the import.
- The current task tests in `tasks.rs` and `aifred-tasks-mcp` must pass with no change, except the lines that read `tasks.json` directly.
- The search test checks that word search finds an exact name, and that meaning search finds a related word. It runs for facts, chat messages, tasks, GitHub items, and ClickUp tasks.
- A task test checks that `search_tasks` hides done tasks unless `include_done` is true.
- A download test checks that an item that leaves the list gets `in_list = 0`, stays out of the window list, and shows in search only with `include_old`.
- A ClickUp test checks that a new token deletes the rows of the old account.
- The current tests in `github_list.rs` and `clickup_list.rs` must pass, except the lines that write the JSON files directly.
- The two-process write test, from "Many programs at the same time".
- Desktop tests in `apps/desktop/src/App.test.tsx` cover the new memory list.
