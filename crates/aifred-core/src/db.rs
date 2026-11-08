//! The AIfred database: one SQLite file, aifred.db, in the data folder.
//!
//! The desktop app, the command-line program, and the tool servers all open
//! this file, also at the same time. So each connection:
//! - uses WAL mode: readers do not stop a writer,
//! - waits up to 5 seconds for a lock, and does not fail at once,
//! - makes each change in one transaction that takes the write lock first.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use rusqlite::{Connection, Transaction, TransactionBehavior};

pub const FILE: &str = "aifred.db";

/// The schema version that this program writes. It is in `PRAGMA user_version`.
const VERSION: i64 = 26;

const SCHEMA_1: &str = "
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

CREATE TABLE conversation (
  id      INTEGER PRIMARY KEY,
  started INTEGER NOT NULL,
  ended   INTEGER,                       -- NULL = the current conversation
  summary TEXT
);
-- Only one conversation can be the current one.
CREATE UNIQUE INDEX one_current_conversation ON conversation((ended IS NULL)) WHERE ended IS NULL;

CREATE TABLE turn (
  id              INTEGER PRIMARY KEY,
  conversation_id INTEGER NOT NULL REFERENCES conversation(id),
  ts              INTEGER NOT NULL,
  role            TEXT NOT NULL,         -- 'user' or 'assistant'
  text            TEXT NOT NULL,
  backend         TEXT NOT NULL
);
CREATE INDEX turn_by_conversation ON turn(conversation_id, id);

-- Local tasks. The text id is the same as in the old tasks.json.
CREATE TABLE task (
  id       TEXT PRIMARY KEY,             -- 't<nanoseconds>'
  title    TEXT NOT NULL,
  notes    TEXT NOT NULL DEFAULT '',
  done     INTEGER NOT NULL DEFAULT 0,
  added_by TEXT NOT NULL,                -- 'you' or 'aifred'
  created  INTEGER NOT NULL,
  done_at  INTEGER
);

-- GitHub items and ClickUp tasks, from the downloaded lists. An item that
-- leaves the list stays, with in_list = 0.
CREATE TABLE work_item (
  source     TEXT NOT NULL,              -- 'github' or 'clickup'
  item_id    TEXT NOT NULL,              -- the id that GitHub or ClickUp gives
  account    TEXT NOT NULL DEFAULT '',   -- ClickUp: clickup::account_key of the token
  title      TEXT NOT NULL,
  place      TEXT NOT NULL,              -- 'owner/repo', or 'workspace / list'
  url        TEXT NOT NULL,
  details    TEXT NOT NULL DEFAULT '',   -- the description, saved when the AI reads it
  data       TEXT NOT NULL,              -- the full item as JSON, as the app uses it
  in_list    INTEGER NOT NULL,           -- 1 = in the last download
  pos        INTEGER NOT NULL,           -- the order in the last download
  first_seen INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL,
  PRIMARY KEY (source, item_id)
);

-- One row for each source: when the last download was, and its notes.
CREATE TABLE work_fetch (
  source     TEXT PRIMARY KEY,
  account    TEXT NOT NULL DEFAULT '',
  fetched_at INTEGER NOT NULL,
  version    INTEGER NOT NULL,           -- the VERSION in clickup_list.rs
  cut        TEXT NOT NULL DEFAULT '[]'  -- ClickUp workspaces with too many tasks, as JSON
);

-- Settings of the database, for example the name of the model that made the vectors.
CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT NOT NULL);
";

/// Word search (FTS5). Triggers keep each search table the same as its main
/// table. "porter" finds other forms of a word: "invoices" finds "invoice".
const SCHEMA_2: &str = "
CREATE VIRTUAL TABLE memory_fts USING fts5(text, content='memory', content_rowid='id', tokenize='porter unicode61 remove_diacritics 2');
CREATE TRIGGER memory_fts_insert AFTER INSERT ON memory BEGIN
  INSERT INTO memory_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER memory_fts_delete AFTER DELETE ON memory BEGIN
  INSERT INTO memory_fts (memory_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER memory_fts_update AFTER UPDATE OF text ON memory BEGIN
  INSERT INTO memory_fts (memory_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO memory_fts (rowid, text) VALUES (new.id, new.text);
END;
INSERT INTO memory_fts (memory_fts) VALUES ('rebuild');

CREATE VIRTUAL TABLE turn_fts USING fts5(text, content='turn', content_rowid='id', tokenize='porter unicode61 remove_diacritics 2');
CREATE TRIGGER turn_fts_insert AFTER INSERT ON turn BEGIN
  INSERT INTO turn_fts (rowid, text) VALUES (new.id, new.text);
END;
CREATE TRIGGER turn_fts_delete AFTER DELETE ON turn BEGIN
  INSERT INTO turn_fts (turn_fts, rowid, text) VALUES ('delete', old.id, old.text);
END;
CREATE TRIGGER turn_fts_update AFTER UPDATE OF text ON turn BEGIN
  INSERT INTO turn_fts (turn_fts, rowid, text) VALUES ('delete', old.id, old.text);
  INSERT INTO turn_fts (rowid, text) VALUES (new.id, new.text);
END;
INSERT INTO turn_fts (turn_fts) VALUES ('rebuild');

CREATE VIRTUAL TABLE task_fts USING fts5(title, notes, content='task', content_rowid='rowid', tokenize='porter unicode61 remove_diacritics 2');
CREATE TRIGGER task_fts_insert AFTER INSERT ON task BEGIN
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
CREATE TRIGGER task_fts_delete AFTER DELETE ON task BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
END;
CREATE TRIGGER task_fts_update AFTER UPDATE OF title, notes ON task BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
INSERT INTO task_fts (task_fts) VALUES ('rebuild');

CREATE VIRTUAL TABLE work_item_fts USING fts5(title, place, details, content='work_item', content_rowid='rowid', tokenize='porter unicode61 remove_diacritics 2');
CREATE TRIGGER work_item_fts_insert AFTER INSERT ON work_item BEGIN
  INSERT INTO work_item_fts (rowid, title, place, details) VALUES (new.rowid, new.title, new.place, new.details);
END;
CREATE TRIGGER work_item_fts_delete AFTER DELETE ON work_item BEGIN
  INSERT INTO work_item_fts (work_item_fts, rowid, title, place, details) VALUES ('delete', old.rowid, old.title, old.place, old.details);
END;
CREATE TRIGGER work_item_fts_update AFTER UPDATE OF title, place, details ON work_item BEGIN
  INSERT INTO work_item_fts (work_item_fts, rowid, title, place, details) VALUES ('delete', old.rowid, old.title, old.place, old.details);
  INSERT INTO work_item_fts (rowid, title, place, details) VALUES (new.rowid, new.title, new.place, new.details);
END;
INSERT INTO work_item_fts (work_item_fts) VALUES ('rebuild');
";

/// Meaning search (sqlite-vec). Each vector has the rowid of its main row.
/// When a text changes or its row goes, a trigger deletes the vector, and
/// embed::fill_missing makes a new one. The WHEN parts matter: a download
/// writes every item again, also when nothing changed.
const SCHEMA_3: &str = "
CREATE VIRTUAL TABLE memory_vec USING vec0(embedding float[384] distance_metric=cosine);
CREATE TRIGGER memory_vec_delete AFTER DELETE ON memory BEGIN
  DELETE FROM memory_vec WHERE rowid = old.id;
END;
CREATE TRIGGER memory_vec_update AFTER UPDATE OF text ON memory WHEN old.text IS NOT new.text BEGIN
  DELETE FROM memory_vec WHERE rowid = old.id;
END;

CREATE VIRTUAL TABLE turn_vec USING vec0(embedding float[384] distance_metric=cosine);
CREATE TRIGGER turn_vec_delete AFTER DELETE ON turn BEGIN
  DELETE FROM turn_vec WHERE rowid = old.id;
END;

CREATE VIRTUAL TABLE task_vec USING vec0(embedding float[384] distance_metric=cosine);
CREATE TRIGGER task_vec_delete AFTER DELETE ON task BEGIN
  DELETE FROM task_vec WHERE rowid = old.rowid;
END;
CREATE TRIGGER task_vec_update AFTER UPDATE OF title, notes ON task
  WHEN old.title IS NOT new.title OR old.notes IS NOT new.notes BEGIN
  DELETE FROM task_vec WHERE rowid = old.rowid;
END;

CREATE VIRTUAL TABLE work_item_vec USING vec0(embedding float[384] distance_metric=cosine);
CREATE TRIGGER work_item_vec_delete AFTER DELETE ON work_item BEGIN
  DELETE FROM work_item_vec WHERE rowid = old.rowid;
END;
CREATE TRIGGER work_item_vec_update AFTER UPDATE OF title, place, details ON work_item
  WHEN old.title IS NOT new.title OR old.place IS NOT new.place OR old.details IS NOT new.details BEGIN
  DELETE FROM work_item_vec WHERE rowid = old.rowid;
END;
";

/// Chat summaries (assistant.rs): word and meaning search for them, and the
/// last message that each summary covers.
const SCHEMA_4: &str = "
ALTER TABLE conversation ADD COLUMN summary_upto INTEGER;  -- the id of the last turn in the summary

CREATE VIRTUAL TABLE conversation_fts USING fts5(summary, content='conversation', content_rowid='id', tokenize='porter unicode61 remove_diacritics 2');
CREATE TRIGGER conversation_fts_insert AFTER INSERT ON conversation BEGIN
  INSERT INTO conversation_fts (rowid, summary) VALUES (new.id, new.summary);
END;
CREATE TRIGGER conversation_fts_delete AFTER DELETE ON conversation BEGIN
  INSERT INTO conversation_fts (conversation_fts, rowid, summary) VALUES ('delete', old.id, old.summary);
END;
CREATE TRIGGER conversation_fts_update AFTER UPDATE OF summary ON conversation BEGIN
  INSERT INTO conversation_fts (conversation_fts, rowid, summary) VALUES ('delete', old.id, old.summary);
  INSERT INTO conversation_fts (rowid, summary) VALUES (new.id, new.summary);
END;
INSERT INTO conversation_fts (conversation_fts) VALUES ('rebuild');

CREATE VIRTUAL TABLE conversation_vec USING vec0(embedding float[384] distance_metric=cosine);
CREATE TRIGGER conversation_vec_delete AFTER DELETE ON conversation BEGIN
  DELETE FROM conversation_vec WHERE rowid = old.id;
END;
CREATE TRIGGER conversation_vec_update AFTER UPDATE OF summary ON conversation WHEN old.summary IS NOT new.summary BEGIN
  DELETE FROM conversation_vec WHERE rowid = old.id;
END;
";

/// Short summaries of long descriptions (summaries.rs). The overview tool
/// sends them, so the AI does not read every description. A summary goes
/// back to NULL when its title or description changes, so it is made again.
const SCHEMA_5: &str = "
ALTER TABLE work_item ADD COLUMN summary TEXT;  -- a short summary of the details, or NULL
ALTER TABLE task ADD COLUMN summary TEXT;       -- a short summary of the notes, or NULL

CREATE TRIGGER work_item_summary_old AFTER UPDATE OF title, details ON work_item
  WHEN old.title IS NOT new.title OR old.details IS NOT new.details BEGIN
  UPDATE work_item SET summary = NULL WHERE rowid = new.rowid;
END;
CREATE TRIGGER task_summary_old AFTER UPDATE OF title, notes ON task
  WHEN old.title IS NOT new.title OR old.notes IS NOT new.notes BEGIN
  UPDATE task SET summary = NULL WHERE rowid = new.rowid;
END;
";

/// A fixed row number for tasks and items. Word search, meaning search and
/// summaries find a task or item by its rowid. Without an INTEGER PRIMARY KEY,
/// VACUUM can give the rows new rowids, and the search tables then point to
/// the wrong rows. So the new `rid` column is the rowid, and VACUUM keeps it.
/// Each row keeps its old rowid. DROP TABLE drops the old triggers, so they
/// are made again here, the same as in SCHEMA_2, SCHEMA_3 and SCHEMA_5.
const SCHEMA_6: &str = "
CREATE TABLE task_new (
  rid      INTEGER PRIMARY KEY,
  id       TEXT NOT NULL UNIQUE,
  title    TEXT NOT NULL,
  notes    TEXT NOT NULL DEFAULT '',
  done     INTEGER NOT NULL DEFAULT 0,
  added_by TEXT NOT NULL,
  created  INTEGER NOT NULL,
  done_at  INTEGER,
  summary  TEXT
);
INSERT INTO task_new (rid, id, title, notes, done, added_by, created, done_at, summary)
  SELECT rowid, id, title, notes, done, added_by, created, done_at, summary FROM task;
DROP TABLE task;
ALTER TABLE task_new RENAME TO task;

CREATE TABLE work_item_new (
  rid        INTEGER PRIMARY KEY,
  source     TEXT NOT NULL,
  item_id    TEXT NOT NULL,
  account    TEXT NOT NULL DEFAULT '',
  title      TEXT NOT NULL,
  place      TEXT NOT NULL,
  url        TEXT NOT NULL,
  details    TEXT NOT NULL DEFAULT '',
  data       TEXT NOT NULL,
  in_list    INTEGER NOT NULL,
  pos        INTEGER NOT NULL,
  first_seen INTEGER NOT NULL,
  last_seen  INTEGER NOT NULL,
  summary    TEXT,
  UNIQUE (source, item_id)
);
INSERT INTO work_item_new (rid, source, item_id, account, title, place, url, details, data, in_list, pos, first_seen, last_seen, summary)
  SELECT rowid, source, item_id, account, title, place, url, details, data, in_list, pos, first_seen, last_seen, summary FROM work_item;
DROP TABLE work_item;
ALTER TABLE work_item_new RENAME TO work_item;

CREATE TRIGGER task_fts_insert AFTER INSERT ON task BEGIN
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
CREATE TRIGGER task_fts_delete AFTER DELETE ON task BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
END;
CREATE TRIGGER task_fts_update AFTER UPDATE OF title, notes ON task BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
CREATE TRIGGER work_item_fts_insert AFTER INSERT ON work_item BEGIN
  INSERT INTO work_item_fts (rowid, title, place, details) VALUES (new.rowid, new.title, new.place, new.details);
END;
CREATE TRIGGER work_item_fts_delete AFTER DELETE ON work_item BEGIN
  INSERT INTO work_item_fts (work_item_fts, rowid, title, place, details) VALUES ('delete', old.rowid, old.title, old.place, old.details);
END;
CREATE TRIGGER work_item_fts_update AFTER UPDATE OF title, place, details ON work_item BEGIN
  INSERT INTO work_item_fts (work_item_fts, rowid, title, place, details) VALUES ('delete', old.rowid, old.title, old.place, old.details);
  INSERT INTO work_item_fts (rowid, title, place, details) VALUES (new.rowid, new.title, new.place, new.details);
END;
CREATE TRIGGER task_vec_delete AFTER DELETE ON task BEGIN
  DELETE FROM task_vec WHERE rowid = old.rowid;
END;
CREATE TRIGGER task_vec_update AFTER UPDATE OF title, notes ON task
  WHEN old.title IS NOT new.title OR old.notes IS NOT new.notes BEGIN
  DELETE FROM task_vec WHERE rowid = old.rowid;
END;
CREATE TRIGGER work_item_vec_delete AFTER DELETE ON work_item BEGIN
  DELETE FROM work_item_vec WHERE rowid = old.rowid;
END;
CREATE TRIGGER work_item_vec_update AFTER UPDATE OF title, place, details ON work_item
  WHEN old.title IS NOT new.title OR old.place IS NOT new.place OR old.details IS NOT new.details BEGIN
  DELETE FROM work_item_vec WHERE rowid = old.rowid;
END;
CREATE TRIGGER work_item_summary_old AFTER UPDATE OF title, details ON work_item
  WHEN old.title IS NOT new.title OR old.details IS NOT new.details BEGIN
  UPDATE work_item SET summary = NULL WHERE rowid = new.rowid;
END;
CREATE TRIGGER task_summary_old AFTER UPDATE OF title, notes ON task
  WHEN old.title IS NOT new.title OR old.notes IS NOT new.notes BEGIN
  UPDATE task SET summary = NULL WHERE rowid = new.rowid;
END;
";

/// Word search: change the index only when the text changes. A download
/// writes every item again, also when nothing changed. Without the WHEN part,
/// each item gets a delete and an insert in the index on each download.
const SCHEMA_7: &str = "
DROP TRIGGER task_fts_update;
CREATE TRIGGER task_fts_update AFTER UPDATE OF title, notes ON task
  WHEN old.title IS NOT new.title OR old.notes IS NOT new.notes BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
DROP TRIGGER work_item_fts_update;
CREATE TRIGGER work_item_fts_update AFTER UPDATE OF title, place, details ON work_item
  WHEN old.title IS NOT new.title OR old.place IS NOT new.place OR old.details IS NOT new.details BEGIN
  INSERT INTO work_item_fts (work_item_fts, rowid, title, place, details) VALUES ('delete', old.rowid, old.title, old.place, old.details);
  INSERT INTO work_item_fts (rowid, title, place, details) VALUES (new.rowid, new.title, new.place, new.details);
END;
";

/// More fields on local tasks (tasks.rs). Dates are "YYYY-MM-DD" in the
/// user's own time zone, or NULL. Tags and the checklist are JSON lists.
const SCHEMA_8: &str = "
ALTER TABLE task ADD COLUMN due       TEXT;                       -- the day the task must be done
ALTER TABLE task ADD COLUMN start     TEXT;                       -- the first day to work on it
ALTER TABLE task ADD COLUMN priority  TEXT;                       -- 'urgent', 'high', 'normal', 'low', or NULL
ALTER TABLE task ADD COLUMN tags      TEXT NOT NULL DEFAULT '[]'; -- [\"work\", ...]
ALTER TABLE task ADD COLUMN checklist TEXT NOT NULL DEFAULT '[]'; -- [{\"text\": ..., \"done\": false}, ...]
";

/// The day plan, time guesses, the daily brief, the side chats, and files.
/// An item key names one work item in every source: "task:<id>",
/// "github:<id>", or "clickup:<id>" (items.rs).
const SCHEMA_9: &str = "
-- One plan for each day: the items in the order of the day.
CREATE TABLE day_plan (
  day     TEXT PRIMARY KEY,              -- 'YYYY-MM-DD' in the user's own time zone
  mode    TEXT NOT NULL,                 -- 'ai' = AIfred made it, 'self' = the user made it
  items   TEXT NOT NULL DEFAULT '[]',    -- [{\"key\": ..., \"why\": ...}], first item first
  updated INTEGER NOT NULL
);

-- How long an item takes, in minutes.
CREATE TABLE estimate (
  key     TEXT PRIMARY KEY,              -- an item key
  minutes INTEGER NOT NULL,
  by      TEXT NOT NULL                  -- 'you' or 'aifred'
);

-- The brief of each day: a short text from AIfred, and a reason for each item it names.
CREATE TABLE brief (
  day     TEXT PRIMARY KEY,
  written INTEGER NOT NULL,
  text    TEXT NOT NULL,
  why     TEXT NOT NULL DEFAULT '{}'     -- {\"<item key>\": \"<reason>\"}
);

-- Chats apart from the main conversation: the plan chat of a day, and the chat of one item.
CREATE TABLE side_turn (
  id     INTEGER PRIMARY KEY,
  thread TEXT NOT NULL,                  -- 'plan:YYYY-MM-DD' or 'item:<item key>'
  ts     INTEGER NOT NULL,
  role   TEXT NOT NULL,                  -- 'user' or 'assistant'
  text   TEXT NOT NULL,
  meta   TEXT NOT NULL DEFAULT '{}'      -- {\"model\": ..., \"files\": [...], \"info\": ...}
);
CREATE INDEX side_turn_by_thread ON side_turn(thread, id);

-- Files that the user sent with a message, or keeps on a task (uploads.rs).
ALTER TABLE turn ADD COLUMN files TEXT NOT NULL DEFAULT '[]';
ALTER TABLE task ADD COLUMN files TEXT NOT NULL DEFAULT '[]';
";

/// AIfred's own jobs (jobs.rs), their runs, and the events that start them.
const SCHEMA_10: &str = "
CREATE TABLE job (
  id         TEXT PRIMARY KEY,
  title      TEXT NOT NULL,
  action     TEXT NOT NULL,              -- 'remind', 'agent', or 'brief'
  message    TEXT NOT NULL DEFAULT '',   -- the reminder text, or the instructions for the AI
  trigger    TEXT NOT NULL,              -- {\"kind\": \"once\" | \"repeat\" | \"event\", ...}
  tz         TEXT NOT NULL,              -- IANA time zone, for example 'Asia/Jakarta'
  status     TEXT NOT NULL,              -- 'active', 'paused', 'done', or 'off'
  next_run   INTEGER,                    -- Unix seconds, or NULL
  created_by TEXT NOT NULL,              -- 'you' or 'aifred'
  created    INTEGER NOT NULL,
  updated    INTEGER NOT NULL,
  failures   INTEGER NOT NULL DEFAULT 0, -- failed runs in a row
  cursor     INTEGER NOT NULL DEFAULT 0  -- event jobs: the last job_event id that they used
);
CREATE INDEX job_by_next ON job(status, next_run);

-- One row for each run, also for a time that was skipped. A time runs at most once.
CREATE TABLE job_run (
  id       INTEGER PRIMARY KEY,
  job      TEXT NOT NULL REFERENCES job(id) ON DELETE CASCADE,
  planned  INTEGER NOT NULL,
  started  INTEGER,
  finished INTEGER,
  state    TEXT NOT NULL,                -- 'running', 'ok', 'quiet', 'skipped', or 'failed'
  late     INTEGER NOT NULL DEFAULT 0,
  note     TEXT NOT NULL DEFAULT '',     -- why it was skipped, or the error
  output   TEXT NOT NULL DEFAULT '',
  cost_usd REAL,
  UNIQUE (job, planned)
);
CREATE INDEX job_run_by_job ON job_run(job, id);

-- Events from all AIfred programs, for example 'task_done'.
CREATE TABLE job_event (
  id   INTEGER PRIMARY KEY,
  name TEXT NOT NULL,
  ts   INTEGER NOT NULL
);
CREATE INDEX job_event_by_name ON job_event(name, id);

-- AIfred writes the brief again when the user finishes tasks.
INSERT INTO job (id, title, action, message, trigger, tz, status, created_by, created, updated)
VALUES ('brief-on-task-done', 'Update the brief when you finish a task', 'brief', '',
        '{\"kind\":\"event\",\"event\":\"task_done\",\"wait_secs\":90}', 'UTC', 'active', 'aifred',
        CAST(strftime('%s','now') AS INTEGER), CAST(strftime('%s','now') AS INTEGER));
";

/// The user's goals (goals.rs), and the goal of each item in the brief.
/// A removed goal's id is never used again (AUTOINCREMENT). The brief and
/// the plan keep goal ids: an old id must not point at a new goal. The
/// list is in id order.
const SCHEMA_11: &str = "
CREATE TABLE goal (
  id      INTEGER PRIMARY KEY AUTOINCREMENT,
  text    TEXT NOT NULL,
  created INTEGER NOT NULL,
  updated INTEGER NOT NULL
);

ALTER TABLE brief ADD COLUMN goal TEXT NOT NULL DEFAULT '{}'; -- {\"<item key>\": <goal id>}
";

/// The "waiting for" list (waiting.rs), and the email threads that the
/// user hid (gmail.rs).
const SCHEMA_12: &str = "
CREATE TABLE waiting (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  what      TEXT NOT NULL,
  who       TEXT NOT NULL,
  since     TEXT NOT NULL,              -- YYYY-MM-DD
  follow_up TEXT,                       -- YYYY-MM-DD: tell the user on this day when there is no answer
  told      INTEGER NOT NULL DEFAULT 0, -- 1 after the notification of follow_up
  thread    TEXT,                       -- the Gmail thread, when the item came from an email
  sent_at   INTEGER,                    -- the user's email in the thread, Unix seconds
  url       TEXT,
  done      INTEGER NOT NULL DEFAULT 0,
  done_why  TEXT NOT NULL DEFAULT '',
  created   INTEGER NOT NULL,
  updated   INTEGER NOT NULL
);
CREATE INDEX waiting_by_thread ON waiting(thread);

-- A hidden thread stays hidden until a message newer than last_at comes.
CREATE TABLE mail_hidden (
  thread    TEXT PRIMARY KEY,
  last_at   INTEGER NOT NULL,
  hidden_at INTEGER NOT NULL
);
";

/// The people that an email item waits for.
const SCHEMA_13: &str = "
ALTER TABLE waiting ADD COLUMN wait_for TEXT NOT NULL DEFAULT ''; -- ,ana@x.com,bo@y.com, or '': anyone
";

/// What AIfred learns from the user (learn.rs, worktime.rs): the user's
/// corrections of the brief, the memory facts that came from them, and the
/// real time of the work on each item.
const SCHEMA_14: &str = "
-- The user says that the brief is wrong about an item.
CREATE TABLE correction (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  key   TEXT NOT NULL,                -- the item key
  kind  TEXT NOT NULL,                -- 'not_today' or 'wrong_priority'
  day   TEXT NOT NULL,                -- YYYY-MM-DD: the day of the brief
  title TEXT NOT NULL,
  place TEXT NOT NULL DEFAULT '',     -- where the item is from, for example 'GitHub acme/docs', or ''
  ts    INTEGER NOT NULL
);
CREATE INDEX correction_by_day ON correction(day);
CREATE INDEX correction_by_place ON correction(place, kind);

-- One memory fact for each place that AIfred learned from corrections.
-- The row stays when the user forgets the fact: AIfred learns a place once.
CREATE TABLE learned (
  place      TEXT PRIMARY KEY,
  fact       INTEGER NOT NULL,        -- memory.id
  correction INTEGER NOT NULL,        -- the correction that made the fact
  ts         INTEGER NOT NULL
);

-- The user works on one item at a time.
CREATE TABLE work_session (
  id      INTEGER PRIMARY KEY,
  key     TEXT NOT NULL,
  started INTEGER NOT NULL,
  ended   INTEGER                     -- NULL while the user works on the item
);
CREATE INDEX work_session_by_key ON work_session(key);
CREATE UNIQUE INDEX one_running_session ON work_session((ended IS NULL)) WHERE ended IS NULL;

-- A finished item: the real time of the work next to the time guess.
CREATE TABLE work_done (
  key      TEXT PRIMARY KEY,
  title    TEXT NOT NULL,
  place    TEXT NOT NULL DEFAULT '',
  guess    INTEGER,                   -- the time guess in minutes, or NULL
  guess_by TEXT,                      -- 'you' or 'aifred', or NULL
  minutes  INTEGER NOT NULL,          -- all sessions together
  finished INTEGER NOT NULL
);
";

/// The reviews (review.rs): the end-of-day check, the weekly review, the
/// user's pick for each item in the weekly review, and the last change of
/// each local task.
const SCHEMA_15: &str = "
-- One row for each day that the user finished the end-of-day check.
CREATE TABLE day_review (
  day   TEXT PRIMARY KEY,             -- YYYY-MM-DD
  moved TEXT NOT NULL DEFAULT '[]',   -- the item keys that went to the next day's plan
  ts    INTEGER NOT NULL
);

-- One row for each weekly review that the user finished.
CREATE TABLE week_review (
  day TEXT PRIMARY KEY,               -- YYYY-MM-DD: the day of the review
  ts  INTEGER NOT NULL
);

-- What the user picked for an item in the weekly review.
CREATE TABLE review_pick (
  id    INTEGER PRIMARY KEY AUTOINCREMENT,
  key   TEXT NOT NULL,                -- the item key
  pick  TEXT NOT NULL,                -- 'do', 'plan', 'delegate', or 'drop'
  title TEXT NOT NULL,
  place TEXT NOT NULL DEFAULT '',
  note  TEXT NOT NULL DEFAULT '',     -- plan: the day; delegate: the person
  undo  TEXT NOT NULL DEFAULT '{}',   -- what the undo puts back
  ts    INTEGER NOT NULL
);
CREATE INDEX review_pick_by_key ON review_pick(key, ts);

-- The last change of a local task, in Unix seconds. 0: no change after it was added.
ALTER TABLE task ADD COLUMN updated INTEGER NOT NULL DEFAULT 0;
-- The old versions did not keep the last change. A task can have changed
-- yesterday, so the upgrade counts as a change of each task. Else the first
-- weekly review shows active tasks as stale.
UPDATE task SET updated = CAST(strftime('%s', 'now') AS INTEGER);
";

/// Subtasks (links.rs; one level only before SCHEMA_22): the items that show under
/// another item, and the pairs that the user said are not one piece of work.
const SCHEMA_16: &str = "
CREATE TABLE item_link (
  key  TEXT PRIMARY KEY,              -- the item key of the item under the main item
  main TEXT NOT NULL,                 -- the item key of the main item; never itself in key
  ts   INTEGER NOT NULL
);
CREATE INDEX item_link_by_main ON item_link(main);

-- A suggestion that the user rejected. a < b.
CREATE TABLE link_no (
  a  TEXT NOT NULL,
  b  TEXT NOT NULL,
  ts INTEGER NOT NULL,
  PRIMARY KEY (a, b)
);
";

/// The next step of each item in the brief, and the steps that AIfred can
/// draft later (brief.rs).
const SCHEMA_17: &str = "
ALTER TABLE brief ADD COLUMN next  TEXT NOT NULL DEFAULT '{}'; -- {\"<item key>\": \"Reply to Ana\"}
ALTER TABLE brief ADD COLUMN draft TEXT NOT NULL DEFAULT '{}'; -- {\"<item key>\": \"email\" | \"doc\" | \"sheet\"}
";

/// The log of AIfred's actions in other apps (actions.rs).
const SCHEMA_18: &str = "
CREATE TABLE action (
  id        INTEGER PRIMARY KEY,
  at        INTEGER NOT NULL,           -- Unix seconds
  app       TEXT NOT NULL,              -- 'gmail'
  kind      TEXT NOT NULL,              -- 'draft'
  item      TEXT NOT NULL,              -- the item key, for example 'gmail:<thread>'
  title     TEXT NOT NULL,              -- what AIfred did, in one line
  after     TEXT NOT NULL,              -- the new text, as the user saw it
  target    TEXT NOT NULL,              -- JSON: the ids in the other app, for Undo
  url       TEXT NOT NULL,
  undone_at INTEGER,                    -- NULL: not undone
  note      TEXT NOT NULL DEFAULT ''    -- why Undo did not delete it, for example
);
CREATE INDEX action_by_item ON action(item);
";

/// One task type (plan-tasks-missions-calendar.md, decision A): AIfred's
/// jobs move into the task table with doer 'aifred'. The user's tasks get
/// doer 'you'. The job fields stay NULL on them. The reads of the user's
/// tasks must filter on doer = 'you'.
///
/// The order is necessary. foreign_keys is on, so a DROP of `job` while
/// `job_run` points at it deletes every run. So the rows go into `task`
/// first, then `job_run` points at `task`, and only then `job` goes.
/// The job message stays out of `notes`: the task triggers index and
/// summarize the notes.
const SCHEMA_19: &str = "
ALTER TABLE task ADD COLUMN doer     TEXT NOT NULL DEFAULT 'you'; -- 'you' or 'aifred'
ALTER TABLE task ADD COLUMN action   TEXT;                        -- AIfred: 'remind', 'agent', or 'brief'
ALTER TABLE task ADD COLUMN message  TEXT NOT NULL DEFAULT '';    -- AIfred: the reminder text, or the instructions for the AI
ALTER TABLE task ADD COLUMN trigger  TEXT;                        -- {\"kind\": \"once\" | \"repeat\" | \"event\", ...}
ALTER TABLE task ADD COLUMN tz       TEXT;                        -- IANA time zone of the trigger
ALTER TABLE task ADD COLUMN status   TEXT;                        -- with a trigger: 'active', 'paused', 'done', or 'off'
ALTER TABLE task ADD COLUMN next_run INTEGER;                     -- Unix seconds, or NULL
ALTER TABLE task ADD COLUMN failures INTEGER NOT NULL DEFAULT 0;  -- failed runs in a row
ALTER TABLE task ADD COLUMN cursor   INTEGER NOT NULL DEFAULT 0;  -- event triggers: the last job_event id that they used

-- The user's tasks only. Reads of the user's tasks use this view, so they
-- cannot get AIfred's rows by mistake. A view has no rowid: use rid.
-- Writes go to `task` and must still filter on doer = 'you'. db::write
-- stops a write that touches a job row (GUARD_JOBS).
CREATE VIEW user_task AS SELECT * FROM task WHERE doer = 'you';

-- Word search, meaning search and summaries are for the user's tasks only.
-- The doer of a row never changes. All three search triggers must skip the
-- same rows: an FTS 'delete' of a row that is not in the index breaks it.
-- These come before the copy of the jobs, so the jobs are never indexed.
DROP TRIGGER task_fts_insert;
CREATE TRIGGER task_fts_insert AFTER INSERT ON task WHEN new.doer = 'you' BEGIN
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
DROP TRIGGER task_fts_delete;
CREATE TRIGGER task_fts_delete AFTER DELETE ON task WHEN old.doer = 'you' BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
END;
DROP TRIGGER task_fts_update;
CREATE TRIGGER task_fts_update AFTER UPDATE OF title, notes ON task
  WHEN new.doer = 'you' AND (old.title IS NOT new.title OR old.notes IS NOT new.notes) BEGIN
  INSERT INTO task_fts (task_fts, rowid, title, notes) VALUES ('delete', old.rowid, old.title, old.notes);
  INSERT INTO task_fts (rowid, title, notes) VALUES (new.rowid, new.title, new.notes);
END;
DROP TRIGGER task_vec_delete;
CREATE TRIGGER task_vec_delete AFTER DELETE ON task WHEN old.doer = 'you' BEGIN
  DELETE FROM task_vec WHERE rowid = old.rowid;
END;
DROP TRIGGER task_vec_update;
CREATE TRIGGER task_vec_update AFTER UPDATE OF title, notes ON task
  WHEN new.doer = 'you' AND (old.title IS NOT new.title OR old.notes IS NOT new.notes) BEGIN
  DELETE FROM task_vec WHERE rowid = old.rowid;
END;
DROP TRIGGER task_summary_old;
CREATE TRIGGER task_summary_old AFTER UPDATE OF title, notes ON task
  WHEN new.doer = 'you' AND (old.title IS NOT new.title OR old.notes IS NOT new.notes) BEGIN
  UPDATE task SET summary = NULL WHERE rowid = new.rowid;
END;

INSERT INTO task (id, title, notes, done, added_by, created, done_at, updated,
                  doer, action, message, trigger, tz, status, next_run, failures, cursor)
  SELECT id, title, '', status = 'done', created_by, created,
         CASE WHEN status = 'done' THEN updated END, updated,
         'aifred', action, message, trigger, tz, status, next_run, failures, cursor
  FROM job;

CREATE TABLE job_run_new (
  id       INTEGER PRIMARY KEY,
  job      TEXT NOT NULL REFERENCES task(id) ON DELETE CASCADE,
  planned  INTEGER NOT NULL,
  started  INTEGER,
  finished INTEGER,
  state    TEXT NOT NULL,                -- 'running', 'ok', 'quiet', 'skipped', or 'failed'
  late     INTEGER NOT NULL DEFAULT 0,
  note     TEXT NOT NULL DEFAULT '',     -- why it was skipped, or the error
  output   TEXT NOT NULL DEFAULT '',
  cost_usd REAL,
  UNIQUE (job, planned)
);
INSERT INTO job_run_new (id, job, planned, started, finished, state, late, note, output, cost_usd)
  SELECT id, job, planned, started, finished, state, late, note, output, cost_usd FROM job_run;
DROP TABLE job_run;
ALTER TABLE job_run_new RENAME TO job_run;
CREATE INDEX job_run_by_job ON job_run(job, id);
DROP TABLE job;

CREATE INDEX task_by_next ON task(doer, status, next_run);

-- An AIfred task with the status 'done' is done, the same as a user task.
CREATE TRIGGER task_aifred_done AFTER UPDATE OF status ON task
  WHEN new.doer = 'aifred' AND (new.status = 'done') IS NOT new.done BEGIN
  UPDATE task SET done = (new.status = 'done'),
                  done_at = CASE WHEN new.status = 'done' THEN CAST(strftime('%s', 'now') AS INTEGER) END
  WHERE rowid = new.rowid;
END;
";

/// Missions (missions.rs): units of work that the brief makes from one or
/// more items. `mission_item` gives each item at most one mission.
const SCHEMA_20: &str = "
CREATE TABLE mission (
  id        INTEGER PRIMARY KEY AUTOINCREMENT,
  title     TEXT NOT NULL,
  doer      TEXT NOT NULL DEFAULT 'you',  -- 'you' or 'aifred'
  checklist TEXT NOT NULL DEFAULT '[]',   -- [{\"text\": \"...\", \"done\": false}]
  day       TEXT,                         -- the day of the last brief that ranked it
  rank      INTEGER,                      -- 1 is first in that brief; NULL: not ranked
  status    TEXT NOT NULL DEFAULT 'open', -- 'open' or 'done'
  created   INTEGER NOT NULL,
  updated   INTEGER NOT NULL
);
CREATE INDEX mission_by_day ON mission(day, rank);

CREATE TABLE mission_item (
  key     TEXT PRIMARY KEY,               -- an item key: one item is in at most one mission
  mission INTEGER NOT NULL REFERENCES mission(id) ON DELETE CASCADE,
  pos     INTEGER NOT NULL                -- 0 is the item that the next action is about
);
CREATE INDEX mission_item_by_mission ON mission_item(mission, pos);
";

/// The "Same work?" card is gone (plan part 3b). So are the pairs that the
/// user rejected in it: nothing blocks the brief for good.
const SCHEMA_21: &str = "
DROP TABLE link_no;
";

/// Subtasks (links.rs, plan part 3b): a row of `item_link` puts an item
/// under its parent, with many levels. The old links were made by the user.
const SCHEMA_22: &str = "
ALTER TABLE item_link ADD COLUMN placed_by TEXT NOT NULL DEFAULT 'you';  -- 'you' or 'aifred'
ALTER TABLE item_link ADD COLUMN why TEXT NOT NULL DEFAULT '';    -- AIfred's reason
";

/// The AI sort (sort.rs, plan part 3b step 2). `item_sort` has the items
/// that the sort has seen. `fixed` = 1: the user placed the item (a move,
/// "Move out", or an undo of an AI move), so the AI never moves it. `link_move` logs each
/// move of the AI, for the brief and its Undo. The links from before were
/// made by the user. (SCHEMA_25 fixes their parents too.)
const SCHEMA_23: &str = "
CREATE TABLE item_sort (
  key   TEXT PRIMARY KEY,
  ts    INTEGER NOT NULL,
  fixed INTEGER NOT NULL DEFAULT 0
);
INSERT INTO item_sort (key, ts, fixed) SELECT key, ts, 1 FROM item_link;
CREATE TABLE link_move (
  id       INTEGER PRIMARY KEY,
  ts       INTEGER NOT NULL,
  key      TEXT NOT NULL,
  main     TEXT NOT NULL,             -- the new parent
  old_main TEXT NOT NULL DEFAULT '',  -- '' = it had no parent
  old_by   TEXT NOT NULL DEFAULT '',
  old_why  TEXT NOT NULL DEFAULT '',
  why      TEXT NOT NULL
);
CREATE INDEX link_move_by_ts ON link_move(ts);
";

/// Smaller missions (missions.rs, plan part 3b step 3). `placed_by` of
/// `mission_item` says who put the item in the mission: an item that the
/// user added stays, and the brief cannot take it away. `mission_out` has
/// the items that the user took out of a mission: the brief never puts them
/// back in that mission. Only the user can add them again.
const SCHEMA_24: &str = "
ALTER TABLE mission_item ADD COLUMN placed_by TEXT NOT NULL DEFAULT 'aifred';  -- 'you' or 'aifred'
CREATE TABLE mission_out (
  mission INTEGER NOT NULL REFERENCES mission(id) ON DELETE CASCADE,
  key     TEXT NOT NULL,
  ts      INTEGER NOT NULL,
  PRIMARY KEY (mission, key)
);
";

/// SCHEMA_23 fixed only the subtask of each link that the user made. The
/// parent must be fixed too: else the AI sort could move the top item of a
/// tree that the user built. A parent that the sort saw already becomes
/// fixed too. The parents of the AI's links stay free.
const SCHEMA_25: &str = "
INSERT INTO item_sort (key, ts, fixed)
  SELECT main, MAX(ts), 1 FROM item_link WHERE placed_by = 'you' GROUP BY main
  ON CONFLICT (key) DO UPDATE SET fixed = 1;
";

/// The brief and the Undo of AIfred's moves look up the last move of each
/// item (sort.rs). Without this index, each lookup reads all of `link_move`.
const SCHEMA_26: &str = "
CREATE INDEX link_move_by_key ON link_move(key, id);
";

pub fn path(dir: &Path) -> PathBuf {
    dir.join(FILE)
}

/// Opens the database in `dir`. On the first open, it makes the tables and
/// imports the old files (see import.rs).
pub fn open(dir: &Path) -> anyhow::Result<Connection> {
    // sqlite-vec goes into each new connection of this program.
    static VEC: std::sync::Once = std::sync::Once::new();
    VEC.call_once(|| unsafe {
        #[allow(clippy::missing_transmute_annotations)]
        rusqlite::ffi::sqlite3_auto_extension(Some(std::mem::transmute(
            sqlite_vec::sqlite3_vec_init as *const (),
        )));
    });
    let path = path(dir);
    let mut conn =
        Connection::open(&path).with_context(|| format!("cannot open {}", path.display()))?;
    // Set the wait first: the change to WAL mode also needs a lock.
    conn.busy_timeout(Duration::from_secs(5))?;
    set_wal(&conn).context("cannot set WAL mode")?;
    conn.pragma_update(None, "foreign_keys", true)?;
    let version: i64 = conn
        .pragma_query_value(None, "user_version", |r| r.get(0))
        .context("cannot read the schema version")?;
    if version > VERSION {
        bail!(
            "{} is from a newer AIfred (schema {version}). Update AIfred.",
            path.display()
        );
    }
    if version < VERSION {
        upgrade(&mut conn, dir).with_context(|| format!("cannot set up {}", path.display()))?;
    }
    Ok(conn)
}

/// Turns on WAL mode. It stays on in the file, so only the first open of a
/// new file changes it. SQLite does not use the busy wait for this change:
/// when another program holds the new file, it fails at once. So it tries
/// again for up to 5 seconds.
fn set_wal(conn: &Connection) -> rusqlite::Result<()> {
    let start = std::time::Instant::now();
    loop {
        let mode: String = conn.pragma_query_value(None, "journal_mode", |r| r.get(0))?;
        if mode.eq_ignore_ascii_case("wal") {
            return Ok(());
        }
        match conn.pragma_update_and_check(None, "journal_mode", "WAL", |r| r.get::<_, String>(0)) {
            Ok(_) => return Ok(()),
            Err(e)
                if e.sqlite_error_code() == Some(rusqlite::ErrorCode::DatabaseBusy)
                    && start.elapsed() < Duration::from_secs(5) =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    }
}

/// Makes the tables of each missing schema version, in one transaction.
/// Two programs can start at the same time, so it reads the version again
/// after it has the write lock.
fn upgrade(conn: &mut Connection, dir: &Path) -> anyhow::Result<()> {
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("cannot start the schema upgrade")?;
    let version: i64 = tx.pragma_query_value(None, "user_version", |r| r.get(0))?;
    let mut imported = None;
    if version < 1 {
        tx.execute_batch(SCHEMA_1)?;
        imported = Some(crate::import::run(&tx, dir, crate::files::now())?);
    }
    if version < 2 {
        tx.execute_batch(SCHEMA_2)?;
    }
    if version < 3 {
        tx.execute_batch(SCHEMA_3)?;
    }
    if version < 4 {
        tx.execute_batch(SCHEMA_4)?;
    }
    if version < 5 {
        tx.execute_batch(SCHEMA_5)?;
    }
    if version < 6 {
        tx.execute_batch(SCHEMA_6)?;
    }
    if version < 7 {
        tx.execute_batch(SCHEMA_7)?;
    }
    if version < 8 {
        tx.execute_batch(SCHEMA_8)?;
    }
    if version < 9 {
        tx.execute_batch(SCHEMA_9)?;
    }
    if version < 10 {
        tx.execute_batch(SCHEMA_10)?;
    }
    if version < 11 {
        tx.execute_batch(SCHEMA_11)?;
    }
    if version < 12 {
        tx.execute_batch(SCHEMA_12)?;
    }
    if version < 13 {
        tx.execute_batch(SCHEMA_13)?;
    }
    if version < 14 {
        tx.execute_batch(SCHEMA_14)?;
    }
    if version < 15 {
        tx.execute_batch(SCHEMA_15)?;
    }
    if version < 16 {
        tx.execute_batch(SCHEMA_16)?;
    }
    if version < 17 {
        tx.execute_batch(SCHEMA_17)?;
    }
    if version < 18 {
        tx.execute_batch(SCHEMA_18)?;
    }
    if version < 19 {
        tx.execute_batch(SCHEMA_19)?;
    }
    if version < 20 {
        tx.execute_batch(SCHEMA_20)?;
    }
    if version < 21 {
        tx.execute_batch(SCHEMA_21)?;
    }
    if version < 22 {
        tx.execute_batch(SCHEMA_22)?;
    }
    if version < 23 {
        tx.execute_batch(SCHEMA_23)?;
    }
    if version < 24 {
        tx.execute_batch(SCHEMA_24)?;
    }
    if version < 25 {
        tx.execute_batch(SCHEMA_25)?;
    }
    if version < 26 {
        tx.execute_batch(SCHEMA_26)?;
    }
    tx.pragma_update(None, "user_version", VERSION)?;
    tx.commit()?;
    // The data is in the database now. Only then move the old files away.
    if let Some(done) = imported {
        done.finish(dir);
    }
    Ok(())
}

/// Runs `f` in one transaction that takes the write lock at the start.
/// A transaction that takes the lock later can fail at once with "database is
/// locked", because SQLite cannot wait in that case. If `f` fails, nothing is saved.
pub fn write<R>(
    dir: &Path,
    f: impl FnOnce(&Transaction) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    write_with(dir, true, f)
}

/// AIfred's jobs share the task table with the user's tasks (SCHEMA_19).
/// A change of the user's tasks must never add, change, or delete a job row:
/// a delete also deletes the job's runs. So `write` puts these guards on its
/// connection. A query that forgets "doer = 'you'" then fails, and it does
/// not change a job. TEMP triggers belong to one connection only, and each
/// write opens its own connection. Only jobs.rs writes job rows, with
/// `write_jobs`, which has no guards.
const GUARD_JOBS: &str = "
CREATE TEMP TRIGGER guard_job_insert BEFORE INSERT ON main.task WHEN new.doer = 'aifred' BEGIN
  SELECT RAISE(ABORT, 'a change of your tasks cannot add AIfred''s jobs');
END;
CREATE TEMP TRIGGER guard_job_update BEFORE UPDATE ON main.task
  WHEN old.doer = 'aifred' OR new.doer IS NOT old.doer BEGIN
  SELECT RAISE(ABORT, 'a change of your tasks cannot change AIfred''s jobs');
END;
CREATE TEMP TRIGGER guard_job_delete BEFORE DELETE ON main.task WHEN old.doer = 'aifred' BEGIN
  SELECT RAISE(ABORT, 'a change of your tasks cannot delete AIfred''s jobs');
END;
";

/// `write` for the rows of AIfred's jobs (jobs.rs), with no guards.
pub(crate) fn write_jobs<R>(
    dir: &Path,
    f: impl FnOnce(&Transaction) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    write_with(dir, false, f)
}

fn write_with<R>(
    dir: &Path,
    guard: bool,
    f: impl FnOnce(&Transaction) -> anyhow::Result<R>,
) -> anyhow::Result<R> {
    let mut conn = open(dir)?;
    if guard {
        conn.execute_batch(GUARD_JOBS)
            .context("cannot add the job guards")?;
    }
    let tx = conn
        .transaction_with_behavior(TransactionBehavior::Immediate)
        .context("cannot start a change")?;
    let out = f(&tx)?;
    tx.commit().context("cannot save a change")?;
    Ok(out)
}

/// Runs `f` with a connection for reading.
pub fn read<R>(dir: &Path, f: impl FnOnce(&Connection) -> anyhow::Result<R>) -> anyhow::Result<R> {
    f(&open(dir)?)
}

/// A value from the meta table.
pub(crate) fn meta(c: &Connection, key: &str) -> anyhow::Result<Option<String>> {
    use rusqlite::OptionalExtension;
    Ok(
        c.query_row("SELECT value FROM meta WHERE key = ?1", [key], |r| r.get(0))
            .optional()?,
    )
}

/// Sets a value in the meta table. Use it in a db::write transaction.
pub(crate) fn set_meta(c: &Connection, key: &str, value: &str) -> anyhow::Result<()> {
    c.execute(
        "INSERT INTO meta (key, value) VALUES (?1, ?2) ON CONFLICT (key) DO UPDATE SET value = excluded.value",
        [key, value],
    )?;
    Ok(())
}

/// How many results each search method gives to the merge.
pub(crate) const CANDIDATES: usize = 20;

/// The largest cosine distance that counts as related. Measured with
/// all-MiniLM-L6-v2: related pairs ("book my flight" and "I prefer window
/// seats") are at 0.73 or less, unrelated pairs ("thanks!" and any fact) at
/// 0.84 or more. Without a limit, a search always returns the nearest rows,
/// also when nothing is related. A new model needs a new measurement.
pub(crate) const MAX_DISTANCE: f64 = 0.8;

/// The rowids of the vectors nearest in meaning to `query`, nearest first.
/// Only rows nearer than MAX_DISTANCE.
/// `keep` drops the rows that the search does not want (for example done
/// tasks), and the first CANDIDATES that stay are returned. Empty when there
/// are no vectors of this model, or the model cannot load: word search still works.
pub(crate) fn nearest(
    c: &Connection,
    dir: &Path,
    vec_table: &str,
    query: &str,
    mut keep: impl FnMut(i64) -> anyhow::Result<bool>,
) -> anyhow::Result<Vec<i64>> {
    if meta(c, "embed_model")?.as_deref() != Some(crate::embed::MODEL) {
        return Ok(vec![]);
    }
    let any: bool = c.query_row(
        &format!("SELECT EXISTS (SELECT 1 FROM {vec_table})"),
        [],
        |r| r.get(0),
    )?;
    if !any || query.trim().is_empty() {
        return Ok(vec![]);
    }
    // A short wait: the user or the chat waits for this search.
    let v = match crate::embed::embed_query(dir, query) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("aifred: {e}");
            return Ok(vec![]);
        }
    };
    // More than CANDIDATES, because `keep` can drop some.
    let mut q = c.prepare(&format!(
        "SELECT rowid, distance FROM {vec_table} WHERE embedding MATCH ?1 AND k = ?2 ORDER BY distance"
    ))?;
    let rows: Vec<(i64, f64)> = q
        .query_map(
            rusqlite::params![crate::embed::to_blob(&v), (CANDIDATES * 5) as i64],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, distance) in rows {
        // Nearest first, so the rest are farther.
        if out.len() == CANDIDATES || distance > MAX_DISTANCE {
            break;
        }
        if keep(id)? {
            out.push(id);
        }
    }
    Ok(out)
}

/// Merges the word results and the meaning results, both best first, with
/// reciprocal rank fusion: each row gets 1 / (60 + rank) from each list.
/// A row near the top of both lists comes first.
pub(crate) fn merge(word: &[i64], meaning: &[i64], limit: usize) -> Vec<i64> {
    let mut score: Vec<(i64, f64)> = Vec::new();
    for list in [word, meaning] {
        for (rank, id) in list.iter().enumerate() {
            let s = 1.0 / (60.0 + rank as f64 + 1.0);
            match score.iter_mut().find(|(x, _)| x == id) {
                Some((_, total)) => *total += s,
                None => score.push((*id, s)),
            }
        }
    }
    // A stable sort: for the same score, the word result stays first.
    score.sort_by(|a, b| b.1.total_cmp(&a.1));
    score.into_iter().take(limit).map(|(id, _)| id).collect()
}

/// One item of a downloaded list, for save_list.
pub(crate) struct ListItem {
    pub id: String,
    pub title: String,
    pub place: String,
    pub url: String,
    /// The full item as JSON.
    pub data: String,
    /// The description, when the download has it. None keeps the saved one.
    pub details: Option<String>,
}

/// Saves a downloaded list of `source`. New items are added. Items that are
/// already there get the new data. Items that are not in the download get
/// in_list = 0: they stay for search. `details` changes only when the
/// download has a description.
pub(crate) fn save_list(
    tx: &Transaction,
    source: &str,
    account: &str,
    items: &[ListItem],
    cut: &[String],
    version: u32,
    now: u64,
) -> anyhow::Result<()> {
    tx.execute(
        "UPDATE work_item SET in_list = 0 WHERE source = ?1",
        [source],
    )?;
    let mut upsert = tx.prepare(
        "INSERT INTO work_item (source, item_id, account, title, place, url, data, in_list, pos, first_seen, last_seen, details)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 1, ?8, ?9, ?9, COALESCE(?10, ''))
         ON CONFLICT (source, item_id) DO UPDATE SET
           account = excluded.account, title = excluded.title, place = excluded.place, url = excluded.url,
           data = excluded.data, in_list = 1, pos = excluded.pos, last_seen = excluded.last_seen,
           details = COALESCE(?10, work_item.details)",
    )?;
    for (pos, i) in items.iter().enumerate() {
        upsert.execute(rusqlite::params![
            source, i.id, account, i.title, i.place, i.url, i.data, pos as i64, now as i64,
            i.details
        ])?;
    }
    tx.execute(
        "INSERT INTO work_fetch (source, account, fetched_at, version, cut) VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT (source) DO UPDATE SET
           account = excluded.account, fetched_at = excluded.fetched_at, version = excluded.version, cut = excluded.cut",
        rusqlite::params![source, account, now as i64, version, serde_json::to_string(cut)?],
    )?;
    Ok(())
}

/// The data JSON of the items of `source` that are in the last download, in
/// the download order. `account` limits the rows when it is Some.
pub(crate) fn list_data<T: serde::de::DeserializeOwned>(
    conn: &Connection,
    source: &str,
    account: Option<&str>,
) -> anyhow::Result<Vec<T>> {
    let mut q = conn.prepare(
        "SELECT item_id, data FROM work_item
         WHERE source = ?1 AND in_list = 1 AND (?2 IS NULL OR account = ?2) ORDER BY pos",
    )?;
    let rows = q.query_map(rusqlite::params![source, account], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    rows.map(|row| {
        let (id, data) = row?;
        serde_json::from_str(&data)
            .with_context(|| format!("cannot read the saved {source} item {id}"))
    })
    .collect()
}

/// Common words that match almost every text. They do not help a search.
const STOP_WORDS: &[&str] = &[
    "a", "an", "and", "are", "as", "at", "be", "by", "did", "do", "for", "from", "has", "have",
    "i", "in", "is", "it", "me", "my", "of", "on", "or", "that", "the", "this", "to", "was", "we",
    "what", "when", "where", "which", "who", "with", "you", "about",
];

/// Turns the words of a question into an FTS5 search. Each word also finds
/// longer words that start with it ("inv" finds "invoice"). A text that has
/// any of the words matches; texts with more and rarer words come first.
/// None when the query has no useful words.
pub(crate) fn match_query(query: &str) -> Option<String> {
    let words: Vec<String> = query
        .split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty() && !STOP_WORDS.contains(&w.as_str()))
        .map(|w| format!("\"{w}\"*"))
        .collect();
    (!words.is_empty()).then(|| words.join(" OR "))
}

/// The number of results to send back: `limit`, or 10, and never more than 50.
pub(crate) fn result_limit(limit: Option<u32>) -> i64 {
    limit.unwrap_or(10).clamp(1, 50) as i64
}

/// One GitHub item or ClickUp task that a search found.
#[derive(Debug, serde::Serialize)]
pub struct WorkHit {
    /// The item, as the list tools show it.
    pub item: serde_json::Value,
    /// False when the item was not in the last download, for example a closed issue.
    pub in_list: bool,
    /// When AIfred last saw the item in a download, in Unix seconds.
    pub last_seen: u64,
}

impl WorkHit {
    /// The item as the chat tools send it: its fields, plus in_list and last_seen.
    pub fn into_json(self) -> serde_json::Value {
        let mut v = self.item;
        v["in_list"] = self.in_list.into();
        v["last_seen"] = self.last_seen.into();
        v
    }
}

/// Word search and meaning search together. Returns the rowids of the best
/// `limit` rows, best first.
///
/// - `word_sql` gets the FTS5 query as ?1 and `params` as ?2, ?3, ... It
///   returns rowids, best first.
/// - `keep_sql` gets a rowid as ?1 and `params` as ?2, ?3, ... It returns a
///   row when the search wants that row. It filters the meaning results.
///
/// A query with only common words gets no word search. The meaning search
/// still runs, so "who am I?" can find facts.
// Each search passes its own SQL and values. A struct for them adds no clarity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn hybrid(
    c: &Connection,
    dir: &Path,
    query: &str,
    vec_table: &str,
    word_sql: &str,
    keep_sql: &str,
    params: &[&dyn rusqlite::ToSql],
    limit: Option<u32>,
) -> anyhow::Result<Vec<i64>> {
    use rusqlite::OptionalExtension;
    let word: Vec<i64> = match match_query(query) {
        Some(q) => {
            let mut word_params: Vec<&dyn rusqlite::ToSql> = vec![&q];
            word_params.extend_from_slice(params);
            let mut s = c.prepare(&format!("{word_sql} LIMIT {CANDIDATES}"))?;
            s.query_map(word_params.as_slice(), |r| r.get(0))?
                .collect::<Result<_, _>>()?
        }
        None => vec![],
    };
    let mut keep = c.prepare(keep_sql)?;
    let meaning = nearest(c, dir, vec_table, query, |id| {
        let mut p: Vec<&dyn rusqlite::ToSql> = vec![&id];
        p.extend_from_slice(params);
        Ok(keep
            .query_row(p.as_slice(), |_| Ok(()))
            .optional()?
            .is_some())
    })?;
    Ok(merge(&word, &meaning, result_limit(limit) as usize))
}

/// Searches the saved items of `source`. `account` limits the rows when it
/// is Some. A non-empty `owners` (lowercase) keeps only the items whose place
/// ("owner/repo") starts with one of those owners. The filter is in the SQL,
/// so items of other owners do not use up result slots.
// Each value is one filter of the search. A struct for them adds no clarity.
#[allow(clippy::too_many_arguments)]
pub(crate) fn search_work(
    dir: &Path,
    source: &str,
    account: Option<&str>,
    owners: &[String],
    query: &str,
    include_old: bool,
    limit: Option<u32>,
) -> anyhow::Result<Vec<WorkHit>> {
    let owners: Option<String> = if owners.is_empty() {
        None
    } else {
        Some(serde_json::to_string(owners)?)
    };
    read(dir, |c| {
        let filter = "w.source = ?2 AND (?3 IS NULL OR w.account = ?3) AND (?4 OR w.in_list = 1)
            AND (?5 IS NULL OR lower(substr(w.place, 1, instr(w.place, '/') - 1)) IN (SELECT value FROM json_each(?5)))";
        // The title counts more than the place and the description.
        let ids = hybrid(
            c,
            dir,
            query,
            "work_item_vec",
            &format!(
                "SELECT w.rowid FROM work_item_fts f JOIN work_item w ON w.rowid = f.rowid
                 WHERE work_item_fts MATCH ?1 AND {filter} ORDER BY bm25(work_item_fts, 3.0, 1.0, 1.0)"
            ),
            &format!("SELECT 1 FROM work_item w WHERE w.rowid = ?1 AND {filter}"),
            &[&source, &account, &include_old, &owners],
            limit,
        )?;
        let mut get =
            c.prepare("SELECT data, in_list, last_seen FROM work_item WHERE rowid = ?1")?;
        ids.iter()
            .map(|id| {
                let (data, in_list, last_seen): (String, bool, u64) =
                    get.query_row([id], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
                Ok(WorkHit {
                    item: serde_json::from_str(&data)?,
                    in_list,
                    last_seen,
                })
            })
            .collect()
    })
}

/// Saves the description of one item, so a search can find words in it.
/// Nothing happens if the item is not saved.
pub(crate) fn save_details(
    dir: &Path,
    source: &str,
    id: &str,
    details: &str,
) -> anyhow::Result<()> {
    write(dir, |tx| {
        tx.execute(
            "UPDATE work_item SET details = ?1 WHERE source = ?2 AND item_id = ?3 COLLATE NOCASE AND details != ?1",
            [details, source, id],
        )?;
        Ok(())
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A new, empty data folder for each test. Tests run at the same time,
    /// so each one needs its own folder.
    pub(crate) fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-db-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn open_makes_the_tables_once_and_uses_wal() {
        let d = temp("open");
        let c = open(&d).unwrap();
        let mode: String = c
            .pragma_query_value(None, "journal_mode", |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        drop(c);
        // A second open does not make the tables again (that would fail).
        let c = open(&d).unwrap();
        let v: i64 = c
            .pragma_query_value(None, "user_version", |r| r.get(0))
            .unwrap();
        assert_eq!(v, VERSION);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_update_from_22_fixes_both_sides_of_the_old_links() {
        let d = temp("v23");
        // Load sqlite-vec into new connections, then make a version 22 file by hand.
        let vec_dir = temp("v23-vec");
        drop(open(&vec_dir).unwrap());
        std::fs::remove_dir_all(&vec_dir).unwrap();
        let c = Connection::open(path(&d)).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c.execute_batch(
            &[
                SCHEMA_1, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5, SCHEMA_6, SCHEMA_7, SCHEMA_8,
                SCHEMA_9, SCHEMA_10, SCHEMA_11, SCHEMA_12, SCHEMA_13, SCHEMA_14, SCHEMA_15,
                SCHEMA_16, SCHEMA_17, SCHEMA_18, SCHEMA_19, SCHEMA_20, SCHEMA_21, SCHEMA_22,
            ]
            .concat(),
        )
        .unwrap();
        c.pragma_update(None, "user_version", 22).unwrap();
        // The user put a pull request under a ClickUp task, which has a parent too.
        c.execute_batch(
            "INSERT INTO item_link (key, main, ts) VALUES ('github:a/b#1', 'clickup:c1', 5);
             INSERT INTO item_link (key, main, ts) VALUES ('clickup:c1', 'clickup:top', 3);",
        )
        .unwrap();
        drop(c);

        let c = open(&d).unwrap();
        let mut q = c
            .prepare("SELECT key FROM item_sort WHERE fixed = 1 ORDER BY key")
            .unwrap();
        let fixed: Vec<String> = q
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(fixed, ["clickup:c1", "clickup:top", "github:a/b#1"]);
        drop(q);
        drop(c);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_update_to_25_fixes_the_parents_of_your_links_only() {
        let d = temp("v25");
        // Load sqlite-vec into new connections, then make a version 24 file by hand.
        let vec_dir = temp("v25-vec");
        drop(open(&vec_dir).unwrap());
        std::fs::remove_dir_all(&vec_dir).unwrap();
        let c = Connection::open(path(&d)).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c.execute_batch(
            &[
                SCHEMA_1, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5, SCHEMA_6, SCHEMA_7, SCHEMA_8,
                SCHEMA_9, SCHEMA_10, SCHEMA_11, SCHEMA_12, SCHEMA_13, SCHEMA_14, SCHEMA_15,
                SCHEMA_16, SCHEMA_17, SCHEMA_18, SCHEMA_19, SCHEMA_20, SCHEMA_21, SCHEMA_22,
                SCHEMA_23, SCHEMA_24,
            ]
            .concat(),
        )
        .unwrap();
        c.pragma_update(None, "user_version", 24).unwrap();
        // Your link: its parent was seen by the sort, but is not fixed.
        // AIfred's link: its parent stays free.
        c.execute_batch(
            "INSERT INTO item_link (key, main, ts, placed_by) VALUES ('github:a/b#1', 'clickup:c1', 5, 'you');
             INSERT INTO item_link (key, main, ts, placed_by, why) VALUES ('github:a/b#2', 'clickup:c2', 6, 'aifred', 'x');
             INSERT INTO item_sort (key, ts, fixed) VALUES ('github:a/b#1', 5, 1), ('clickup:c1', 4, 0),
                                                         ('github:a/b#2', 6, 0), ('clickup:c2', 4, 0);",
        )
        .unwrap();
        drop(c);

        let c = open(&d).unwrap();
        let mut q = c
            .prepare("SELECT key FROM item_sort WHERE fixed = 1 ORDER BY key")
            .unwrap();
        let fixed: Vec<String> = q
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert_eq!(fixed, ["clickup:c1", "github:a/b#1"]);
        drop(q);
        drop(c);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_upgrade_keeps_the_row_numbers_and_vacuum_does_not_change_them() {
        let d = temp("rid");
        // Load sqlite-vec into new connections, then make a version 5 file by hand.
        let vec_dir = temp("rid-vec");
        drop(open(&vec_dir).unwrap());
        std::fs::remove_dir_all(&vec_dir).unwrap();
        let c = Connection::open(path(&d)).unwrap();
        c.execute_batch(&[SCHEMA_1, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5].concat())
            .unwrap();
        c.pragma_update(None, "user_version", 5).unwrap();
        for id in ["t1", "t2", "t3"] {
            c.execute(
                "INSERT INTO task (id, title, notes, added_by, created) VALUES (?1, ?1, 'invoice for Sam', 'you', 0)",
                [id],
            )
            .unwrap();
        }
        c.execute("DELETE FROM task WHERE id = 't1'", []).unwrap();
        drop(c);

        let c = open(&d).unwrap();
        let rowid = |c: &Connection| -> i64 {
            c.query_row("SELECT rowid FROM task WHERE id = 't3'", [], |r| r.get(0))
                .unwrap()
        };
        assert_eq!(rowid(&c), 3, "the upgrade keeps the old row number");
        c.execute_batch("VACUUM").unwrap();
        assert_eq!(rowid(&c), 3, "VACUUM keeps the row number");
        // The word search still finds the task, and it is the right one.
        let found: String = c
            .query_row(
                "SELECT t.id FROM task_fts f JOIN task t ON t.rowid = f.rowid WHERE task_fts MATCH 'invoice' ORDER BY t.rowid DESC LIMIT 1",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(found, "t3");
        // The triggers work on the new table.
        c.execute("UPDATE task SET summary = 'S' WHERE id = 't3'", [])
            .unwrap();
        c.execute("UPDATE task SET notes = 'new' WHERE id = 't3'", [])
            .unwrap();
        let summary: Option<String> = c
            .query_row("SELECT summary FROM task WHERE id = 't3'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(summary, None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn version_19_moves_the_jobs_into_the_task_table_and_keeps_their_runs() {
        let d = temp("v19");
        // Load sqlite-vec into new connections, then make a version 18 file by hand.
        let vec_dir = temp("v19-vec");
        drop(open(&vec_dir).unwrap());
        std::fs::remove_dir_all(&vec_dir).unwrap();
        let c = Connection::open(path(&d)).unwrap();
        c.pragma_update(None, "foreign_keys", true).unwrap();
        c.execute_batch(
            &[
                SCHEMA_1, SCHEMA_2, SCHEMA_3, SCHEMA_4, SCHEMA_5, SCHEMA_6, SCHEMA_7, SCHEMA_8,
                SCHEMA_9, SCHEMA_10, SCHEMA_11, SCHEMA_12, SCHEMA_13, SCHEMA_14, SCHEMA_15,
                SCHEMA_16, SCHEMA_17, SCHEMA_18,
            ]
            .concat(),
        )
        .unwrap();
        c.pragma_update(None, "user_version", 18).unwrap();
        c.execute(
            "INSERT INTO task (id, title, notes, added_by, created) VALUES ('t1', 'Pay rent', '', 'you', 1)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO job (id, title, action, message, trigger, tz, status, next_run, created_by, created, updated, cursor)
             VALUES ('j1', 'Check PR 12', 'agent', 'Read the PR', '{\"kind\":\"once\",\"at\":\"2026-01-01T09:00\"}',
                     'UTC', 'active', 500, 'you', 2, 3, 7),
                    ('j2', 'Old reminder', 'remind', 'Call Ana', '{\"kind\":\"once\",\"at\":\"2025-01-01T09:00\"}',
                     'UTC', 'done', NULL, 'aifred', 2, 4, 0)",
            [],
        )
        .unwrap();
        c.execute(
            "INSERT INTO job_run (job, planned, state, output) VALUES ('j1', 400, 'ok', 'PR is open')",
            [],
        )
        .unwrap();
        drop(c);

        let c = open(&d).unwrap();
        let tables: i64 = c
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'job'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(tables, 0, "the job table is gone");
        let row = |id: &str| -> (
            String,
            String,
            String,
            i64,
            Option<i64>,
            String,
            i64,
            String,
        ) {
            c.query_row(
                "SELECT doer, COALESCE(action, ''), notes, done, done_at, COALESCE(status, ''), cursor, message FROM task WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?)),
            )
            .unwrap()
        };
        let (doer, _, _, done, _, status, _, _) = row("t1");
        assert_eq!((doer.as_str(), done, status.as_str()), ("you", 0, ""));
        let (doer, action, notes, done, _, status, cursor, message) = row("j1");
        assert_eq!(
            (
                doer.as_str(),
                action.as_str(),
                notes.as_str(),
                done,
                status.as_str(),
                cursor,
                message.as_str()
            ),
            ("aifred", "agent", "", 0, "active", 7, "Read the PR"),
            "the job message stays out of the notes"
        );
        let (_, _, _, done, done_at, status, _, _) = row("j2");
        assert_eq!((done, done_at, status.as_str()), (1, Some(4), "done"));
        let (doer, action, ..) = row(crate::jobs::BRIEF_JOB);
        assert_eq!((doer.as_str(), action.as_str()), ("aifred", "brief"));

        // The run points at the task now, and goes with it.
        let runs = |c: &Connection| -> i64 {
            c.query_row("SELECT COUNT(*) FROM job_run WHERE job = 'j1'", [], |r| {
                r.get(0)
            })
            .unwrap()
        };
        assert_eq!(runs(&c), 1, "the upgrade keeps the runs");
        let err = c.execute(
            "INSERT INTO job_run (job, planned, state) VALUES ('nothing', 1, 'ok')",
            [],
        );
        assert!(err.is_err(), "a run needs a task");
        // AIfred's tasks are not in the word search, and not in the view.
        let count = |sql: &str| -> i64 { c.query_row(sql, [], |r| r.get(0)).unwrap() };
        assert_eq!(
            count("SELECT COUNT(*) FROM task_fts WHERE task_fts MATCH 'check OR PR'"),
            0,
            "the job titles are not indexed"
        );
        assert_eq!(
            count("SELECT COUNT(*) FROM task_fts WHERE task_fts MATCH 'rent'"),
            1
        );
        assert_eq!(count("SELECT COUNT(*) FROM user_task"), 1);
        c.execute("UPDATE task SET title = 'Check PR 13' WHERE id = 'j1'", [])
            .unwrap();
        assert_eq!(
            count("SELECT COUNT(*) FROM task_fts WHERE task_fts MATCH 'PR'"),
            0
        );
        // A job that is done now is a done task.
        c.execute("UPDATE task SET status = 'done' WHERE id = 'j1'", [])
            .unwrap();
        assert_eq!(row("j1").3, 1);
        c.execute("UPDATE task SET status = 'active' WHERE id = 'j1'", [])
            .unwrap();
        let (_, _, _, done, done_at, ..) = row("j1");
        assert_eq!((done, done_at), (0, None));
        c.execute("DELETE FROM task WHERE id = 'j1'", []).unwrap();
        assert_eq!(runs(&c), 0, "a deleted task loses its runs");
        // The delete of a job that is not in the index keeps the index whole.
        // rank 0: check the index only. The jobs are in the content table on purpose.
        c.execute(
            "INSERT INTO task_fts (task_fts, rank) VALUES ('integrity-check', 0)",
            [],
        )
        .unwrap();
        drop(c);

        // The user's task list does not show AIfred's tasks.
        let titles: Vec<String> = crate::tasks::TaskStore::new(&d)
            .load()
            .unwrap()
            .into_iter()
            .map(|t| t.title)
            .collect();
        assert_eq!(titles, ["Pay rent"]);
        let jobs: Vec<String> = crate::jobs::list(&d, true)
            .unwrap()
            .into_iter()
            .map(|j| j.id)
            .collect();
        assert_eq!(jobs, [crate::jobs::BRIEF_JOB, "j2"]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn word_search_changes_only_when_the_text_changes() {
        let d = temp("fts-when");
        let c = open(&d).unwrap();
        for name in ["task_fts_update", "work_item_fts_update"] {
            let sql: String = c
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name = ?1",
                    [name],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(sql.contains("old.title IS NOT new.title"), "{name}: {sql}");
        }
        c.execute("INSERT INTO task (id, title, notes, added_by, created) VALUES ('t', 'Pay rent', '', 'you', 0)", []).unwrap();
        let find = |c: &Connection, word: &str| -> i64 {
            c.query_row(
                "SELECT count(*) FROM task_fts WHERE task_fts MATCH ?1",
                [word],
                |r| r.get(0),
            )
            .unwrap()
        };
        // The same text again: the index still finds it once.
        c.execute("UPDATE task SET title = 'Pay rent' WHERE id = 't'", [])
            .unwrap();
        assert_eq!(find(&c, "rent"), 1);
        // A new text: the index has the new words only.
        c.execute("UPDATE task SET title = 'Pay the bills' WHERE id = 't'", [])
            .unwrap();
        assert_eq!(find(&c, "rent"), 0);
        assert_eq!(find(&c, "bills"), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_database_from_a_newer_aifred_is_an_error() {
        let d = temp("newer");
        open(&d)
            .unwrap()
            .pragma_update(None, "user_version", VERSION + 1)
            .unwrap();
        let e = open(&d).unwrap_err().to_string();
        assert!(e.contains("newer AIfred"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn match_query_keeps_useful_words_only() {
        assert_eq!(
            match_query("What was the invoice task?").as_deref(),
            Some("\"invoice\"* OR \"task\"*")
        );
        assert_eq!(
            match_query("\"); DROP TABLE task; --").as_deref(),
            Some("\"drop\"* OR \"table\"* OR \"task\"*")
        );
        assert_eq!(match_query("the ??"), None);
    }

    #[test]
    fn merge_puts_rows_found_by_both_methods_first() {
        // 3 is second in both lists, so it beats 1 (first in one list only).
        assert_eq!(merge(&[1, 3, 4], &[2, 3], 10), vec![3, 1, 2, 4]);
        assert_eq!(merge(&[1, 2], &[], 1), vec![1]);
    }

    #[test]
    fn a_failed_write_saves_nothing() {
        let d = temp("rollback");
        let r: anyhow::Result<()> = write(&d, |tx| {
            tx.execute("INSERT INTO meta (key, value) VALUES ('a', 'b')", [])?;
            bail!("stop")
        });
        assert!(r.is_err());
        let n: i64 = read(&d, |c| {
            Ok(c.query_row("SELECT count(*) FROM meta", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(n, 0);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
