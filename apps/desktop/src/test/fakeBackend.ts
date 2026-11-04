// A fake AIfred backend for window tests. It answers the same commands as
// the Rust app (src-tauri/src/main.rs), but keeps everything in memory.
// No Rust, no Claude, no GitHub, no cost.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Answer, ClickupItem, Fact, GithubItem, ProgressEvent, Settings, Task, TaskChanges, Turn } from "../api";

type AskHandler = (prompt: string, progress: (e: ProgressEvent) => void) => Promise<Answer> | Answer;

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
  /** The model picked in the chat box. null: the default. */
  chatModel: string | null;
  history: Turn[];
  /** All facts, also the forgotten ones. */
  memory: Fact[];
  /** Commands that fail, with the error text. Example: { tasks: "cannot read tasks.json" }. */
  fail: Record<string, string>;
  /** Every command the window called, in order. */
  calls: { cmd: string; args: Record<string, unknown> }[];
  onAsk: AskHandler;
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

export function answer(text: string, extra: Partial<Answer> = {}): Answer {
  return { text, stats: { startup_ms: 10, total_ms: 1500, tool_calls: [] }, model_ms: 1000, cost_usd: 0.005, memory: [], model: null, ...extra };
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
    settings: { refresh_minutes: 15 },
    chatModel: null,
    history: [],
    memory: [],
    fail: {},
    calls: [],
    onAsk: (p) => answer(`You said: ${p}`),
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
        fake.settings = structuredClone(args.settings);
        return null;
      case "ask": {
        // Send progress the way Rust does: through the channel's callback.
        let index = 0;
        const progress = (e: ProgressEvent) =>
          (window as any).__TAURI_INTERNALS__.runCallback(args.onEvent.id, { index: index++, message: e });
        const a = await fake.onAsk(args.prompt, progress);
        return { ...a, model: fake.chatModel };
      }
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
