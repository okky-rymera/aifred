// The item logic with no window: day labels, groups, the quick add, and the schedule.
import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS } from "./api";
import { addDays, buildItems, dueInfo, dur, fmtTime, groupItems, parseQuick, schedule, shortDay, type Item } from "./items";
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

describe("parseQuick", () => {
  it("reads the day, the priority, and the tags of a new task", () => {
    expect(parseQuick("pay rent friday high #home", TODAY)).toEqual({ ask: false, title: "Pay rent", due: "2026-11-06", priority: "high", tags: ["home"] });
    expect(parseQuick("call Sam tomorrow", TODAY)).toMatchObject({ title: "Call Sam", due: "2026-11-05" });
    expect(parseQuick("send report next week, urgent", TODAY)).toMatchObject({ title: "Send report", due: "2026-11-09", priority: "urgent" });
    expect(parseQuick("renew passport due 2027-02-01", TODAY)).toMatchObject({ title: "Renew passport", due: "2027-02-01" });
    // The same weekday as today is next week, not today.
    expect(parseQuick("water plants wednesday", TODAY)).toMatchObject({ due: "2026-11-11" });
  });

  it("sends questions to AIfred, and keeps task verbs as tasks", () => {
    expect(parseQuick("what is due today?", TODAY)).toEqual({ ask: true, text: "what is due today?", plan: false });
    expect(parseQuick("Plan my day", TODAY)).toEqual({ ask: true, text: "Plan my day", plan: true });
    expect(parseQuick("Review the contract", TODAY)).toMatchObject({ ask: false, title: "Review the contract" });
    expect(parseQuick("Find a plumber", TODAY)).toMatchObject({ ask: false });
    expect(parseQuick("   ", TODAY)).toBeNull();
    expect(parseQuick("Do I have meetings today", TODAY)).toMatchObject({ ask: true });
    expect(parseQuick("Is the build green", TODAY)).toMatchObject({ ask: true });
    expect(parseQuick("Do the taxes friday", TODAY)).toMatchObject({ ask: false, title: "Do the taxes", due: "2026-11-06" });
    expect(parseQuick("Plan the team offsite #work", TODAY)).toMatchObject({ ask: false, title: "Plan the team offsite", tags: ["work"] });
    expect(parseQuick("Plan the day", TODAY)).toEqual({ ask: true, text: "Plan the day", plan: true });
    expect(parseQuick("plan my day please.", TODAY)).toMatchObject({ ask: true, plan: true });
    expect(parseQuick("Plan my week", TODAY)).toMatchObject({ ask: true, plan: false });
    expect(parseQuick("Plan the day trip to Bali", TODAY)).toMatchObject({ ask: false, title: "Plan the day trip to Bali", due: null });
  });

  it("does not read a day inside another word", () => {
    expect(parseQuick("Review today's PR", TODAY)).toMatchObject({ title: "Review today's PR", due: null });
    expect(parseQuick("Prepare tomorrow’s slides", TODAY)).toMatchObject({ title: "Prepare tomorrow’s slides", due: null });
    expect(parseQuick("Buy sun cream", TODAY)).toMatchObject({ title: "Buy sun cream", due: null });
    expect(parseQuick("Sat exam prep", TODAY)).toMatchObject({ due: null });
    expect(parseQuick("water plants on wed", TODAY)).toMatchObject({ title: "Water plants", due: "2026-11-11" });
    expect(parseQuick("call mum fri", TODAY)).toMatchObject({ title: "Call mum", due: "2026-11-06" });
  });

  it("reads a priority word only at the end", () => {
    expect(parseQuick("Review high-level design", TODAY)).toMatchObject({ title: "Review high-level design", priority: null });
    expect(parseQuick("Fix low battery warning", TODAY)).toMatchObject({ title: "Fix low battery warning", priority: null });
    expect(parseQuick("Fix the login, high priority", TODAY)).toMatchObject({ title: "Fix the login", priority: "high" });
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

  it("says how much is too much, and has no lunch for an empty plan", () => {
    expect(schedule([item("a", 600)], DEFAULT_SETTINGS).free).toBe(-135);
    const empty = schedule([], DEFAULT_SETTINGS);
    expect([empty.all, empty.end]).toEqual([[], null]);
    expect(schedule([item("a", 30)], { ...DEFAULT_SETTINGS, lunch_minutes: 0 }).all.map((b) => b.key)).toEqual(["a"]);
    expect(dur(90)).toBe("1 h 30 m");
    expect(dur(45)).toBe("45 m");
  });
});
