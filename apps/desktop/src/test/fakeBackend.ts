// A fake AIfred backend for window tests. It answers the same commands as
// the Rust app (src-tauri/src/main.rs), but keeps everything in memory.
// No Rust, no Claude, no GitHub, no cost.
import { mockIPC } from "@tauri-apps/api/mocks";
import { DEFAULT_SETTINGS } from "../api";
import type {
  Answer,
  Brief,
  CalEvent,
  CalendarList,
  ClickupItem,
  DayPlan,
  Estimate,
  Fact,
  FileRef,
  Goals,
  GoogleStatus,
  GithubItem,
  ItemDetails,
  Job,
  JobRun,
  ProgressEvent,
  Settings,
  SideTurn,
  Task,
  TaskChanges,
  Turn,
  UpdateStatus,
} from "../api";

type AskHandler = (prompt: string, progress: (e: ProgressEvent) => void, args: Record<string, any>) => Promise<Answer> | Answer;

export type Fake = {
  tasks: Task[];
  /** Open items on "GitHub". */
  github: GithubItem[];
  githubError?: string;
  /** Sent with the list, like the Rust app when GitHub cannot be asked. */
  githubWarning?: string;
  /** Runs after the list is made and before it is returned. Return a
   * promise to hold the answer, like a slow GitHub. */
  onGithubItems?: (force: boolean) => Promise<void> | void;
  /** Runs after edit_goal changes the goal and before it answers. A test
   * can hold the answer with it, so the answer comes after a later change. */
  onEditGoal?: () => Promise<void> | void;
  /** Runs before day_plan answers. A test can hold the plan load with it. */
  onDayPlan?: (day: string) => Promise<void> | void;
  /** Runs before connect_google answers, like a user in the browser. A
   * rejected promise is a login that stopped. */
  onConnectGoogle?: () => Promise<void> | void;
  /** How many times the window asked GitHub (not the saved list). */
  githubFetches: number;
  /** The ClickUp token, or "" when ClickUp is not connected. */
  clickupToken: string;
  /** Open tasks assigned to you on "ClickUp". A task closed here leaves this list. */
  clickup: ClickupItem[];
  /** Tasks that were closed, by id, as they were before. */
  clickupClosed: Record<string, ClickupItem>;
  clickupError?: string;
  clickupWarning?: string;
  /** How many times the window asked ClickUp (not the saved list). */
  clickupFetches: number;
  settings: Settings;
  /** The update status. A check moves it to "ready" when `updateTo` is set. */
  update: UpdateStatus;
  /** The version that a check finds. null: no new version. */
  updateTo: string | null;
  /** How many times the window asked to restart. */
  restarts: number;
  /** The model picked in the chat box. null: the default. */
  chatModel: string | null;
  history: Turn[];
  /** All facts, also the forgotten ones. */
  memory: Fact[];
  /** The calendar. null: Google is not connected, so no calendar. */
  calendar: CalendarList | null;
  calendarError?: string;
  /** The Google connection. connect_google connects as me@x.com and allows all features. */
  google: GoogleStatus;
  /** The user's goals, like goals.rs. */
  goals: Goals;
  /** Commands that fail, with the error text. Example: { tasks: "cannot read tasks.json" }. */
  fail: Record<string, string>;
  /** Every command the window called, in order. */
  calls: { cmd: string; args: Record<string, unknown> }[];
  onAsk: AskHandler;
  /** Answers in the side chats. The thread is in args.thread. */
  onAskSide: AskHandler;
  /** Day plans, by day. */
  plans: Record<string, DayPlan>;
  estimates: Record<string, Estimate>;
  briefs: Record<string, Brief>;
  /** AIfred's own jobs, and their runs (newest first). */
  jobs: Job[];
  jobRuns: JobRun[];
  /** What write_brief makes. Throw to fail it. */
  onWriteBrief: (day: string) => Promise<Brief> | Brief;
  side: Record<string, SideTurn[]>;
  /** Keys of GitHub and ClickUp items that came in the last day. */
  newKeys: string[];
  summaries: Record<string, string>;
  details: Record<string, string>;
  uploads: FileRef[];
  /** The name of the next file that the window sends. The fake cannot see the header. */
  uploadName: string;
};

let nextId = 1;
/** Unix seconds, like the Rust app (tasks.rs). Not milliseconds. */
const nowSecs = () => Math.floor(Date.now() / 1000);

export function task(title: string, extra: Partial<Task> = {}): Task {
  const n = nextId++;
  // Each new task is one second newer, so the sort order is known.
  return {
    id: `t${n}`,
    title,
    notes: "",
    done: false,
    added_by: "you",
    created: nowSecs() + n,
    done_at: null,
    due: null,
    start: null,
    priority: null,
    tags: [],
    checklist: [],
    files: [],
    ...extra,
  };
}

/** "YYYY-MM-DD" for a real day, like is_date in tasks.rs. Empty clears the field. */
function cleanDate(name: string, s: string): string | null {
  const v = s.trim();
  if (!v) return null;
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(v);
  const d = m && new Date(Date.UTC(+m[1], +m[2] - 1, +m[3]));
  if (!m || !d || d.getUTCMonth() !== +m[2] - 1 || d.getUTCDate() !== +m[3]) {
    throw `the ${name} date "${v}" is not a day in the form YYYY-MM-DD`;
  }
  return v;
}

/** Like apply() in tasks.rs: the same cleaning and the same checks. */
function applyChanges(t: Task, c: TaskChanges) {
  if (c.title !== undefined) t.title = c.title.trim();
  if (!t.title) throw "the task title is empty";
  if (c.notes !== undefined) t.notes = c.notes.trim();
  if (c.due !== undefined) t.due = cleanDate("due", c.due);
  if (c.start !== undefined) t.start = cleanDate("start", c.start);
  if (c.priority !== undefined) {
    const p = c.priority.trim().toLowerCase();
    if (p && p !== "none" && !["urgent", "high", "normal", "low"].includes(p)) throw `the priority "${p}" is not one of: urgent, high, normal, low`;
    t.priority = p && p !== "none" ? (p as Task["priority"]) : null;
  }
  if (c.tags !== undefined) {
    const tags: string[] = [];
    for (const raw of c.tags) {
      const tag = raw.trim().replace(/^#+/, "").trim();
      if (tag && !tags.some((x) => x.toLowerCase() === tag.toLowerCase())) tags.push(tag);
    }
    t.tags = tags;
  }
  if (c.checklist !== undefined) {
    // A step with no done value keeps the tick of the old step with the same text.
    const old = t.checklist;
    t.checklist = c.checklist
      .map((i) => {
        const text = i.text.trim();
        const was = old.some((o) => o.done && o.text.toLowerCase() === text.toLowerCase());
        return { text, done: i.done ?? was };
      })
      .filter((i) => i.text);
  }
  if (t.start && t.due && t.start > t.due) throw `the start date ${t.start} is after the due date ${t.due}`;
}

export function fact(text: string, extra: Partial<Fact> = {}): Fact {
  const n = nextId++;
  return { id: n, text, core: true, added_by: "you", created: nowSecs(), updated: nowSecs(), forgotten: null, ...extra };
}

export function githubItem(n: number, reason: GithubItem["reason"], title = `Item ${n}`): GithubItem {
  return {
    id: `acme/shop#${n}`,
    repo: "acme/shop",
    number: n,
    title,
    // Assigned items are issues. The others are pull requests.
    url: `https://github.com/acme/shop/${reason === "assigned" ? "issues" : "pull"}/${n}`,
    reason,
    updated: new Date(Date.now() - 2 * 86400_000).toISOString(),
  };
}

export function clickupItem(id: string, title = `Task ${id}`, extra: Partial<ClickupItem> = {}): ClickupItem {
  return {
    id,
    title,
    url: `https://app.clickup.com/t/${id}`,
    status: "in progress",
    status_color: "#4194f6",
    list: "Sprint 12",
    workspace: "Rymera",
    updated: Date.now() - 3 * 3600_000,
    ...extra,
  };
}

/** A busy meeting today from `from` to `to` ("HH:MM", local time). */
export function meeting(id: string, title: string, from: string, to: string, extra: Partial<CalEvent> = {}): CalEvent {
  const at = (hhmm: string) => {
    const [h, m] = hhmm.split(":").map(Number);
    const d = new Date();
    d.setHours(h, m, 0, 0);
    return Math.floor(d.getTime() / 1000);
  };
  return { id, title, start: at(from), end: at(to), all_day: false, busy: true, attendees: ["Josh", "Ana"], more_attendees: 0, response: "accepted", url: `https://calendar.google.com/e/${id}`, description: "", ...extra };
}

export function answer(text: string, extra: Partial<Answer> = {}): Answer {
  return withInfo({ text, stats: { startup_ms: 10, total_ms: 1500, tool_calls: [] }, model_ms: 1000, cost_usd: 0.005, memory: [], model: null, info: "", ...extra });
}

/** Fills the line under the answer as the backend does (answer_info in assistant.rs). */
function withInfo(a: Answer): Answer {
  const parts = [`${(a.stats.total_ms / 1000).toFixed(1)}s`];
  if (a.model) parts.push(a.model.charAt(0).toUpperCase() + a.model.slice(1));
  const n = a.stats.tool_calls.length;
  if (n) parts.push(`${n} lookup${n === 1 ? "" : "s"}`);
  if (a.cost_usd != null) parts.push(`~$${a.cost_usd.toFixed(3)} usage`);
  return { ...a, info: parts.join(" · ") };
}

export function fakeBackend(init: Partial<Fake> = {}): Fake {
  const fake: Fake = {
    tasks: [],
    github: [],
    githubFetches: 0,
    clickupToken: "",
    clickup: [],
    clickupClosed: {},
    clickupFetches: 0,
    settings: { ...DEFAULT_SETTINGS },
    update: { current: "0.1.1", state: "idle", latest: null, page: null, message: null, checked_at: null },
    updateTo: null,
    restarts: 0,
    chatModel: null,
    history: [],
    memory: [],
    goals: { goals: [], checked: 0, stale: false },
    calendar: null,
    google: { client_id: "", secret_set: false, connected: false, email: null, features: [["Calendar", false]] },
    fail: {},
    calls: [],
    onAsk: (p) => answer(`You said: ${p}`),
    onAskSide: (p) => answer(`Side: ${p}`),
    plans: {},
    estimates: {},
    briefs: {},
    jobs: [],
    jobRuns: [],
    onWriteBrief: (day) => ({ day, written: nowSecs(), text: "Nothing urgent today.", why: {} }),
    side: {},
    newKeys: [],
    summaries: {},
    details: {},
    uploads: [],
    uploadName: "file.txt",
    ...init,
  };

  mockIPC(async (cmd, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    fake.calls.push({ cmd, args });
    if (cmd in fake.fail) throw fake.fail[cmd];
    const findFact = (id: number) => {
      const f = fake.memory.find((f) => f.id === id);
      if (!f) throw `no fact with id ${id}`;
      return f;
    };
    const factText = (text: unknown) => {
      const t = String(text).split(/\s+/).filter(Boolean).join(" ");
      if (!t) throw "the fact is empty";
      return t;
    };
    const findUpload = (id: string) => {
      const f = fake.uploads.find((u) => u.id === id);
      if (!f) throw `no file with id ${id}`;
      return f;
    };
    /** Open items, like items::is_open in Rust. */
    const isOpen = (key: string) =>
      (key.startsWith("task:") && fake.tasks.some((t) => `task:${t.id}` === key && !t.done)) ||
      fake.github.some((g) => `github:${g.id}` === key) ||
      fake.clickup.some((c) => `clickup:${c.id}` === key);
    const progressOf = (a: Record<string, any>) => {
      let index = 0;
      return (e: ProgressEvent) => (window as any).__TAURI_INTERNALS__.runCallback(a.onEvent.id, { index: index++, message: e });
    };
    const find = (id: string) => {
      const t = fake.tasks.find((t) => t.id === id);
      if (!t) throw `no task with id ${id}`;
      return t;
    };
    switch (cmd) {
      case "info":
        return { data_dir: "/fake/aifred", backend: "Claude (your login)", github_owners: null, api_key_set: false, meaning_search: "on" };
      case "history":
        return fake.history;
      case "memory_facts":
        // In use first, in their order. Then forgotten, newest first. Like memory.rs.
        return structuredClone([
          ...fake.memory.filter((f) => f.forgotten === null),
          ...fake.memory.filter((f) => f.forgotten !== null).sort((a, b) => b.forgotten! - a.forgotten!),
        ]);
      case "goals":
        return structuredClone(fake.goals);
      case "add_goal": {
        const t = String(args.text).split(/\s+/).filter(Boolean).join(" ");
        if (!t) throw "the goal is empty";
        if (fake.goals.goals.length >= 5) throw "the user has 5 goals, the most that AIfred keeps. Change or remove a goal first";
        fake.goals.goals.push({ id: nextId++, text: t, created: nowSecs(), updated: nowSecs() });
        fake.goals = { ...fake.goals, checked: nowSecs(), stale: false };
        return structuredClone(fake.goals);
      }
      case "edit_goal": {
        const g = fake.goals.goals.find((g) => g.id === args.id);
        if (!g) throw `no goal with id ${args.id}`;
        g.text = String(args.text).trim();
        fake.goals = { ...fake.goals, checked: nowSecs(), stale: false };
        const answer = structuredClone(fake.goals);
        await fake.onEditGoal?.();
        return answer;
      }
      case "remove_goal": {
        if (!fake.goals.goals.some((g) => g.id === args.id)) throw `no goal with id ${args.id}`;
        fake.goals = { goals: fake.goals.goals.filter((g) => g.id !== args.id), checked: nowSecs(), stale: false };
        return structuredClone(fake.goals);
      }
      case "confirm_goals":
        fake.goals = { ...fake.goals, checked: nowSecs(), stale: false };
        return structuredClone(fake.goals);
      case "add_fact": {
        const f = fact(factText(args.text));
        fake.memory.push(f);
        return structuredClone(f);
      }
      case "edit_fact": {
        const f = findFact(args.id);
        f.text = factText(args.text);
        return structuredClone(f);
      }
      case "set_fact_core": {
        const f = findFact(args.id);
        f.core = args.core;
        return structuredClone(f);
      }
      case "forget_fact": {
        const f = findFact(args.id);
        f.forgotten ??= nowSecs();
        return structuredClone(f);
      }
      case "restore_fact": {
        const f = findFact(args.id);
        f.forgotten = null;
        return structuredClone(f);
      }
      case "new_conversation":
        fake.history = [];
        return null;
      case "tasks":
        return structuredClone(fake.tasks);
      case "add_task": {
        const f = args.task as TaskChanges;
        if (!String(f.title ?? "").trim()) throw "the task title is empty";
        const t = task(String(f.title).trim());
        applyChanges(t, f);
        if (f.files) t.files = f.files.map((id) => findUpload(id));
        fake.tasks.push(t);
        return t;
      }
      case "edit_task": {
        const t = find(args.id);
        const next = structuredClone(t);
        applyChanges(next, args.changes as TaskChanges);
        Object.assign(t, next);
        return structuredClone(t);
      }
      case "set_task_step": {
        const t = find(args.id);
        const step = t.checklist[args.index];
        if (!step || step.text !== args.text) throw "the checklist changed. Reload the list and try again";
        step.done = args.done;
        return structuredClone(t);
      }
      case "set_task_done": {
        const t = find(args.id);
        t.done = args.done;
        t.done_at = args.done ? nowSecs() : null;
        return t;
      }
      case "delete_task":
        find(args.id);
        fake.tasks = fake.tasks.filter((t) => t.id !== args.id);
        return null;
      case "google_status":
        return structuredClone(fake.google);
      case "set_google_client":
        if (!String(args.clientId).trim().endsWith(".apps.googleusercontent.com")) throw "this does not look like a Google client ID. It ends with .apps.googleusercontent.com";
        fake.google = { ...fake.google, client_id: String(args.clientId).trim(), secret_set: true };
        return structuredClone(fake.google);
      case "connect_google":
        await fake.onConnectGoogle?.();
        if (!fake.google.secret_set) throw "add the client ID and the client secret of your Google registration first";
        fake.google = { ...fake.google, connected: true, email: "me@x.com", features: [["Calendar", true]] };
        fake.calendar = fake.calendar ?? { fetched_at: nowSecs(), events: [] };
        return structuredClone(fake.google);
      case "disconnect_google":
        fake.google = { ...fake.google, connected: false, email: null, features: [["Calendar", false]] };
        fake.calendar = null;
        return structuredClone(fake.google);
      case "calendar_events":
        if (fake.calendarError) throw fake.calendarError;
        return structuredClone(fake.calendar);
      case "github_items": {
        // The real app uses a saved list when force is false. The fake
        // counts only the real GitHub calls, so tests can see the difference.
        if (args.force || fake.githubFetches === 0) {
          if (fake.githubError) throw fake.githubError;
          fake.githubFetches++;
        }
        const list = {
          items: structuredClone(fake.github),
          fetched_at: nowSecs(),
          warning: fake.githubWarning,
        };
        await fake.onGithubItems?.(Boolean(args.force));
        return list;
      }
      case "clickup_token_status":
        return { set: fake.clickupToken !== "", from_env: false };
      case "set_clickup_token":
        if (!String(args.token).startsWith("pk_")) throw "ClickUp refused the token. (Token invalid)";
        fake.clickupToken = args.token;
        return "okky";
      case "remove_clickup_token":
        fake.clickupToken = "";
        return null;
      case "clickup_items": {
        if (!fake.clickupToken) throw "ClickUp is not connected. Add a token on the Settings page.";
        if (args.force || fake.clickupFetches === 0) {
          if (fake.clickupError) throw fake.clickupError;
          fake.clickupFetches++;
        }
        return { items: structuredClone(fake.clickup), fetched_at: nowSecs(), warning: fake.clickupWarning };
      }
      case "clickup_mark_done": {
        const t = fake.clickup.find((c) => c.id === args.id);
        if (!t) throw `ClickUp said 404: no task ${args.id}`;
        fake.clickup = fake.clickup.filter((c) => c.id !== args.id);
        fake.clickupClosed[t.id] = t;
        return t.status;
      }
      case "clickup_undo_done": {
        const before = fake.clickupClosed[args.id];
        if (!before) throw `no closed task ${args.id}`;
        delete fake.clickupClosed[args.id];
        fake.clickup.push({ ...before, status: args.status });
        return null;
      }
      case "chat_model":
        return fake.chatModel;
      case "set_chat_model":
        if (args.model !== null && !["fable", "opus", "sonnet", "haiku"].includes(args.model)) throw `unknown model "${args.model}"`;
        fake.chatModel = args.model;
        return null;
      case "get_settings":
        return structuredClone(fake.settings);
      case "set_settings":
        if (args.settings.work_start >= args.settings.work_end) throw "the work day must end after it starts";
        fake.settings = structuredClone(args.settings);
        return null;
      case "ask": {
        // Send progress the way Rust does: through the channel's callback.
        const a = await fake.onAsk(args.prompt, progressOf(args), args);
        const files = (args.files ?? []).map((id: string) => findUpload(id));
        fake.history.push({ ts: nowSecs(), role: "user", text: args.prompt, backend: "fake", files });
        fake.history.push({ ts: nowSecs(), role: "assistant", text: a.text, backend: "fake" });
        // "default": the default model. No model: the pick that is saved.
        const model = args.model === undefined ? fake.chatModel : args.model === "default" ? null : args.model;
        return withInfo({ ...a, model });
      }
      case "ask_side": {
        if (!/^(plan:\d{4}-\d{2}-\d{2}|item:(task|github|clickup):.+)$/.test(args.thread)) throw `"${args.thread}" is not a chat name`;
        const a = await fake.onAskSide(args.prompt, progressOf(args), args);
        const files = (args.files ?? []).map((id: string) => findUpload(id));
        const list = (fake.side[args.thread] ??= []);
        list.push({ id: list.length + 1, ts: nowSecs(), role: "user", text: args.prompt, files });
        list.push({ id: list.length + 1, ts: nowSecs(), role: "assistant", text: a.text, files: [], info: "1.0s" });
        return withInfo({ ...a, model: args.model === "default" ? null : (args.model ?? null) });
      }
      case "side_history":
        return structuredClone(fake.side[args.thread] ?? []);
      case "clear_side":
        delete fake.side[args.thread];
        return null;
      case "upload_file": {
        const bytes = payload as unknown as Uint8Array;
        const f = { id: `f${nextId++}`, name: fake.uploadName, size: bytes.length };
        fake.uploads.push(f);
        return f;
      }
      case "open_file":
        findUpload(args.id);
        return null;
      case "day_plan":
        await fake.onDayPlan?.(args.day);
        return structuredClone(fake.plans[args.day] ?? null);
      case "set_day_plan": {
        if (!["ai", "self"].includes(args.mode)) throw `the plan mode must be "ai" or "self"`;
        // Like plan.rs: a key that was in the plan before stays, also when its item is not open.
        const old = new Set((fake.plans[args.day]?.items ?? []).map((i) => i.key));
        const seen = new Set<string>();
        const items = (args.items as DayPlan["items"]).filter((i) => !seen.has(i.key) && (seen.add(i.key), old.has(i.key) || isOpen(i.key)));
        fake.plans[args.day] = { day: args.day, mode: args.mode, items, updated: nowSecs() };
        return structuredClone(fake.plans[args.day]);
      }
      case "clear_day_plan":
        delete fake.plans[args.day];
        return null;
      case "estimates":
        return structuredClone(fake.estimates);
      case "set_estimate":
        if (args.minutes == null) delete fake.estimates[args.key];
        else fake.estimates[args.key] = { minutes: Math.min(600, Math.max(5, args.minutes)), by: "you" };
        return null;
      case "brief":
        return structuredClone(fake.briefs[args.day] ?? null);
      case "write_brief": {
        const b = await fake.onWriteBrief(args.day);
        fake.briefs[args.day] = b;
        return structuredClone(b);
      }
      case "jobs":
        return structuredClone(fake.jobs.filter((j) => args.all || j.status === "active" || j.status === "paused"));
      case "job_runs":
        return structuredClone(
          fake.jobRuns.filter((r) => (args.id == null || r.job === args.id) && (args.skipped || r.state !== "skipped")).slice(0, args.limit),
        );
      case "set_job_status": {
        const j = fake.jobs.find((x) => x.id === args.id);
        if (!j) throw `no job with id ${args.id}`;
        j.status = args.status;
        j.can_start = j.status !== "active";
        return structuredClone(j);
      }
      case "delete_job":
        fake.jobs = fake.jobs.filter((j) => j.id !== args.id);
        fake.jobRuns = fake.jobRuns.filter((r) => r.job !== args.id);
        return null;
      case "item_meta":
        return [
          ...fake.tasks.filter((t) => !t.done).map((t) => ({ key: `task:${t.id}`, first_seen: t.created, new: false })),
          ...fake.github.map((g) => ({ key: `github:${g.id}`, first_seen: nowSecs(), new: fake.newKeys.includes(`github:${g.id}`) })),
          ...fake.clickup.map((c) => ({ key: `clickup:${c.id}`, first_seen: nowSecs(), new: fake.newKeys.includes(`clickup:${c.id}`) })),
        ].map((m) => (fake.summaries[m.key] ? { ...m, summary: fake.summaries[m.key] } : m));
      case "item_details": {
        const d: ItemDetails = { key: args.key, details: fake.details[args.key] ?? "" };
        if (fake.summaries[args.key]) d.summary = fake.summaries[args.key];
        return d;
      }
      case "update_status":
        return fake.update;
      case "check_update":
        fake.update = fake.updateTo
          ? { ...fake.update, state: "ready", latest: fake.updateTo, page: `https://example.com/v${fake.updateTo}`, checked_at: nowSecs() }
          : { ...fake.update, state: "none", checked_at: nowSecs() };
        return fake.update;
      case "restart_to_update":
        if (fake.update.state !== "ready") throw "No downloaded update waits. Check for updates again.";
        fake.restarts++;
        return null;
      default:
        throw `fake backend: unknown command ${cmd}`;
    }
  });
  return fake;
}

/** A promise you can finish later, to hold AIfred in the "thinking" state. */
export function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
}

/** One of AIfred's jobs for a test: a weekly reminder unless `extra` says else. */
export function job(id: string, title: string, extra: Partial<Job> = {}): Job {
  return {
    id,
    title,
    action: "remind",
    message: "Submit the timesheet.",
    trigger: { kind: "repeat", start: "2030-11-04T09:00", rule: "FREQ=WEEKLY;BYDAY=MO" },
    tz: "Asia/Jakarta",
    status: "active",
    next_run: nowSecs() + 3600,
    created_by: "aifred",
    created: nowSecs(),
    updated: nowSecs(),
    failures: 0,
    when: "Every Monday at 09:00 (Asia/Jakarta)",
    can_start: (extra.status ?? "active") !== "active",
    ...extra,
  };
}
