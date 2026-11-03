// A fake aifred backend for window tests. It answers the same commands as
// the Rust app (src-tauri/src/main.rs), but keeps everything in memory.
// No Rust, no Claude, no GitHub, no cost.
import { mockIPC } from "@tauri-apps/api/mocks";
import type { Answer, DoneItem, GithubItem, ProgressEvent, Settings, Task, Turn } from "../api";

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
  githubDone: DoneItem[];
  /** Items that are already closed on GitHub, by id. */
  githubClosed: string[];
  /** How many times the window asked GitHub (not the saved list). */
  githubFetches: number;
  settings: Settings;
  history: Turn[];
  memory: string;
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
  return { id: `t${n}`, title, notes: "", done: false, added_by: "you", created: nowSecs() + n, done_at: null, ...extra };
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

export function answer(text: string, extra: Partial<Answer> = {}): Answer {
  return { text, stats: { startup_ms: 10, total_ms: 1500, tool_calls: [] }, model_ms: 1000, cost_usd: 0.005, ...extra };
}

export function fakeBackend(init: Partial<Fake> = {}): Fake {
  const fake: Fake = {
    tasks: [],
    github: [],
    githubDone: [],
    githubClosed: [],
    githubFetches: 0,
    settings: { github_refresh_minutes: 15 },
    history: [],
    memory: "",
    fail: {},
    calls: [],
    onAsk: (p) => answer(`You said: ${p}`),
    ...init,
  };

  mockIPC(async (cmd, payload) => {
    const args = (payload ?? {}) as Record<string, any>;
    fake.calls.push({ cmd, args });
    if (cmd in fake.fail) throw fake.fail[cmd];
    const find = (id: string) => {
      const t = fake.tasks.find((t) => t.id === id);
      if (!t) throw `no task with id ${id}`;
      return t;
    };
    switch (cmd) {
      case "info":
        return { data_dir: "/fake/aifred", backend: "Claude (your login)", github_owners: null, api_key_set: false };
      case "history":
        return fake.history;
      case "get_memory":
        return fake.memory;
      case "set_memory":
        fake.memory = args.text;
        return null;
      case "new_conversation":
        fake.history = [];
        return null;
      case "tasks":
        return structuredClone(fake.tasks);
      case "add_task": {
        if (!String(args.title).trim()) throw "the task title is empty";
        const t = task(String(args.title).trim());
        fake.tasks.push(t);
        return t;
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
        const doneIds = fake.githubDone.map((d) => d.id);
        const list = {
          items: fake.github.filter((g) => !doneIds.includes(g.id)),
          done: structuredClone(fake.githubDone),
          fetched_at: nowSecs(),
          warning: fake.githubWarning,
        };
        await fake.onGithubItems?.(Boolean(args.force));
        return list;
      }
      case "github_mark_done": {
        const g = fake.github.find((g) => g.id === args.id);
        if (!g) throw `${args.id} is not in the GitHub list. Reload the list.`;
        if (!fake.githubDone.some((d) => d.id === g.id)) fake.githubDone.push({ ...g, done_at: nowSecs() });
        const open = !fake.githubClosed.includes(g.id);
        return {
          still_open: open,
          can_close: open && g.reason !== "review_requested",
          is_pull_request: g.url.includes("/pull/"),
        };
      }
      case "github_close":
        if (!fake.githubDone.some((d) => d.id === args.id)) throw `${args.id} is not marked done`;
        fake.githubClosed.push(args.id);
        fake.github = fake.github.filter((g) => g.id !== args.id);
        return null;
      case "github_undo":
        fake.githubDone = fake.githubDone.filter((d) => d.id !== args.id);
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
        return fake.onAsk(args.prompt, progress);
      }
      default:
        throw `fake backend: unknown command ${cmd}`;
    }
  });
  return fake;
}

/** A promise you can finish later, to hold aifred in the "thinking" state. */
export function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((a, b) => {
    resolve = a;
    reject = b;
  });
  return { promise, resolve, reject };
}
