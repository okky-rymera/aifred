// A fake AIfred backend for window tests. It answers the same commands as
// the Rust app (src-tauri/src/main.rs), but keeps everything in memory.
// No Rust, no Claude, no GitHub, no cost.
import { mockIPC } from "@tauri-apps/api/mocks";
import { DEFAULT_SETTINGS } from "../api";
import { MAX_GROUP, groupOf } from "../items";
import type {
  Action,
  DocEdit,
  SheetComment,
  Answer,
  Brief,
  CalEvent,
  CalendarList,
  ClickupItem,
  Correction,
  DayCheck,
  DayItem,
  DayPlan,
  Estimate,
  Fact,
  FileRef,
  Goals,
  GoogleStatus,
  GithubItem,
  ItemDetails,
  ItemLink,
  LinkSuggestion,
  Job,
  JobRun,
  Mail,
  MailList,
  ProgressEvent,
  ReviewPick,
  Running,
  Settings,
  SideTurn,
  StaleItem,
  Task,
  TaskChanges,
  Turn,
  UpdateStatus,
  Waiting,
  WorkDone,
  Step,
} from "../api";

/** What suggest_doc_changes gives. `canEdit` false: the user can only comment on the Doc. */
type DocAnswer = { edits: DocEdit[]; message?: string; canEdit?: boolean };
/** What suggest_sheet_comments gives. */
type SheetAnswer = { comments: SheetComment[]; message?: string };
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
  /** Runs after gmail_threads makes its list and before it answers. A test
   * can hold the answer with it, so the answer comes after a later change. */
  onGmailThreads?: () => Promise<void> | void;
  /** Runs before add_steps saves. A test can hold the save with it, like a slow disk. */
  onAddSteps?: () => Promise<void> | void;
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
  /** Gmail. null: Gmail is not connected, so no emails. Hidden threads are left out. */
  mail: MailList | null;
  mailError?: string;
  /** The threads that the user hid ("no answer needed"). */
  mailHidden: string[];
  /** The "waiting for" list, like waiting.rs. */
  waiting: Waiting[];
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
  /** What suggest_steps gives for an item. Throw to fail it. */
  onSuggestSteps: (key: string) => Promise<Step[]> | Step[];
  /** The text of the reply that write_reply gives for an email item. Throw to fail it. */
  onWriteReply: (key: string, note: string) => Promise<string> | string;
  /** The Doc link that work_links finds for an item, by key. */
  docLinks?: Record<string, string>;
  /** Runs before work_links answers: a test can hold the answer back. */
  beforeDocLink?: () => Promise<void>;
  /** How many times the window called work_links. */
  linkCalls?: number;
  /** What suggest_doc_changes gives: the changes and the message. Throw to fail it. */
  onSuggestDoc?: (key: string, link: string, note: string) => Promise<DocAnswer> | DocAnswer;
  /** The edits of the last save_doc_suggestions. */
  savedDocEdits?: DocEdit[];
  /** The Sheet link that work_links finds for an item, by key. */
  sheetLinks?: Record<string, string>;
  /** What suggest_sheet_comments gives. Throw to fail it. */
  onSuggestSheet?: (key: string, link: string, note: string) => Promise<SheetAnswer> | SheetAnswer;
  /** The comments of the last save_sheet_comments. */
  savedSheetComments?: SheetComment[];
  /** True: Google gives no comment ids, so the save closes its action at once (sheets.rs). */
  sheetNoIds?: boolean;
  /** AIfred's actions in other apps, like actions.rs, oldest first. */
  actions: Action[];
  /** True: the next Undo finds the draft gone from Gmail (sent or deleted). */
  draftGone?: boolean;
  /** Runs before a draft save, so a test can hold the save open. */
  beforeSave?: () => Promise<void>;
  /** Runs before an Undo of an action, so a test can hold the Undo open. */
  beforeUndo?: () => Promise<void>;
  /** How many Undo calls of actions came in. */
  undoCalls?: number;
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
  /** Corrections of the brief, like learn.rs. */
  corrections: Correction[];
  /** Like the learned table: the fact id, the time, and the text that AIfred wrote, by place. A place stays after its fact is forgotten. */
  learned: Record<string, { fact: number; ts: number; text: string }>;
  /** Work sessions, like worktime.rs. `ended` null: the running one. */
  sessions: { key: string; started: number; ended: number | null }[];
  /** Finished items with a real time, by key. */
  workDone: Record<string, WorkDone>;
  /** When set, finish_work fails with this text. */
  failFinishWork?: string;
  /** The clock of the work timer, in Unix seconds. A test can move it. */
  now: () => number;
  /** The finished end-of-day checks: the moved keys, by day (review.rs). */
  dayReviews: Record<string, string[]>;
  /** The days of the finished weekly reviews. */
  weekReviews: string[];
  /** The items with no change for 3 weeks. The fake does not count the weeks: a test sets them. */
  stale: StaleItem[];
  /** The weekly review shows at most this many items (review::STALE_MAX). */
  staleMax: number;
  /** The picks of the weekly review, with what the undo puts back. */
  picks: (ReviewPick & { undo: { plan?: string; start?: string | null; setStart?: boolean; waiting?: number; closed?: boolean } })[];
  /** The finished items of the last 7 days that served each goal, by goal id. */
  goalDone: Record<number, number>;
  /** The items under another item, like links.rs. */
  links: ItemLink[];
  /** AIfred's suggestions. The fake does not find them: a test sets them.
   * A suggestion of a linked or rejected pair is left out, like in links.rs. */
  linkSuggestions: LinkSuggestion[];
  /** The rejected pairs, as "a|b" with a < b. */
  linkNo: string[];
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

/** An email thread. `kind` "reply" waits for your answer; "sent" waits for an answer to you. */
export function mail(id: string, subject: string, kind: Mail["kind"], extra: Partial<Mail> = {}): Mail {
  const t = nowSecs() - 3 * 86_400;
  return { id, subject, kind, people: ["Ana Lee"], more_people: 0, email: "ana@y.com", last_at: t, since: t, snippet: "Can you send the price?", direct: true, unread: false, question: true, messages: 1, url: `https://mail.google.com/mail/#all/${id}`, ...extra };
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
    mail: null,
    mailHidden: [],
    waiting: [],
    google: { client_id: "", secret_set: false, connected: false, email: null, features: [["Calendar", false], ["Gmail", false]], optional: ["Gmail"], off: [] },
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
    onSuggestSteps: () => [{ text: "Read the item" }],
    onWriteReply: (_key, note) => `Hi Ana, thanks. ${note}`.trim(),
    actions: [],
    side: {},
    newKeys: [],
    summaries: {},
    details: {},
    uploads: [],
    uploadName: "file.txt",
    corrections: [],
    learned: {},
    sessions: [],
    workDone: {},
    now: nowSecs,
    dayReviews: {},
    weekReviews: [],
    stale: [],
    staleMax: 30,
    picks: [],
    goalDone: {},
    links: [],
    linkSuggestions: [],
    linkNo: [],
    ...init,
  };

  /** The links and the suggestions, like links::get. A rejected pair
   * blocks all pairs of its two groups, like in links.rs. */
  const linksNow = () => {
    const head = (k: string) => groupOf(fake.links, k)[0];
    const pairOf = (a: string, b: string) => [head(a), head(b)].sort().join("|");
    const no = new Set(fake.linkNo.map((p) => pairOf(...(p.split("|") as [string, string]))));
    return structuredClone({
      links: [...fake.links].sort((a, b) => a.main.localeCompare(b.main) || a.key.localeCompare(b.key)),
      suggestions: fake.linkSuggestions.filter((s) => head(s.main) !== head(s.key) && !no.has(pairOf(s.main, s.key))),
    });
  };
  /** Takes `key` out of its group, like links::unlink_in. */
  const unlink = (key: string) => {
    const g = groupOf(fake.links, key);
    if (g[0] !== key) fake.links = fake.links.filter((l) => l.key !== key);
    else {
      const [next, ...others] = g.slice(1);
      fake.links = fake.links.filter((l) => l.main !== key);
      for (const k of others) fake.links.push({ key: k, main: next });
    }
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
    /** Like items::title_place in Rust. */
    const titlePlace = (key: string): [string, string] | null => {
      const t = fake.tasks.find((t) => `task:${t.id}` === key);
      if (t) return [t.title, t.tags[0] ? `Your tasks with #${t.tags[0]}` : ""];
      const g = fake.github.find((g) => `github:${g.id}` === key);
      if (g) return [g.title, `GitHub ${g.repo}`];
      const c = fake.clickup.find((c) => `clickup:${c.id}` === key);
      if (c) return [c.title, `ClickUp ${c.workspace} / ${c.list}`];
      const m = fake.mail?.reply.find((m) => `gmail:${m.id}` === key);
      // Like items::title_place: an item with no known place has the place "".
      const p = m?.people[0]?.trim();
      if (m) return [m.subject, p ? `Email from ${p}` : ""];
      return null;
    };
    /** Like learn.rs: the different items with a "wrong priority" correction from `place`, in the 60 days before `now`. */
    const counted = (place: string, now: number) =>
      new Set(fake.corrections.filter((x) => x.kind === "wrong_priority" && x.place === place && x.ts >= now - 60 * 86400).map((x) => x.key)).size;
    /** Like learn::fact_text. */
    const learnedText = (place: string, n: number) =>
      `${place}: these items are less important to the user than they look. Rank them lower. (AIfred learned this from ${n} "wrong priority" corrections in the brief.)`;
    const running = (): Running | null => {
      const r = fake.sessions.find((x) => x.ended === null);
      if (!r) return null;
      // Like worktime::running_in: a forgotten timer (more than 10 hours) does not count.
      const before = fake.sessions
        .filter((x) => x.key === r.key && x.ended !== null && x.ended - x.started <= 10 * 3600)
        .reduce((n, x) => n + x.ended! - x.started, 0);
      return { key: r.key, title: titlePlace(r.key)?.[0] ?? r.key, started: r.started, before };
    };
    /** Like worktime::finish_in in Rust. A session of more than 10 hours does not count. */
    const finishWork = (key: string): WorkDone | null => {
      const r = fake.sessions.find((x) => x.ended === null);
      if (r && r.key === key) r.ended = fake.now();
      let secs = 0;
      let skipped = 0;
      for (const x of fake.sessions.filter((x) => x.key === key && x.ended !== null)) {
        const n = x.ended! - x.started;
        if (n > 10 * 3600) skipped++;
        else secs += n;
      }
      if (!secs && !skipped) return null;
      const d: WorkDone = { key, title: titlePlace(key)?.[0] ?? key, minutes: secs ? Math.max(1, Math.round(secs / 60)) : 0, guess: fake.estimates[key]?.minutes ?? null, skipped };
      if (secs) fake.workDone[key] = d;
      return d;
    };
    /** Like review.rs: the day after "YYYY-MM-DD". */
    const nextDay = (day: string) => {
      const d = new Date(`${day}T12:00:00`);
      d.setDate(d.getDate() + 1);
      return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    };
    const localDay = (secs: number) => {
      const d = new Date(secs * 1000);
      return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, "0")}-${String(d.getDate()).padStart(2, "0")}`;
    };
    /** Keys that the user gave away or dropped in a weekly review. */
    const hidden = () => new Set(fake.picks.filter((p) => p.pick === "delegate" || p.pick === "drop").map((p) => p.key));
    const dayItem = (key: string, minutes?: number): DayItem => {
      const [title, place] = titlePlace(key) ?? [key, ""];
      return minutes ? { key, title, place, minutes } : { key, title, place };
    };
    /** Like review::day_check_in. */
    const dayCheck = (day: string): DayCheck => {
      const h = hidden();
      const done = fake.tasks
        .filter((t) => t.done && t.done_at && localDay(t.done_at) === day && !h.has(`task:${t.id}`))
        .map((t) => dayItem(`task:${t.id}`, fake.workDone[`task:${t.id}`]?.minutes));
      const doneKeys = new Set(done.map((d) => d.key));
      const keys = [
        ...(fake.plans[day]?.items ?? []).map((i) => i.key),
        ...fake.tasks.filter((t) => !t.done && t.due === day && (!t.start || t.start <= day)).map((t) => `task:${t.id}`),
      ];
      const left = [...new Set(keys)].filter((k) => !h.has(k) && !doneKeys.has(k) && isOpen(k)).map((k) => dayItem(k));
      return { day, done, left, finished: day in fake.dayReviews, moved: fake.dayReviews[day] ?? [] };
    };
    /** Like review::add_to_plan. True when the key was in the plan. */
    const addToPlan = (day: string, key: string) => {
      const p = fake.plans[day] ?? { day, mode: "review" as const, items: [], updated: nowSecs() };
      if (p.items.some((i) => i.key === key)) return true;
      fake.plans[day] = { ...p, items: [...p.items, { key }], updated: nowSecs() };
      return false;
    };
    const weekReview = (today: string) => {
      const last = fake.weekReviews.length ? [...fake.weekReviews].sort().at(-1)! : null;
      const days = (a: string, b: string) => Math.round((new Date(`${b}T12:00:00`).getTime() - new Date(`${a}T12:00:00`).getTime()) / 86_400_000);
      const due = last ? days(last, today) >= 7 : new Date(`${today}T12:00:00`).getDay() % 6 === 0 || new Date(`${today}T12:00:00`).getDay() === 5;
      const h = hidden();
      const picked = new Set(fake.picks.map((p) => p.key));
      const all = fake.stale.filter((i) => !h.has(i.key) && !picked.has(i.key) && isOpen(i.key));
      return {
        due,
        last,
        items: structuredClone(all.slice(0, fake.staleMax)),
        more: Math.max(0, all.length - fake.staleMax),
        goals: fake.goals.goals.map((g) => ({ id: g.id, text: g.text, done: fake.goalDone[g.id] ?? 0 })),
        goals_stale: fake.goals.stale,
      };
    };
    switch (cmd) {
      case "day_check":
        return dayCheck(args.day);
      case "finish_day": {
        const next = nextDay(args.day);
        const moved: string[] = [];
        for (const k of args.moved as string[]) {
          if (!isOpen(k)) continue;
          addToPlan(next, k);
          const t = fake.tasks.find((t) => `task:${t.id}` === k);
          if (t && t.due === args.day) t.due = next;
          moved.push(k);
        }
        fake.dayReviews[args.day] = [...new Set([...(fake.dayReviews[args.day] ?? []), ...moved])];
        return dayCheck(args.day);
      }
      case "week_review":
        return weekReview(args.today);
      case "finish_week_review":
        fake.weekReviews.push(args.today);
        return weekReview(args.today);
      case "review_pick": {
        const { key, pick, today } = args as { key: string; pick: string; today: string };
        const note = String(args.note).split(/\s+/).filter(Boolean).join(" ");
        if (!["do", "plan", "delegate", "drop"].includes(pick)) throw `a pick is "do", "plan", "delegate", or "drop", not "${pick}"`;
        if (!isOpen(key)) throw `the item ${key} is not open any more`;
        const [title, place] = titlePlace(key) ?? [key, ""];
        const t = fake.tasks.find((t) => `task:${t.id}` === key);
        const undo: Fake["picks"][number]["undo"] = {};
        if (pick === "do" && !addToPlan(today, key)) undo.plan = today;
        if (pick === "plan") {
          if (!/^\d{4}-\d{2}-\d{2}$/.test(note)) throw `the day "${note}" is not in the form YYYY-MM-DD`;
          if (note <= today) throw `pick a day after today. To work on it today, pick "do"`;
          if (!addToPlan(note, key)) undo.plan = note;
          if (t && (!t.due || t.due >= note)) {
            undo.setStart = true;
            undo.start = t.start;
            t.start = note;
          }
        }
        if (pick === "delegate") {
          if (!note) throw "say who does it";
          const id = fake.waiting.reduce((a, x) => Math.max(a, x.id), 0) + 1;
          fake.waiting.push({ id, what: title, who: note, since: today, follow_up: null, told: false, thread: null, url: null, done: false, done_why: "", created: nowSecs(), updated: nowSecs() });
          undo.waiting = id;
        }
        if ((pick === "delegate" || pick === "drop") && t) {
          t.done = true;
          t.done_at = nowSecs();
          undo.closed = true;
        }
        const p = { id: nextId++, key, pick: pick as ReviewPick["pick"], title, place, note: pick === "plan" || pick === "delegate" ? note : "", ts: nowSecs(), undo };
        fake.picks.push(p);
        const { undo: _, ...out } = p;
        return structuredClone(out);
      }
      case "undo_review_pick": {
        const p = fake.picks.find((x) => x.id === args.id);
        if (!p) throw `no pick with id ${args.id}`;
        const t = fake.tasks.find((t) => `task:${t.id}` === p.key);
        if (p.undo.closed && t) {
          t.done = false;
          t.done_at = null;
        }
        if (p.undo.plan && fake.plans[p.undo.plan]) {
          const pl = fake.plans[p.undo.plan];
          const items = pl.items.filter((i) => i.key !== p.key);
          if (pl.mode === "review" && !items.length) delete fake.plans[p.undo.plan];
          else fake.plans[p.undo.plan] = { ...pl, items };
        }
        if (p.undo.setStart && t && t.start === p.note) t.start = p.undo.start ?? null;
        if (p.undo.waiting) fake.waiting = fake.waiting.filter((w) => w.id !== p.undo.waiting);
        fake.picks = fake.picks.filter((x) => x !== p);
        return null;
      }
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
      case "remove_task_steps": {
        // Like tasks.rs: only the steps that are not done go.
        const t = find(args.id);
        const gone = new Set(args.steps as string[]);
        t.checklist = t.checklist.filter((c) => c.done || !gone.has(c.text));
        return structuredClone(t);
      }
      case "set_task_done": {
        const t = find(args.id);
        // Like tasks.rs: the work of a task ends with it, or opens again.
        let work: WorkDone | null = null;
        if (args.done && !t.done) work = finishWork(`task:${t.id}`);
        else if (!args.done && t.done) delete fake.workDone[`task:${t.id}`];
        t.done = args.done;
        t.done_at = args.done ? nowSecs() : null;
        return { task: t, work: structuredClone(work) };
      }
      case "delete_task":
        find(args.id);
        fake.tasks = fake.tasks.filter((t) => t.id !== args.id);
        fake.sessions = fake.sessions.filter((x) => x.key !== `task:${args.id}`);
        // Like tasks.rs: a deleted task leaves its group.
        unlink(`task:${args.id}`);
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
        {
          // The login does not ask for a feature that is turned off.
          const gmail = !fake.google.off.includes("Gmail");
          fake.google = { ...fake.google, connected: true, email: "me@x.com", features: [["Calendar", true], ["Gmail", gmail]] };
          fake.calendar = fake.calendar ?? { fetched_at: nowSecs(), events: [] };
          fake.mail = gmail ? (fake.mail ?? { fetched_at: nowSecs(), reply: [], sent: [] }) : null;
        }
        return structuredClone(fake.google);
      case "disconnect_google":
        fake.google = { ...fake.google, connected: false, email: null, features: [["Calendar", false], ["Gmail", false]] };
        fake.calendar = null;
        fake.mail = null;
        return structuredClone(fake.google);
      case "calendar_events":
        if (fake.calendarError) throw fake.calendarError;
        return structuredClone(fake.calendar);
      case "set_google_feature": {
        const name = String(args.name);
        if (!fake.google.optional.includes(name)) throw `${name} cannot be turned off`;
        const off = fake.google.off.filter((x) => x !== name);
        if (!args.on) off.push(name);
        fake.google = { ...fake.google, off };
        if (!args.on && name === "Gmail") fake.mail = null;
        return structuredClone(fake.google);
      }
      case "gmail_saved":
      case "gmail_threads": {
        if (fake.google.off.includes("Gmail")) return null;
        // The saved list needs no download, so a download error does not stop it.
        if (cmd === "gmail_threads" && fake.mailError) throw fake.mailError;
        if (!fake.mail) return null;
        const tracked = new Set(fake.waiting.filter((w) => !w.done && w.thread).map((w) => w.thread));
        const shown = (m: Mail) => !fake.mailHidden.includes(m.id);
        const list = structuredClone({
          ...fake.mail,
          reply: fake.mail.reply.filter(shown),
          sent: fake.mail.sent.filter((m) => shown(m) && !tracked.has(m.id)),
        });
        if (cmd === "gmail_threads") await fake.onGmailThreads?.();
        return list;
      }
      case "hide_email": {
        const id = String(args.id);
        fake.mailHidden = fake.mailHidden.filter((x) => x !== id);
        if (args.hide) fake.mailHidden.push(id);
        return null;
      }
      case "write_reply": {
        const key = String(args.key);
        const m = fake.mail?.reply.find((x) => `gmail:${x.id}` === key);
        if (!m) throw "AIfred can write a reply only for an email";
        const body = await fake.onWriteReply(key, String(args.note));
        const canSave = fake.google.features.some(([n, on]) => n === "Gmail drafts" && on) && !fake.google.off.includes("Gmail drafts");
        return { key, to: m.email, to_name: m.people[0] ?? m.email, subject: `Re: ${m.subject}`, body, last_message: `${m.id}-last`, can_save: canSave };
      }
      case "save_reply_draft": {
        await fake.beforeSave?.();
        const key = String(args.key);
        const m = fake.mail?.reply.find((x) => `gmail:${x.id}` === key);
        if (!m) throw "this email is not in Gmail any more";
        if (args.to !== m.email || args.lastMessage !== `${m.id}-last`) throw "the email changed after AIfred wrote the reply. Write the reply again";
        if (!String(args.body).trim()) throw "the reply is empty";
        const a: Action = {
          id: fake.actions.length + 1,
          at: nowSecs(),
          app: "gmail",
          kind: "draft",
          item: key,
          title: `Reply draft to ${m.email}: Re: ${m.subject}`,
          after: String(args.body).trim(),
          url: m.url,
          undone_at: null,
          note: "",
        };
        fake.actions.push(a);
        return structuredClone(a);
      }
      case "work_links": {
        fake.linkCalls = (fake.linkCalls ?? 0) + 1;
        await fake.beforeDocLink?.();
        const key = String(args.key);
        return { doc: fake.docLinks?.[key] ?? null, sheet: fake.sheetLinks?.[key] ?? null };
      }
      case "suggest_doc_changes": {
        const key = String(args.key);
        const link = String(args.link);
        // Like docs::doc_id.
        const id = /\/document\/d\/([\w-]{25,})/.exec(link)?.[1];
        if (!id) throw "this is not the link of a Google Doc (docs.google.com/document/d/…)";
        // Like docs::connect: no read before you turn on Docs suggestions.
        const on = fake.google.features.some(([n, on]) => n === "Docs suggestions" && on) && !fake.google.off.includes("Docs suggestions");
        if (!on) throw 'AIfred cannot use Google Docs yet. In Settings, tick "Use Docs suggestions", then connect Google again';
        const r: DocAnswer = fake.onSuggestDoc ? await fake.onSuggestDoc(key, link, String(args.note)) : { edits: [] };
        return { key, doc: id, tab: "", title: "Offer", url: `https://docs.google.com/document/d/${id}/edit`, revision: "r1", can_edit: r.canEdit ?? true, edits: r.edits, message: r.message ?? "" };
      }
      case "save_doc_suggestions": {
        await fake.beforeSave?.();
        if (args.revision !== "r1") throw "the Doc changed after AIfred read it. Suggest the changes again";
        const edits = structuredClone(args.edits as DocEdit[]);
        if (!edits.length) throw "there are no changes to save";
        fake.savedDocEdits = edits;
        const n = edits.length;
        const a: Action = {
          id: fake.actions.length + 1,
          at: nowSecs(),
          app: "docs",
          kind: "suggestion",
          item: String(args.key),
          title: `Suggested ${n} change${n === 1 ? "" : "s"} in the Doc "Offer"`,
          after: edits.map((e) => `− ${e.find}\n+ ${e.replace}`).join("\n\n"),
          url: `https://docs.google.com/document/d/${args.doc}/edit`,
          undone_at: null,
          note: "",
        };
        fake.actions.push(a);
        return structuredClone(a);
      }
      case "suggest_sheet_comments": {
        const key = String(args.key);
        const link = String(args.link);
        // Like sheets::sheet_id and sheets::gid.
        const id = /\/spreadsheets\/d\/([\w-]{25,})/.exec(link)?.[1];
        if (!id) throw "this is not the link of a Google Sheet (docs.google.com/spreadsheets/d/…)";
        const gid = Number(/[#?&]gid=(\d+)/.exec(link)?.[1] ?? 0);
        // Like sheets::connect: no read before you turn on Sheets comments.
        const on = fake.google.features.some(([n, on]) => n === "Sheets comments" && on) && !fake.google.off.includes("Sheets comments");
        if (!on) throw 'AIfred cannot use Google Sheets yet. In Settings, tick "Use Sheets comments", then connect Google again';
        const r: SheetAnswer = fake.onSuggestSheet ? await fake.onSuggestSheet(key, link, String(args.note)) : { comments: [] };
        return { key, sheet: id, gid, tab: "Q4", title: "Prices", url: `https://docs.google.com/spreadsheets/d/${id}/edit#gid=${gid}`, comments: r.comments, message: r.message ?? "", check: "tab-hash" };
      }
      case "save_sheet_comments": {
        await fake.beforeSave?.();
        const comments = structuredClone(args.comments as SheetComment[]);
        if (!comments.length) throw "there are no comments to save";
        // Like sheets::save: the window sends the hash of the tab that it showed.
        if (args.check !== "tab-hash") throw "the Sheet changed after AIfred read it. Suggest the comments again";
        fake.savedSheetComments = comments;
        const n = comments.length;
        const a: Action = {
          id: fake.actions.length + 1,
          at: nowSecs(),
          app: "sheets",
          kind: "comment",
          item: String(args.key),
          title: `Commented on ${n} cell${n === 1 ? "" : "s"} in the Sheet "Prices"${fake.sheetNoIds ? ". Google gave no ids, so Undo cannot remove them" : ""}`,
          after: comments.map((c) => `${c.cell} (now: ${c.value || "empty"})\n${c.text}`).join("\n\n"),
          url: `https://docs.google.com/spreadsheets/d/${args.sheet}/edit#gid=${args.gid}`,
          undone_at: fake.sheetNoIds ? nowSecs() : null,
          note: fake.sheetNoIds ? "AIfred removed nothing: Google gave no ids for these comments. Delete them in the Sheet." : "",
        };
        fake.actions.push(a);
        return structuredClone(a);
      }
      case "actions":
        return structuredClone(
          fake.actions
            .filter((a) => args.item == null || a.item === args.item)
            .slice()
            .reverse(),
        );
      case "undo_action": {
        fake.undoCalls = (fake.undoCalls ?? 0) + 1;
        await fake.beforeUndo?.();
        const a = fake.actions.find((x) => x.id === args.id);
        if (!a) throw "there is no such action";
        if (a.undone_at !== null) throw "this action is undone already";
        a.undone_at = nowSecs();
        // Like drafts::undo: a draft that is not in Gmail any more is not deleted.
        if (fake.draftGone) a.note = "AIfred deleted nothing: the draft was not in Gmail any more. You sent it or deleted it.";
        return structuredClone(a);
      }
      case "suggest_steps":
        if (!titlePlace(String(args.key))) throw `no item ${args.key}`;
        return structuredClone(await fake.onSuggestSteps(String(args.key)));
      case "add_steps": {
        await fake.onAddSteps?.();
        // Like steps::add: the open task of the group, else a new task under the main item.
        const key = String(args.key);
        // Like steps::clean: one line, no empty step, no step twice.
        const seen = new Set<string>();
        const steps = (args.steps as string[])
          .map((x) => x.split(/\s+/).filter(Boolean).join(" "))
          .filter((x) => x && !seen.has(x.toLowerCase()) && seen.add(x.toLowerCase()));
        if (!steps.length) throw "no steps to add";
        if (steps.length > 8) throw "at most 8 steps can be added at one time";
        const g = groupOf(fake.links, key);
        const order = [key, ...g.filter((k) => k !== key)];
        const t = order.map((k) => fake.tasks.find((x) => `task:${x.id}` === k && !x.done)).find(Boolean);
        if (t) {
          let added = 0;
          const has = new Set(t.checklist.map((c) => c.text.toLowerCase()));
          for (const text of steps) {
            if (has.has(text.toLowerCase())) continue;
            has.add(text.toLowerCase());
            t.checklist.push({ text, done: false });
            added++;
          }
          return { task: structuredClone(t), added, created: false };
        }
        const item = titlePlace(key);
        if (!item) throw `no item ${key}`;
        const n = task(item[0], { added_by: "aifred", checklist: steps.map((text) => ({ text, done: false })) });
        fake.tasks.push(n);
        fake.links.push({ key: `task:${n.id}`, main: g[0] });
        return { task: structuredClone(n), added: steps.length, created: true };
      }
      case "item_links":
        return linksNow();
      case "link_items": {
        // Like links::link_in.
        const { main, key } = args as { main: string; key: string };
        if (main === key) throw "an item cannot be linked to itself";
        for (const k of [main, key]) if (!titlePlace(k)) throw `AIfred does not know the item ${k}`;
        const [gm, gk] = [groupOf(fake.links, main), groupOf(fake.links, key)];
        const target = gm[0] === gk[0] ? main : gm[0];
        const all = new Set([...gm, ...gk]);
        if (all.size > MAX_GROUP) throw `one piece of work can have at most ${MAX_GROUP} items`;
        fake.links = fake.links.filter((l) => !all.has(l.key));
        for (const k of [...all].sort()) if (k !== target) fake.links.push({ key: k, main: target });
        return linksNow();
      }
      case "unlink_item":
        unlink(String(args.key));
        return linksNow();
      case "reject_link": {
        const pair = [String(args.a), String(args.b)].sort().join("|");
        fake.linkNo = fake.linkNo.filter((p) => p !== pair);
        if (args.no) fake.linkNo.push(pair);
        return linksNow();
      }
      case "waiting_items":
        return structuredClone(fake.waiting);
      case "add_waiting": {
        const n = args.item as { what?: string; who?: string; since?: string; follow_up?: string; thread?: string };
        let { what = "", who = "" } = n;
        let url: string | null = null;
        if (n.thread) {
          const m = fake.mail?.sent.find((x) => x.id === n.thread);
          if (!m) throw `"${n.thread}" is not an email that waits for an answer`;
          what ||= `An answer to "${m.subject}"`;
          who ||= m.people.join(", ");
          url = m.url;
        }
        if (!what.trim()) throw "what is empty";
        if (!who.trim()) throw "who is empty";
        const w: Waiting = {
          id: fake.waiting.reduce((a, x) => Math.max(a, x.id), 0) + 1,
          what: what.trim(),
          who: who.trim(),
          since: n.since ?? new Date().toISOString().slice(0, 10),
          follow_up: n.follow_up ?? null,
          told: false,
          thread: n.thread ?? null,
          url,
          done: false,
          done_why: "",
          created: nowSecs(),
          updated: nowSecs(),
        };
        fake.waiting.push(w);
        return structuredClone(w);
      }
      case "update_waiting": {
        const w = fake.waiting.find((x) => x.id === args.id);
        if (!w) throw `no "waiting for" item with id ${args.id}`;
        const c = args.changes as { what?: string; who?: string; follow_up?: string; done?: boolean };
        if (c.what !== undefined) w.what = c.what;
        if (c.who !== undefined) w.who = c.who;
        if (c.follow_up !== undefined) w.follow_up = c.follow_up || null;
        if (c.done !== undefined) {
          w.done = c.done;
          w.done_why = c.done ? "closed by you" : "";
        }
        w.updated = nowSecs();
        return structuredClone(w);
      }
      case "delete_waiting":
        fake.waiting = fake.waiting.filter((x) => x.id !== args.id);
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
        if (!["ai", "self", "review"].includes(args.mode)) throw `the plan mode must be "ai" or "self"`;
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
      case "brief": {
        // Like brief::get: an item corrected on that day has no reason and no goal.
        const b = structuredClone(fake.briefs[args.day] ?? null);
        for (const c of fake.corrections.filter((c) => c.day === args.day)) {
          if (b) delete b.why[c.key];
          if (b?.goal) delete b.goal[c.key];
          if (b?.next) delete b.next[c.key];
          if (b?.draft) delete b.draft[c.key];
        }
        return b;
      }
      case "correct_brief": {
        const item = titlePlace(args.key);
        if (!item) throw `AIfred does not know the item ${args.key}`;
        const old = fake.corrections.find((c) => c.key === args.key && c.kind === args.kind && c.day === args.day);
        if (old) return { correction: structuredClone(old) };
        const c: Correction = { id: nextId++, key: args.key, kind: args.kind, day: args.day, title: item[0], place: item[1], ts: nowSecs() };
        fake.corrections.push(c);
        // Like learn.rs: 3 different items from one place in 60 days make one
        // fact. AIfred learns each place once.
        if (c.kind === "wrong_priority" && c.place && !fake.learned[c.place]) {
          const n = counted(c.place, c.ts);
          if (n >= 3) {
            const f = fact(learnedText(c.place, n), { added_by: "aifred" });
            fake.memory.push(f);
            fake.learned[c.place] = { fact: f.id, ts: c.ts, text: f.text };
            return { correction: structuredClone(c), learned: structuredClone(f) };
          }
        }
        return { correction: structuredClone(c) };
      }
      case "undo_correction": {
        const c = fake.corrections.find((x) => x.id === args.id);
        if (!c) throw `no correction with id ${args.id}`;
        fake.corrections = fake.corrections.filter((x) => x !== c);
        // Like learn::undo: the fact goes when fewer than 3 corrections are
        // left, and the user did not change or forget it.
        const l = c.kind === "wrong_priority" && c.place ? fake.learned[c.place] : undefined;
        if (l && counted(c.place, l.ts) < 3) {
          const f = fake.memory.find((x) => x.id === l.fact);
          if (f && f.forgotten === null && f.text === l.text) {
            f.forgotten = nowSecs();
            delete fake.learned[c.place];
          }
        }
        return null;
      }
      case "work_running":
        return running();
      case "start_work": {
        if (!titlePlace(args.key)) throw `AIfred does not know the item ${args.key}`;
        const r = fake.sessions.find((x) => x.ended === null);
        if (r?.key === args.key) return running();
        if (r) r.ended = fake.now();
        fake.sessions.push({ key: args.key, started: fake.now(), ended: null });
        return running();
      }
      case "stop_work":
        for (const x of fake.sessions) if (x.ended === null) x.ended = fake.now();
        return null;
      case "finish_work":
        if (fake.failFinishWork) throw fake.failFinishWork;
        return structuredClone(finishWork(args.key));
      case "unfinish_work":
        delete fake.workDone[args.key];
        return null;
      case "write_brief": {
        const b = await fake.onWriteBrief(args.day);
        fake.briefs[args.day] = b;
        return structuredClone(b);
      }
      case "jobs":
        return structuredClone(fake.jobs.filter((j) => args.all || j.status === "active" || j.status === "paused"));
      case "brief_job": {
        const j = fake.jobs.find((x) => x.action === "brief");
        if (!j) throw "no job with id brief-on-task-done";
        return structuredClone(j);
      }
      case "job_runs":
        return structuredClone(
          fake.jobRuns
            .filter((r) => (args.id == null || r.job === args.id) && (args.skipped || r.state !== "skipped"))
            .filter((r) => args.brief || fake.jobs.find((j) => j.id === r.job)?.action !== "brief")
            .slice(0, args.limit),
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
  }, { shouldMockEvents: true });
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
