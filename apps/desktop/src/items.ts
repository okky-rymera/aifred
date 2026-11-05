// One list of work items from all sources: your tasks, GitHub, and ClickUp.
// The item key is the same as in the Rust app (items.rs): "task:<id>",
// "github:<id>", or "clickup:<id>". The plan, the time guesses, the brief,
// and the item chats use it.
import type * as api from "./api";

export type Src = "me" | "gh" | "cu";
export const SRC_NAME: Record<Src, string> = { me: "Mine", gh: "GitHub", cu: "ClickUp" };

/** A time guess when there is none: 30 minutes. */
export const DEFAULT_MINUTES = 30;

export type Item = {
  key: string;
  src: Src;
  title: string;
  /** Where it is: "shop #7", "Rymera · Sprint 12", or "" for your own task. */
  ref: string;
  url?: string;
  /** "YYYY-MM-DD". */
  due: string | null;
  start: string | null;
  priority: api.Priority | null;
  tags: string[];
  done: boolean;
  /** Came in the last day (GitHub and ClickUp only). */
  isNew: boolean;
  minutes: number;
  /** Who set the time guess. null: no guess, so DEFAULT_MINUTES. */
  minutesBy: "you" | "aifred" | null;
  /** Why it needs you, from the brief. */
  why?: string;
  /** The summary of a long description, by AIfred. */
  summary?: string;
  task?: api.Task;
  gh?: api.GithubItem;
  cu?: api.ClickupItem;
};

export const taskKey = (id: string) => `task:${id}`;

/** "YYYY-MM-DD" of a time in your own time zone. */
export function dayOf(d: Date): string {
  const two = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${two(d.getMonth() + 1)}-${two(d.getDate())}`;
}

/** A Date at noon of a "YYYY-MM-DD" day, so a time zone change does not move it to another day. */
export function dateOf(day: string): Date {
  const [y, m, d] = day.split("-").map(Number);
  return new Date(y, m - 1, d, 12);
}

export function addDays(day: string, n: number): string {
  const d = dateOf(day);
  d.setDate(d.getDate() + n);
  return dayOf(d);
}

/** Days from `from` to `to`. Negative when `to` is earlier. */
export function daysBetween(from: string, to: string): number {
  const utc = (s: string) => {
    const [y, m, d] = s.split("-").map(Number);
    return Date.UTC(y, m - 1, d);
  };
  return Math.round((utc(to) - utc(from)) / 86400_000);
}

const WEEKDAYS = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
const MONTHS = ["Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec"];
const LONG_WEEKDAYS = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];
const LONG_MONTHS = ["January", "February", "March", "April", "May", "June", "July", "August", "September", "October", "November", "December"];

/** "Fri 6 Nov", and the year when it is not this year. */
export function shortDay(day: string, today: string): string {
  const d = dateOf(day);
  const year = day.slice(0, 4) === today.slice(0, 4) ? "" : ` ${d.getFullYear()}`;
  return `${WEEKDAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}${year}`;
}

/** "Wednesday, 4 November". */
export function longDay(day: string): string {
  const d = dateOf(day);
  return `${LONG_WEEKDAYS[d.getDay()]}, ${d.getDate()} ${LONG_MONTHS[d.getMonth()]}`;
}

/** The text and the color of a due day. */
export function dueInfo(day: string, today: string): { text: string; cls: "" | "late" | "today" } {
  const n = daysBetween(today, day);
  if (n === 0) return { text: "Today", cls: "today" };
  if (n === 1) return { text: "Tomorrow", cls: "" };
  if (n === -1) return { text: "Due yesterday", cls: "late" };
  if (n < 0) return { text: `${-n} days late`, cls: "late" };
  return { text: shortDay(day, today), cls: "" };
}

const RANK: Record<api.Priority, number> = { urgent: 0, high: 1, normal: 2, low: 3 };

/** The nearest due day first (no due day last), then the highest priority, then the title from A to Z. */
export function byPlan(a: Item, b: Item): number {
  if (a.due !== b.due) {
    if (!a.due) return 1;
    if (!b.due) return -1;
    return a.due < b.due ? -1 : 1;
  }
  const r = (i: Item) => RANK[i.priority ?? "normal"];
  return r(a) - r(b) || a.title.localeCompare(b.title);
}

/** All items, from the three lists and what the app knows about them. */
export function buildItems(
  tasks: api.Task[],
  github: api.GithubItem[],
  clickup: api.ClickupItem[],
  meta: api.ItemMeta[],
  est: Record<string, api.Estimate>,
  why: Record<string, string>,
): Item[] {
  const m = new Map(meta.map((x) => [x.key, x]));
  const extra = (key: string) => {
    const e = est[key];
    return {
      minutes: e?.minutes ?? DEFAULT_MINUTES,
      minutesBy: e?.by ?? null,
      why: why[key],
      summary: m.get(key)?.summary,
      isNew: m.get(key)?.new ?? false,
    };
  };
  const out: Item[] = [];
  for (const t of tasks) {
    const key = taskKey(t.id);
    out.push({
      key,
      src: "me",
      title: t.title,
      ref: "",
      due: t.due,
      start: t.start,
      priority: t.priority,
      tags: t.tags,
      done: t.done,
      task: t,
      ...extra(key),
      isNew: false,
    });
  }
  for (const g of github) {
    const key = `github:${g.id}`;
    out.push({
      key,
      src: "gh",
      title: g.title,
      ref: `${g.repo.split("/")[1] ?? g.repo} #${g.number}`,
      url: g.url,
      due: null,
      start: null,
      priority: null,
      tags: [],
      done: false,
      gh: g,
      ...extra(key),
    });
  }
  for (const c of clickup) {
    const key = `clickup:${c.id}`;
    out.push({
      key,
      src: "cu",
      title: c.title,
      ref: `${c.custom_id ? `${c.custom_id} · ` : ""}${c.workspace} · ${c.list}`,
      url: c.url,
      due: c.due ? dayOf(new Date(c.due)) : null,
      start: null,
      priority: c.priority ?? null,
      tags: [],
      done: false,
      cu: c,
      ...extra(key),
    });
  }
  return out;
}

export type Groups = { now: Item[]; today: Item[]; isNew: Item[]; soon: Item[]; later: Item[]; done: Item[] };

/** The groups of the home list. */
export function groupItems(items: Item[], today: string): Groups {
  const open = items.filter((i) => !i.done).sort(byPlan);
  const later = open.filter((i) => i.start && i.start > today);
  const active = open.filter((i) => !later.includes(i));
  const now = active.filter((i) => i.priority === "urgent" || (i.due !== null && i.due < today));
  const isNew = active.filter((i) => i.isNew && !now.includes(i));
  const todayList = active.filter((i) => i.due === today && !now.includes(i) && !isNew.includes(i));
  const soon = active.filter((i) => !now.includes(i) && !isNew.includes(i) && !todayList.includes(i));
  const done = items.filter((i) => i.done).sort((a, b) => (b.task?.done_at ?? 0) - (a.task?.done_at ?? 0));
  return { now, today: todayList, isNew, soon, later, done };
}

// ---------- Quick add: one line gives a task with a day, a priority, and tags ----------

export type Parsed =
  | { ask: true; text: string; plan: boolean }
  | { ask: false; title: string; due: string | null; priority: api.Priority | null; tags: string[] };

// Words that start a question or a request to AIfred. Task verbs like
// "review" or "find" are not here: "Review the contract" is a task.
// "do", "is", "will" and the like are a question only before a word like
// "I", "we", or "there": "Do I have meetings" asks, "Do the taxes" is a task.
const QUESTION = new RegExp(
  "^(" +
    [
      "what|how|why|when|which|who|where",
      "(do|does|did) (i|you|we|they|it|there|anything|aifred)",
      "(is|are|was|were|will|can|could|should|would) (i|you|we|they|it|there|this|that|my|our|the|anything|aifred)",
      "brief me|summari[sz]e|tell me|explain|help me",
    ].join("|") +
    ")\\b",
  "i",
);

// A request to plan the day is the whole line: "Plan the day trip to Bali" is a task.
const PLAN = /^plan (my |the )?(day|today|week)( for me)?( please)?[\s.!]*$/i;

/** The day of the next weekday `wd` (0 is Sunday) after today. */
function nextWeekday(today: string, wd: number): string {
  const d = dateOf(today).getDay();
  return addDays(today, ((wd - d + 7) % 7) || 7);
}

/** Reads one line. A question goes to AIfred. Other text is a new task:
 * "pay rent friday high #home" is "Pay rent", due on Friday, high priority, tag "home". */
export function parseQuick(text: string, today: string): Parsed | null {
  const raw = text.trim();
  if (!raw) return null;
  if (PLAN.test(raw)) return { ask: true, text: raw, plan: !/\bweek\b/i.test(raw) };
  if (/\?\s*$/.test(raw) || QUESTION.test(raw)) return { ask: true, text: raw, plan: false };
  const tags: string[] = [];
  let title = raw.replace(/(^|\s)#([\p{L}\p{N}_-]+)/gu, (_, sp: string, g: string) => {
    tags.push(g);
    return sp;
  });
  let due: string | null = null;
  const days: [RegExp, () => string][] = [
    [/\b(today|tonight)\b/i, () => today],
    [/\btomorrow\b/i, () => addDays(today, 1)],
    [/\bnext week\b/i, () => nextWeekday(today, 1)],
    ...LONG_WEEKDAYS.map((w, i): [RegExp, () => string] => [new RegExp(`\\b${w}\\b`, "i"), () => nextWeekday(today, i)]),
    // A short name like "sun" or "sat" is also a word, so it counts only
    // after "by", "on", or "due", or at the end: "Buy sun cream" keeps "sun".
    ...LONG_WEEKDAYS.map((w, i): [RegExp, () => string] => [
      new RegExp(`(?:(?<=\\b(?:by|on|due) )${w.slice(0, 3)}\\b|\\b${w.slice(0, 3)}(?=[\\s.!]*$))`, "i"),
      () => nextWeekday(today, i),
    ]),
    [/\b(\d{4}-\d{2}-\d{2})\b/, () => title.match(/\b(\d{4}-\d{2}-\d{2})\b/)![1]],
  ];
  for (const [re, day] of days) {
    // The day word must not run into "'s": "Review today's PR" has no due day.
    const m = title.match(new RegExp(`\\b(?:by |on |due )?(?:${re.source})(?![\\w'’])`, "i"));
    if (m) {
      due = day();
      title = title.replace(m[0], " ");
      break;
    }
  }
  let priority: api.Priority | null = null;
  // Only a priority word at the end counts: "Review high-level design" and
  // "Fix low battery warning" keep their words.
  const pm = title.match(/(?:^|,|\s)\s*(urgent|high priority|high|low priority|low)[,\s]*$/i);
  if (pm) {
    const w = pm[1].toLowerCase();
    priority = w.startsWith("urgent") ? "urgent" : w.startsWith("high") ? "high" : "low";
    title = title.replace(pm[0], " ");
  }
  title = title.replace(/\s{2,}/g, " ").replace(/[,\s]+$/, "").trim();
  title = title.charAt(0).toUpperCase() + title.slice(1);
  return { ask: false, title: title || raw, due, priority, tags };
}

// ---------- The day plan on the work day ----------

/** Minutes after midnight for "HH:MM". */
export function minutesOf(hhmm: string): number {
  const [h, m] = hhmm.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
}

/** "9:00" for 540. */
export function fmtTime(m: number): string {
  const h = Math.floor(m / 60);
  return `${h}:${String(Math.round(m % 60)).padStart(2, "0")}`;
}

/** "1 h 30 m", "45 m". */
export function dur(m: number): string {
  if (m < 60) return `${m} m`;
  return `${Math.floor(m / 60)} h${m % 60 ? ` ${m % 60} m` : ""}`;
}

export type Block = { key: string; start: number; end: number; lunch?: false } | { key: "lunch"; start: number; end: number; lunch: true };

export type Schedule = {
  /** All blocks in time order, lunch included. */
  all: Block[];
  /** The start of each planned item, by key. */
  at: Record<string, number>;
  /** Minutes of planned work. */
  work: number;
  /** When the last block ends, or null for an empty plan. */
  end: number | null;
  /** Free minutes left in the work day. Negative: too much. */
  free: number;
};

/** Places the items in order from the start of the work day. An item that
 * would overlap lunch starts after lunch. */
export function schedule(order: Item[], s: api.Settings): Schedule {
  const dayStart = minutesOf(s.work_start);
  const dayEnd = minutesOf(s.work_end);
  const lunch = s.lunch_minutes > 0 ? { start: minutesOf(s.lunch_start), end: minutesOf(s.lunch_start) + s.lunch_minutes } : null;
  const blocks: Block[] = [];
  const at: Record<string, number> = {};
  let cur = dayStart;
  for (const i of order) {
    if (lunch && lunch.start < cur + i.minutes && lunch.end > cur) cur = Math.max(cur, lunch.end);
    blocks.push({ key: i.key, start: cur, end: cur + i.minutes });
    at[i.key] = cur;
    cur += i.minutes;
  }
  const all: Block[] = [...blocks];
  if (lunch && blocks.length) all.push({ key: "lunch", start: lunch.start, end: lunch.end, lunch: true });
  all.sort((a, b) => a.start - b.start);
  const work = order.reduce((n, i) => n + i.minutes, 0);
  const end = blocks.length ? Math.max(...all.map((b) => b.end)) : null;
  const free = dayEnd - dayStart - (lunch ? s.lunch_minutes : 0) - work;
  return { all, at, work, end, free };
}

/** Bytes as "12 KB". */
export function kb(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}
