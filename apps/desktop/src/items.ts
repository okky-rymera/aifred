// One list of work items from all sources: your tasks, GitHub, ClickUp, and
// the emails that wait for your answer. The item key is the same as in the
// Rust app (items.rs): "task:<id>", "github:<id>", "clickup:<id>", or
// "gmail:<thread id>". The plan, the time guesses, the brief,
// and the item chats use it.
import type * as api from "./api";

export type Src = "me" | "gh" | "cu" | "gm";
export const SRC_NAME: Record<Src, string> = { me: "Mine", gh: "GitHub", cu: "ClickUp", gm: "Email" };

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
  /** The item that `why` is about, when it is not this item: a tree row can
   * show the reason of one of its subtasks (joinLinked). A correction goes to it. */
  whyKey?: string;
  /** The goal that it serves, from the brief. */
  goal?: string;
  /** The next action, from the brief. */
  next?: string;
  /** What AIfred can draft for the next action later. None: you do it. */
  draft?: api.DraftKind;
  /** The summary of a long description, by AIfred. */
  summary?: string;
  task?: api.Task;
  gh?: api.GithubItem;
  cu?: api.ClickupItem;
  mail?: api.Mail;
  /** The other open items of the same tree of subtasks, shown under this row (joinLinked). */
  linked?: Item[];
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

/** The Calendar page shows today and the next days: as many days as AIfred reads from Google (calendar::DAYS). */
export const WEEK = 7;

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

/** "today", or "Tue 10 Nov". Use it for each text that names a day of a plan. */
export function dayName(day: string, today: string): string {
  return day === today ? "today" : shortDay(day, today);
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

/** "Mon 2 Nov" of a time in Unix seconds. */
export const dayOfSecs = (t: number, today: string) => shortDay(dayOf(new Date(t * 1000)), today);

/** All items, from the four lists and what the app knows about them. */
export function buildItems(
  tasks: api.Task[],
  github: api.GithubItem[],
  clickup: api.ClickupItem[],
  meta: api.ItemMeta[],
  est: Record<string, api.Estimate>,
  why: Record<string, string>,
  goal: Record<string, string> = {},
  mail: api.Mail[] = [],
  next: Record<string, string> = {},
  draft: Record<string, api.DraftKind> = {},
): Item[] {
  const m = new Map(meta.map((x) => [x.key, x]));
  const extra = (key: string) => {
    const e = est[key];
    return {
      minutes: e?.minutes ?? DEFAULT_MINUTES,
      minutesBy: e?.by ?? null,
      why: why[key],
      goal: goal[key],
      next: next[key],
      draft: draft[key],
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
  for (const m of mail) {
    const key = `gmail:${m.id}`;
    const who = m.people.join(", ") + (m.more_people ? ` +${m.more_people}` : "");
    out.push({
      key,
      src: "gm",
      title: m.subject,
      ref: m.direct ? `from ${who}` : `from ${who} · you are in Cc`,
      url: m.url,
      due: null,
      start: null,
      priority: null,
      tags: [],
      done: false,
      mail: m,
      ...extra(key),
    });
  }
  return out;
}

// ---------- Subtasks (links.rs) ----------

/** The most items in one tree, like links::MAX_GROUP. */
export const MAX_GROUP = 10;

const byText = (a: string, b: string) => (a < b ? -1 : a > b ? 1 : 0);

/** The parent of `key`, or undefined for a top item. */
export const parentOf = (links: api.ItemLink[], key: string) => links.find((l) => l.key === key)?.main;

/** The top item of `key`'s tree. The walk stops after MAX_GROUP levels, like links::head_in. */
export function rootOf(links: api.ItemLink[], key: string): string {
  let at = key;
  for (let n = 0; n < MAX_GROUP; n++) {
    const p = parentOf(links, at);
    if (!p) break;
    at = p;
  }
  return at;
}

/** How many levels `key` is under its top item: 0 for a top item. */
export const depthOf = (links: api.ItemLink[], key: string) => {
  let n = 0;
  for (let at = parentOf(links, key); at && n < MAX_GROUP; at = parentOf(links, at)) n++;
  return n;
};

/** The trees, by top item. Each tree has the top item first, then each item
 * before its subtasks, the subtasks of one item in key order, like
 * links::members_in. This is the one place that reads trees from the links:
 * the home list, the item panel, the link actions, and the test fake all use it. */
export function linkGroups(links: api.ItemLink[]): Map<string, string[]> {
  const kids = new Map<string, string[]>();
  for (const l of [...links].sort((a, b) => byText(a.key, b.key))) kids.set(l.main, [...(kids.get(l.main) ?? []), l.key]);
  const under = new Set(links.map((l) => l.key));
  const out = new Map<string, string[]>();
  for (const top of kids.keys()) {
    if (under.has(top)) continue;
    const tree: string[] = [];
    const todo = [top];
    while (todo.length && tree.length <= MAX_GROUP * 2) {
      const k = todo.pop()!;
      if (tree.includes(k)) break;
      tree.push(k);
      todo.push(...[...(kids.get(k) ?? [])].reverse());
    }
    out.set(top, tree);
  }
  return out;
}

/** The keys of the tree of `key`, the top item first. Only `key` when it has no links. */
export function groupOf(links: api.ItemLink[], key: string): string[] {
  return linkGroups(links).get(rootOf(links, key)) ?? [key];
}

/** The item of a group that shows as the row in the home list: the main
 * item when it is open, else the first open item. */
export function rowKey(group: string[], byKey: Map<string, Item>): string {
  return group.find((k) => byKey.get(k)?.done === false) ?? group[0];
}

/** The rows of the home list: each tree of subtasks is one row.
 * The row is rowKey() of the tree. The other open items go in `linked`.
 * The row has the earliest due day and the highest priority of its open
 * items, so late work in a linked item is not hidden. For the same reason,
 * the row starts later only when all its open items start later. Done
 * items stay their own rows. */
export function joinLinked(items: Item[], links: api.ItemLink[]): Item[] {
  if (!links.length) return items;
  const byKey = new Map(items.map((i) => [i.key, i]));
  const rows = new Map<string, Item>();
  const under = new Set<string>();
  for (const keys of linkGroups(links).values()) {
    const open = keys.map((k) => byKey.get(k)).filter((i): i is Item => !!i && !i.done);
    if (open.length < 2) continue;
    const [head, ...rest] = open;
    const dues = open.map((i) => i.due).filter((d): d is string => !!d).sort();
    const starts = open.map((i) => i.start);
    const pri = open.map((i) => i.priority).filter((p): p is api.Priority => !!p).sort((a, b) => RANK[a] - RANK[b]);
    const withWhy = open.find((i) => i.why);
    // The reason and the next action come from the same item.
    const ranked = head.why ? head : (withWhy ?? head);
    rows.set(head.key, {
      ...head,
      due: dues[0] ?? null,
      // An open item with no start day is work for now.
      start: starts.some((d) => !d) ? null : (starts as string[]).sort()[0],
      priority: pri[0] ?? null,
      why: ranked.why,
      whyKey: ranked.key === head.key ? undefined : ranked.key,
      goal: head.goal ?? withWhy?.goal,
      next: ranked.next,
      draft: ranked.draft,
      isNew: open.some((i) => i.isNew),
      linked: rest,
    });
    for (const i of rest) under.add(i.key);
  }
  return items.filter((i) => !under.has(i.key)).map((i) => rows.get(i.key) ?? i);
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

// ---------- The home box ----------

// A request to plan the day is the whole line: "Plan the day trip to Bali" is a task.
const PLAN = /^plan (my |the )?(day|today)( for me)?( please)?[\s.!]*$/i;

/** True when the line asks to plan today: the home box opens the Day plan.
 * All other lines go to AIfred, which answers or adds a task. */
export function isPlanRequest(text: string): boolean {
  return PLAN.test(text.trim());
}

/** A new task from one line, with no AI call (the "Add task" mode of the home box). */
export type QuickTask = { title: string; due: string | null; priority: api.Priority | null; tags: string[] };

/** The day of the next weekday `wd` (0 is Sunday) after today. */
function nextWeekday(today: string, wd: number): string {
  const d = dateOf(today).getDay();
  return addDays(today, ((wd - d + 7) % 7) || 7);
}

/** Reads one line as a new task: "pay rent friday high #home" is "Pay rent",
 * due on Friday, high priority, tag "home". None for an empty line. */
export function parseTask(text: string, today: string): QuickTask | null {
  const raw = text.trim();
  if (!raw) return null;
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
  return { title: title || raw, due, priority, tags };
}

// ---------- The day plan on the work day ----------

/** Minutes after midnight for "HH:MM". */
export function minutesOf(hhmm: string): number {
  const [h, m] = hhmm.split(":").map(Number);
  return (h || 0) * 60 + (m || 0);
}

/** Minutes after midnight for a strict "HH:MM", or null when it is not a
 * time of the day. Same as minutes_of() in crates/aifred-core/src/settings.rs. */
export function timeOf(hhmm: string): number | null {
  const m = /^(\d{2}):(\d{2})$/.exec(hhmm);
  if (!m) return null;
  const [h, min] = [Number(m[1]), Number(m[2])];
  return h < 24 && min < 60 ? h * 60 + min : null;
}

/** The start and the end of the work day, in minutes after midnight. A
 * setting that AIfred cannot read gives 09:00 to 17:30, as work_day() in
 * crates/aifred-core/src/calendar.rs. Use it for each count of the work day. */
export function workDay(s: api.Settings): [number, number] {
  return [timeOf(s.work_start) ?? 540, timeOf(s.work_end) ?? 1050];
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

/** A planned item, lunch, or a meeting from the calendar. Item blocks have neither `lunch` nor `meeting`. */
export type Block =
  | { key: string; start: number; end: number; lunch?: false; meeting?: undefined; pinned?: boolean }
  | { key: "lunch"; start: number; end: number; lunch: true; meeting?: undefined }
  | { key: string; start: number; end: number; lunch?: false; meeting: api.CalEvent };

export const isItemBlock = (b: Block) => !b.lunch && !b.meeting;

/** A busy event of one day, in minutes after midnight. */
export type Meeting = { start: number; end: number; event: api.CalEvent };

/** The events of one day: `busy` for the schedule, `timed` (all timed
 * events, also the free and the declined ones) and `allDay` for the calendar. */
export type DayEvents = { busy: Meeting[]; timed: Meeting[]; allDay: api.CalEvent[] };

/** The events of `day` ("YYYY-MM-DD"), cut to that day, in one pass. The
 * minutes are clock times: a day when the clocks change has 23 or 25 hours,
 * so seconds since midnight would put a meeting an hour off. */
export function eventsOn(events: api.CalEvent[], day: string): DayEvents {
  const [y, m, d] = day.split("-").map(Number);
  const t0 = new Date(y, m - 1, d).getTime() / 1000;
  const t1 = new Date(y, m - 1, d + 1).getTime() / 1000;
  const clock = (t: number) => {
    const x = new Date(t * 1000);
    return x.getHours() * 60 + x.getMinutes();
  };
  const out: DayEvents = { busy: [], timed: [], allDay: [] };
  for (const e of events) {
    if (!(e.start < t1 && e.end > t0)) continue;
    // An all-day event is never busy time, also when its busy flag is set,
    // as in calendar.rs: it would fill the full day and push all items past midnight.
    if (e.all_day) {
      out.allDay.push(e);
      continue;
    }
    const start = e.start <= t0 ? 0 : clock(e.start);
    const end = e.end >= t1 ? 1440 : clock(e.end);
    const mt = { start, end: Math.max(start, end), event: e };
    if (e.busy) out.busy.push(mt);
    out.timed.push(mt);
  }
  return out;
}

/** Midnight at the end of the day, in minutes after midnight. */
const DAY_END = 1440;

export type Schedule = {
  /** All blocks in time order, lunch included. */
  all: Block[];
  /** The start of each planned item, by key. */
  at: Record<string, number>;
  /** Minutes of planned work. */
  work: number;
  /** When the last planned item ends, or null for an empty plan. Lunch
   * and meetings after the work do not move it. */
  end: number | null;
  /** Free minutes left in the work day. Negative: too much. */
  free: number;
};

/** Places the items in order from the start of the work day. An item that
 * would overlap lunch, a meeting, or an item with a fixed time starts
 * after it. `meetings`: the busy events of the day (eventsOn().busy). Only the
 * ones in the work day count. `pins`: the fixed start times, in minutes
 * after midnight, by key. An item with a fixed time starts at that time. */
export function schedule(order: Item[], s: api.Settings, meetings: Meeting[] = [], pins: Record<string, number> = {}): Schedule {
  const [dayStart, dayEnd] = workDay(s);
  // A lunch time that AIfred cannot read gives no lunch, as in the backend.
  const lunchAt = timeOf(s.lunch_start);
  const lunch = s.lunch_minutes > 0 && lunchAt !== null ? { start: lunchAt, end: lunchAt + s.lunch_minutes } : null;
  const inDay = meetings.filter((m) => m.start < dayEnd && m.end > dayStart);
  const pinned = order.filter((i) => pins[i.key] !== undefined).map((i) => ({ start: pins[i.key], end: pins[i.key] + i.minutes }));
  // Lunch and the meetings: time that is not free for work.
  const busy = [...(lunch ? [lunch] : []), ...inDay];
  const fixed = [...busy, ...pinned];
  const blocks: Block[] = [];
  const at: Record<string, number> = {};
  let cur = dayStart;
  for (const i of order) {
    const pin = pins[i.key];
    if (pin !== undefined) {
      // A block ends at midnight at the latest: the day column ends there.
      blocks.push({ key: i.key, start: pin, end: Math.min(DAY_END, pin + i.minutes), pinned: true });
      at[i.key] = pin;
      continue;
    }
    // Jump over each fixed block that the item would overlap, until it fits.
    for (let hit = fixed.find((f) => f.start < cur + i.minutes && f.end > cur); hit; hit = fixed.find((f) => f.start < cur + i.minutes && f.end > cur)) {
      cur = Math.max(cur, hit.end);
    }
    blocks.push({ key: i.key, start: Math.min(DAY_END, cur), end: Math.min(DAY_END, cur + i.minutes) });
    at[i.key] = cur;
    cur += i.minutes;
  }
  const all: Block[] = [...blocks];
  if (blocks.length) {
    if (lunch) all.push({ key: "lunch", start: lunch.start, end: lunch.end, lunch: true });
    // A meeting that started before the work day (for example an overnight
    // flight) shows from the work-day start, not from midnight.
    for (const m of inDay) all.push({ key: `event:${m.event.id}`, start: Math.max(m.start, dayStart), end: m.end, meeting: m.event });
  }
  all.sort((a, b) => a.start - b.start);
  const work = order.reduce((n, i) => n + i.minutes, 0);
  const end = blocks.length ? Math.max(...blocks.map((b) => b.end)) : null;
  // The parts of the work day that lunch and the meetings take, in time
  // order. Two at the same time count once.
  // calendar::free_minutes in crates/aifred-core/src/calendar.rs gives the AI
  // the same number. A test in items.test.ts checks the same cases as its test.
  const takenSpans: { start: number; end: number }[] = [];
  for (const f of busy.map((f) => ({ start: Math.max(f.start, dayStart), end: Math.min(f.end, dayEnd) })).sort((a, b) => a.start - b.start)) {
    const last = takenSpans[takenSpans.length - 1];
    if (f.end <= f.start) continue;
    if (last && f.start <= last.end) last.end = Math.max(last.end, f.end);
    else takenSpans.push({ ...f });
  }
  const taken = takenSpans.reduce((n, t) => n + t.end - t.start, 0);
  // An item with a fixed time is work only in the part of the work day that
  // lunch and the meetings do not take: that time is in `taken` already. So
  // an item at 19:00 uses no work time. calendar::fixed_work in
  // crates/aifred-core/src/calendar.rs counts the same way for the AI.
  const inDayWork = order.reduce((n, i) => {
    const pin = pins[i.key];
    if (pin === undefined) return n + i.minutes;
    const a = Math.max(pin, dayStart);
    const b = Math.min(pin + i.minutes, dayEnd);
    if (b <= a) return n;
    const overlap = takenSpans.reduce((o, t) => o + Math.max(0, Math.min(t.end, b) - Math.max(t.start, a)), 0);
    return n + b - a - overlap;
  }, 0);
  const free = dayEnd - dayStart - taken - inDayWork;
  return { all, at, work, end, free };
}

/** The side-by-side place of each block of a day, as in Google Calendar:
 * blocks that overlap share the width. `col` is the column of the block,
 * `cols` the number of columns in its group of overlapping blocks. */
export function sideBySide(blocks: { start: number; end: number }[]): { col: number; cols: number }[] {
  const order = blocks.map((b, n) => ({ ...b, n, end: Math.max(b.end, b.start + 1) })).sort((a, b) => a.start - b.start || b.end - a.end);
  const out = blocks.map(() => ({ col: 0, cols: 1 }));
  let group: typeof order = [];
  let colEnds: number[] = [];
  let groupEnd = -1;
  const close = () => {
    for (const b of group) out[b.n].cols = colEnds.length;
    group = [];
    colEnds = [];
  };
  for (const b of order) {
    if (b.start >= groupEnd) close();
    let col = colEnds.findIndex((e) => e <= b.start);
    if (col < 0) {
      col = colEnds.length;
      colEnds.push(b.end);
    } else colEnds[col] = b.end;
    out[b.n].col = col;
    group.push(b);
    groupEnd = Math.max(groupEnd, b.end);
  }
  close();
  return out;
}

/** Bytes as "12 KB". */
export function kb(n: number): string {
  if (n < 1024) return `${n} B`;
  if (n < 1048576) return `${Math.round(n / 1024)} KB`;
  return `${(n / 1048576).toFixed(1)} MB`;
}
