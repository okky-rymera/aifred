// The item logic with no window: day labels, groups, the quick add, and the schedule.
import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS } from "./api";
import type * as api from "./api";
import { addDays, buildItems, dueInfo, dur, fmtTime, groupItems, isPlanRequest, eventsOn, joinLinked, parseTask, schedule, shortDay, sideBySide, treeDepths, workDay, type Item } from "./items";
import { clickupItem, githubItem, task } from "./test/fakeBackend";

// Wednesday 4 November 2026.
const TODAY = "2026-11-04";

describe("days", () => {
  it("names the due day in plain words", () => {
    expect(dueInfo(TODAY, TODAY)).toEqual({ text: "Today", cls: "today" });
    expect(dueInfo("2026-11-05", TODAY).text).toBe("Tomorrow");
    expect(dueInfo("2026-11-03", TODAY)).toEqual({ text: "Due yesterday", cls: "late" });
    expect(dueInfo("2026-11-01", TODAY)).toEqual({ text: "3 days late", cls: "late" });
    expect(dueInfo("2026-11-06", TODAY).text).toBe("Fri 6 Nov");
    expect(shortDay("2027-01-02", TODAY)).toBe("Sat 2 Jan 2027");
    expect(addDays("2026-12-31", 1)).toBe("2027-01-01");
  });
});

describe("isPlanRequest", () => {
  it("is only a whole line that asks to plan today", () => {
    expect(isPlanRequest("Plan my day")).toBe(true);
    expect(isPlanRequest("plan the day please.")).toBe(true);
    expect(isPlanRequest("  plan today ")).toBe(true);
    expect(isPlanRequest("Plan the day trip to Bali")).toBe(false);
    expect(isPlanRequest("Plan my week")).toBe(false);
    expect(isPlanRequest("what is due today?")).toBe(false);
  });
});

describe("parseTask", () => {
  it("reads the day, the priority, and the tags of a new task", () => {
    expect(parseTask("pay rent friday high #home", TODAY)).toEqual({ title: "Pay rent", due: "2026-11-06", priority: "high", tags: ["home"] });
    expect(parseTask("call Sam tomorrow", TODAY)).toMatchObject({ title: "Call Sam", due: "2026-11-05" });
    expect(parseTask("send report next week, urgent", TODAY)).toMatchObject({ title: "Send report", due: "2026-11-09", priority: "urgent" });
    expect(parseTask("renew passport due 2027-02-01", TODAY)).toMatchObject({ title: "Renew passport", due: "2027-02-01" });
    // The same weekday as today is next week, not today.
    expect(parseTask("water plants wednesday", TODAY)).toMatchObject({ due: "2026-11-11" });
  });

  it("keeps the words of a task, and reads nothing from an empty line", () => {
    expect(parseTask("Review the contract", TODAY)).toMatchObject({ title: "Review the contract" });
    expect(parseTask("Find a plumber", TODAY)).toMatchObject({ title: "Find a plumber", due: null });
    expect(parseTask("   ", TODAY)).toBeNull();
    expect(parseTask("Do the taxes friday", TODAY)).toMatchObject({ title: "Do the taxes", due: "2026-11-06" });
    expect(parseTask("Plan the team offsite #work", TODAY)).toMatchObject({ title: "Plan the team offsite", tags: ["work"] });
    expect(parseTask("Plan the day trip to Bali", TODAY)).toMatchObject({ title: "Plan the day trip to Bali", due: null });
  });

  it("does not read a day inside another word", () => {
    expect(parseTask("Review today's PR", TODAY)).toMatchObject({ title: "Review today's PR", due: null });
    expect(parseTask("Prepare tomorrow’s slides", TODAY)).toMatchObject({ title: "Prepare tomorrow’s slides", due: null });
    expect(parseTask("Buy sun cream", TODAY)).toMatchObject({ title: "Buy sun cream", due: null });
    expect(parseTask("Sat exam prep", TODAY)).toMatchObject({ due: null });
    expect(parseTask("water plants on wed", TODAY)).toMatchObject({ title: "Water plants", due: "2026-11-11" });
    expect(parseTask("call mum fri", TODAY)).toMatchObject({ title: "Call mum", due: "2026-11-06" });
  });

  it("reads a priority word only at the end", () => {
    expect(parseTask("Review high-level design", TODAY)).toMatchObject({ title: "Review high-level design", priority: null });
    expect(parseTask("Fix low battery warning", TODAY)).toMatchObject({ title: "Fix low battery warning", priority: null });
    expect(parseTask("Fix the login, high priority", TODAY)).toMatchObject({ title: "Fix the login", priority: "high" });
  });
});

describe("items and groups", () => {
  const items = (): Item[] =>
    buildItems(
      [
        task("Late bill", { due: "2026-11-01" }),
        task("Pay rent", { due: TODAY }),
        task("Book flights", { start: "2026-11-20" }),
        task("Done one", { done: true, done_at: 5 }),
      ],
      [githubItem(7, "review_requested", "Fix cart"), githubItem(9, "assigned", "New bug")],
      [clickupItem("86a", "Write notes", { priority: "urgent" })],
      [{ key: "github:acme/shop#9", first_seen: 1, new: true }],
      { "github:acme/shop#7": { minutes: 45, by: "aifred" } },
      { "github:acme/shop#7": "It waits for 3 days." },
    );

  it("gives each source its key, place, and time guess", () => {
    const all = items();
    const gh = all.find((i) => i.key === "github:acme/shop#7")!;
    expect([gh.ref, gh.minutes, gh.minutesBy, gh.why]).toEqual(["shop #7", 45, "aifred", "It waits for 3 days."]);
    const cu = all.find((i) => i.key === "clickup:86a")!;
    expect(cu.ref).toBe("Rymera · Sprint 12");
    expect(all.find((i) => i.title === "Pay rent")!.minutes).toBe(30);
  });

  it("puts late and urgent items first, then today, new, open, later, and finished", () => {
    const g = groupItems(items(), TODAY);
    const t = (l: Item[]) => l.map((i) => i.title);
    expect(t(g.now)).toEqual(["Late bill", "Write notes"]);
    expect(t(g.today)).toEqual(["Pay rent"]);
    expect(t(g.isNew)).toEqual(["New bug"]);
    expect(t(g.soon)).toEqual(["Fix cart"]);
    expect(t(g.later)).toEqual(["Book flights"]);
    expect(t(g.done)).toEqual(["Done one"]);
  });
});

describe("schedule", () => {
  const item = (key: string, minutes: number) => ({ key, minutes }) as Item;

  it("places items in order from the start of the day, and moves an item that would overlap lunch", () => {
    const s = schedule([item("a", 120), item("b", 90), item("c", 60)], DEFAULT_SETTINGS);
    expect(s.at).toEqual({ a: 540, b: 660, c: 795 });
    expect(s.all.map((b) => b.key)).toEqual(["a", "b", "lunch", "c"]);
    expect(fmtTime(s.end!)).toBe("14:15");
    expect(s.work).toBe(270);
    expect(s.free).toBe(465 - 270);
  });

  it("places items between meetings, and the meetings take free time once", () => {
    const ev = (id: string) => ({ id, title: id }) as api.CalEvent;
    // 10:00-11:00 and 10:30-11:30 overlap. 18:00 is after the work day.
    const meetings = [
      { start: 600, end: 660, event: ev("m1") },
      { start: 630, end: 690, event: ev("m2") },
      { start: 1080, end: 1140, event: ev("late") },
    ];
    const s = schedule([item("a", 30), item("b", 60), item("c", 30)], DEFAULT_SETTINGS, meetings);
    // a: 09:00-09:30. b does not fit before 10:00, so it starts at 11:30. c would overlap lunch, so it starts at 13:15.
    expect(s.at).toEqual({ a: 540, b: 690, c: 795 });
    expect(s.all.map((b) => b.key)).toEqual(["a", "event:m1", "event:m2", "b", "lunch", "c"]);
    expect(s.all[1].meeting?.title).toBe("m1");
    expect(s.free).toBe(465 - 90 - 120);
    expect(eventsOn([], "2026-11-05").busy).toEqual([]);
    // Clock times, and cut at midnight.
    const at = (h: number, m: number) => new Date(2026, 10, 5, h, m).getTime() / 1000;
    const late = { ...ev("late"), busy: true, start: at(23, 0), end: at(23, 0) + 7200 };
    const on = eventsOn([{ ...ev("m1"), busy: true, start: at(9, 30), end: at(10, 15) }, late], "2026-11-05").busy;
    expect(on.map((m) => [m.start, m.end])).toEqual([[570, 615], [1380, 1440]]);
  });

  it("does not count an all-day event as busy time", () => {
    // An all-day "Out of office" with busy set would fill 0-1440 and push all items past midnight.
    const day = (d: number) => new Date(2026, 10, d).getTime() / 1000;
    const off = { id: "ooo", title: "Out of office", busy: true, all_day: true, start: day(5), end: day(6) } as api.CalEvent;
    const on = eventsOn([off], "2026-11-05");
    expect(on.busy).toEqual([]);
    expect(on.timed).toEqual([]);
    expect(on.allDay.map((e) => e.id)).toEqual(["ooo"]);
    expect(schedule([item("a", 30)], DEFAULT_SETTINGS, on.busy).at).toEqual({ a: 540 });
  });

  it("ends with the last item, not with a meeting after it", () => {
    const ev = (id: string) => ({ id, title: id }) as api.CalEvent;
    const s = schedule([item("a", 30)], DEFAULT_SETTINGS, [{ start: 960, end: 1020, event: ev("m") }]);
    expect(fmtTime(s.end!)).toBe("9:30");
    expect(s.all.map((b) => b.key)).toEqual(["a", "lunch", "event:m"]);
  });

  it("shows a meeting from an earlier day from the work-day start, not from midnight", () => {
    const ev = (id: string) => ({ id, title: id }) as api.CalEvent;
    // An overnight event to 09:30: eventsOn gives it the start 0.
    const s = schedule([item("a", 30)], DEFAULT_SETTINGS, [{ start: 0, end: 570, event: ev("flight") }]);
    expect(s.all[0]).toMatchObject({ key: "event:flight", start: 540, end: 570 });
    expect(s.at).toEqual({ a: 570 });
  });

  // The same cases as free_minutes_take_out_lunch_and_meetings_once in
  // crates/aifred-core/src/calendar.rs. The AI plans with the Rust number,
  // and the Plan page shows this one: they must stay the same.
  it("counts free time like the backend", () => {
    const ev = (id: string) => ({ id, title: id }) as api.CalEvent;
    const m = (start: number, end: number) => ({ start, end, event: ev(`${start}`) });
    expect(schedule([], DEFAULT_SETTINGS).free).toBe(465);
    expect(schedule([], DEFAULT_SETTINGS, [m(600, 660), m(630, 690), m(720, 780), m(1080, 1140)]).free).toBe(510 - 90 - 75);
    expect(schedule([], DEFAULT_SETTINGS, [m(0, 1440)]).free).toBe(0);
  });

  it("counts only the work-day part of an item with a fixed time", () => {
    // Work day 9:00–17:30 with 45 minutes of lunch: 465 free minutes.
    expect(schedule([item("a", 120)], DEFAULT_SETTINGS, [], { a: 19 * 60 }).free).toBe(465);
    expect(schedule([item("a", 60)], DEFAULT_SETTINGS, [], { a: 17 * 60 }).free).toBe(435);
    expect(schedule([item("a", 60)], DEFAULT_SETTINGS).free).toBe(405);
  });

  it("uses 09:00 to 17:30 and no lunch for settings that it cannot read, like the backend", () => {
    // calendar::work_day and taken_spans in crates/aifred-core/src/calendar.rs.
    const bad = { ...DEFAULT_SETTINGS, work_start: "x", work_end: "25:00", lunch_start: "noon" };
    expect(workDay(bad)).toEqual([540, 1050]);
    expect(schedule([], bad).free).toBe(510);
    expect(schedule([item("a", 30)], bad).at).toEqual({ a: 540 });
  });

  it("ends a block at midnight at the latest", () => {
    const s = schedule([item("a", 60)], DEFAULT_SETTINGS, [], { a: 23 * 60 + 30 });
    expect(s.all.find((b) => b.key === "a")).toMatchObject({ start: 1410, end: 1440 });
  });

  it("keeps an item with a fixed time at its time, and places the others around it", () => {
    // b is fixed at 9:30. a (60 m) does not fit before it, so a starts after b.
    const s = schedule([item("a", 60), item("b", 30), item("c", 30)], DEFAULT_SETTINGS, [], { b: 570 });
    expect(s.at).toEqual({ a: 600, b: 570, c: 660 });
    expect(s.all.find((x) => x.key === "b")).toMatchObject({ pinned: true });
    // A fixed item is work: it does not count as taken time twice.
    expect(s.free).toBe(465 - 120);
    // A fixed time on lunch stays there.
    expect(schedule([item("a", 30)], DEFAULT_SETTINGS, [], { a: 750 }).at).toEqual({ a: 750 });
    // Lunch (12:30, 45 m) takes those minutes already: an item on it uses no more.
    expect(schedule([item("a", 45)], DEFAULT_SETTINGS, [], { a: 750 }).free).toBe(465);
    // Half on a meeting (10:00-11:00): only the other 30 minutes are work.
    const meet = { start: 600, end: 660, event: { id: "m", title: "m" } as api.CalEvent };
    expect(schedule([item("a", 60)], DEFAULT_SETTINGS, [meet], { a: 630 }).free).toBe(465 - 60 - 30);
  });

  it("puts overlapping blocks side by side", () => {
    expect(sideBySide([{ start: 0, end: 60 }, { start: 30, end: 90 }, { start: 90, end: 120 }, { start: 40, end: 50 }])).toEqual([
      { col: 0, cols: 3 },
      { col: 1, cols: 3 },
      { col: 0, cols: 1 },
      { col: 2, cols: 3 },
    ]);
  });

  it("says how much is too much, and has no lunch for an empty plan", () => {
    expect(schedule([item("a", 600)], DEFAULT_SETTINGS).free).toBe(-135);
    const empty = schedule([], DEFAULT_SETTINGS);
    expect([empty.all, empty.end]).toEqual([[], null]);
    expect(schedule([item("a", 30)], { ...DEFAULT_SETTINGS, lunch_minutes: 0 }).all.map((b) => b.key)).toEqual(["a"]);
    expect(dur(90)).toBe("1 h 30 m");
    expect(dur(45)).toBe("45 m");
  });
});

describe("joinLinked", () => {
  const t = task("Refund Ana", { id: "t1" } as Partial<api.Task>);
  const items = buildItems([t], [githubItem(7, "assigned", "Fix refund")], [clickupItem("86a", "Refund", { due: Date.parse("2026-11-03T12:00:00") })], [], {}, { "github:acme/shop#7": "It blocks the release." });

  it("makes one row of the open items of a group, with the most urgent day", () => {
    const rows = joinLinked(items, [
      { key: "github:acme/shop#7", main: "task:t1" },
      { key: "clickup:86a", main: "task:t1" },
    ]);
    expect(rows.map((r) => r.key)).toEqual(["task:t1"]);
    // The other items are in key order, like links.rs.
    expect(rows[0].linked?.map((l) => l.key)).toEqual(["clickup:86a", "github:acme/shop#7"]);
    // The task has no due day: the row takes the ClickUp one, so it is late.
    expect(rows[0].due).toBe("2026-11-03");
    expect(groupItems(rows, TODAY).now.map((r) => r.key)).toEqual(["task:t1"]);
    // The reason of a linked item shows on the row.
    expect(rows[0].why).toBe("It blocks the release.");
  });

  it("shows the first open item as the row when the main item is done", () => {
    const done = items.map((i) => (i.key === "task:t1" ? { ...i, done: true } : i));
    const rows = joinLinked(done, [
      { key: "github:acme/shop#7", main: "task:t1" },
      { key: "clickup:86a", main: "task:t1" },
    ]);
    expect(rows.map((r) => r.key)).toEqual(["task:t1", "clickup:86a"]);
    expect(rows[1].linked?.map((l) => l.key)).toEqual(["github:acme/shop#7"]);
  });

  it("does not hide late linked work under Later when only the main item starts later", () => {
    const later = items.map((i) => (i.key === "task:t1" ? { ...i, start: addDays(TODAY, 5) } : i));
    const links = [
      { key: "github:acme/shop#7", main: "task:t1" },
      { key: "clickup:86a", main: "task:t1" },
    ];
    const rows = joinLinked(later, links);
    expect(rows[0].start).toBeNull();
    expect(groupItems(rows, TODAY).now.map((r) => r.key)).toEqual(["task:t1"]);
    // When all open items start later, the row starts with the earliest one.
    const all = later.map((i) => ({ ...i, start: i.key === "task:t1" ? i.start : addDays(TODAY, 3) }));
    expect(joinLinked(all, links)[0].start).toBe(addDays(TODAY, 3));
  });

  it("keeps a group with one open item as a plain row", () => {
    const rows = joinLinked(items, [{ key: "github:none#1", main: "task:t1" }]);
    expect(rows).toEqual(items);
  });
});

describe("treeDepths", () => {
  const links = [
    { key: "b", main: "a" },
    { key: "c", main: "a" },
    { key: "d", main: "b" },
  ] as api.ItemLink[];
  it("gives the level under the top item for the whole tree", () => {
    expect([...treeDepths(links, ["a", "b", "d", "c"])]).toEqual([["a", 0], ["b", 1], ["d", 2], ["c", 1]]);
  });
  it("counts only the parents that show", () => {
    // "a" is finished and does not show.
    expect([...treeDepths(links, ["b", "d", "c"])]).toEqual([["b", 0], ["d", 1], ["c", 0]]);
  });
});
