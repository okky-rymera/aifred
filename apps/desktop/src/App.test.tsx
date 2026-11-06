// The full window with a fake backend: Home (brief and list), the task
// panel, the chats, Day plan, Settings, and Memory.
import { describe, expect, it, vi } from "vitest";
import { fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { DEFAULT_SETTINGS, type Answer } from "./api";
import { addDays, dayOf } from "./items";
import { answer, clickupItem, deferred, fact, fakeBackend, githubItem, job, mail, meeting, task } from "./test/fakeBackend";

// The panel stays in the page, hidden, behind the task tab. A role query skips a hidden element.
const panel = () => document.querySelector<HTMLElement>('aside[aria-label="Conversation"]');
const details = () => screen.getByRole("dialog", { name: "Task details" });
const homeBox = () => screen.getByRole("textbox", { name: "Add a task or ask AIfred" });
/** The frame of the home box, with its buttons. */
const homeForm = () => homeBox().closest(".composer") as HTMLElement;
const today = () => dayOf(new Date());

/** Starts the window. The brief does not write itself, unless the test sets auto_brief.
 * The reviews of today are done, so their cards do not show, unless the test says else. */
async function startApp(init: Parameters<typeof fakeBackend>[0] = {}) {
  const fake = fakeBackend({
    dayReviews: { [today()]: [] },
    weekReviews: [today()],
    ...init,
    settings: { ...DEFAULT_SETTINGS, auto_brief: false, ...init.settings },
  });
  render(<App />);
  await screen.findByText("Claude (your login)"); // the first load is done
  return fake;
}

const openItem = async (title: string) => userEvent.click(await screen.findByRole("button", { name: `Open details: ${title}` }));

describe("home", () => {
  it("shows the brief, and the items of all sources in groups, with no chat panel", async () => {
    await startApp({
      clickupToken: "pk_test",
      tasks: [task("Pay rent", { due: today(), priority: "high" }), task("Old bill", { due: addDays(today(), -2) })],
      github: [githubItem(7, "review_requested", "Fix cart")],
      clickup: [clickupItem("86a", "Write notes", { custom_id: "DEV-42" })],
    });
    expect(screen.getByRole("region", { name: "Brief" })).toBeInTheDocument();
    const now = await screen.findByRole("list", { name: "Needs you now" });
    expect(within(now).getByText("Old bill")).toBeInTheDocument();
    expect(within(now).getByText("2 days late")).toBeInTheDocument();
    expect(within(screen.getByRole("list", { name: "Today" })).getByText("Pay rent")).toBeInTheDocument();
    const open = await screen.findByRole("list", { name: "Open" });
    expect(within(open).getByText("Fix cart")).toBeInTheDocument();
    expect(within(open).getByText("Review")).toBeInTheDocument();
    expect(await within(open).findByText("Write notes")).toBeInTheDocument();
    expect(within(open).getByText(/DEV-42 · Rymera · Sprint 12/)).toBeInTheDocument();
    expect(panel()).not.toBeInTheDocument();
  });

  it("puts new GitHub items under New, with the reason from the brief", async () => {
    await startApp({
      github: [githubItem(9, "review_requested", "Bulk edit")],
      newKeys: ["github:acme/shop#9"],
      briefs: { [today()]: { day: today(), written: 1, text: "Review [the bulk edit](item:github:acme/shop#9).", why: { "github:acme/shop#9": "It blocks the release." } } },
    });
    const list = await screen.findByRole("list", { name: "New" });
    expect(within(list).getByText("Bulk edit")).toBeInTheDocument();
    expect(within(list).getByText("It blocks the release.")).toBeInTheDocument();
  });

  it("writes the brief by itself once a day when the setting is on, and the item links open the item", async () => {
    const fake = await startApp({
      settings: { ...DEFAULT_SETTINGS, auto_brief: true },
      tasks: [task("Pay rent")],
      onWriteBrief: (day) => ({ day, written: 1, text: "One thing: [the rent](item:task:t-x).", why: {} }),
    });
    const key = `task:${fake.tasks[0].id}`;
    fake.onWriteBrief = (day) => ({ day, written: 1, text: `One thing: [the rent](item:${key}).`, why: {} });
    await waitFor(() => expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1));
    // The first answer named an item that is not in the list: plain text, no button.
    expect(await screen.findByText("the rent")).not.toHaveClass("ref");
    await userEvent.click(screen.getByRole("button", { name: "Write again" }));
    await userEvent.click(await screen.findByRole("button", { name: "the rent" }));
    expect(within(details()).getByRole("textbox", { name: "Task title" })).toHaveValue("Pay rent");
    expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(2);
  });

  it("does not write the brief by itself when the setting is off, and shows a failed write", async () => {
    const fake = await startApp({ onWriteBrief: () => Promise.reject("claude is not logged in") });
    expect(screen.getByText(/has not written today's brief yet/)).toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "write_brief")).toBe(false);
    await userEvent.click(screen.getByRole("button", { name: "Write the brief" }));
    expect(await screen.findByText("Cannot write the brief: claude is not logged in")).toBeInTheDocument();
  });

  it("keeps tasks with a later start day under Later, and done tasks under Finished", async () => {
    await startApp({ tasks: [task("Book flights", { start: addDays(today(), 5) }), task("Old one", { done: true, done_at: 5 })] });
    expect(await screen.findByText("Nothing to do today. Some tasks start later.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show later (1)" }));
    expect(within(screen.getByRole("list", { name: "Later" })).getByText("Book flights")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Show finished (1)" }));
    expect(within(screen.getByRole("list", { name: "Finished" })).getByText("Old one")).toBeInTheDocument();
  });

  it("a finished task has an undo", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await userEvent.click(await screen.findByRole("button", { name: "Finish: Pay rent" }));
    expect(await screen.findByText("Finished: Pay rent")).toBeInTheDocument();
    expect(fake.tasks[0].done).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].done).toBe(false));
  });

  it("Tomorrow moves a task to tomorrow, with an undo", async () => {
    const fake = await startApp({ tasks: [task("Pay rent", { due: today() })] });
    await userEvent.click(await screen.findByRole("button", { name: "Tomorrow" }));
    await waitFor(() => expect(fake.tasks[0].due).toBe(addDays(today(), 1)));
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].due).toBe(today()));
  });
});

describe("reviews", () => {
  /** "HH:MM" `m` minutes from now, at most 23:59. */
  const hhmm = (m: number) => {
    const d = new Date(Date.now() + m * 60_000);
    if (d.getDate() !== new Date().getDate()) return "23:59";
    return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
  };

  it("the end-of-day check shows before the work day ends, and moves the ticked items to tomorrow", async () => {
    const now = Math.floor(Date.now() / 1000);
    const a = task("Send invoice", { done: true, done_at: now });
    const b = task("Pay rent", { due: today() });
    const c = task("Write notes");
    const fake = await startApp({
      tasks: [a, b, c],
      dayReviews: {},
      plans: { [today()]: { day: today(), mode: "self", items: [{ key: `task:${c.id}` }], updated: 1 } },
      settings: { ...DEFAULT_SETTINGS, auto_brief: false, work_start: "00:00", work_end: hhmm(10) },
    });
    const card = await screen.findByRole("region", { name: "End of the day" });
    expect(within(card).getByText("Send invoice")).toBeInTheDocument();
    const rent = within(card).getByRole("checkbox", { name: "Move to tomorrow: Pay rent" });
    const notes = within(card).getByRole("checkbox", { name: "Move to tomorrow: Write notes" });
    expect(rent).toBeChecked();
    await userEvent.click(notes);
    await userEvent.click(within(card).getByRole("button", { name: "Finish the day" }));
    const next = addDays(today(), 1);
    await waitFor(() => expect(fake.plans[next]?.items).toEqual([{ key: `task:${b.id}` }]));
    expect(fake.tasks.find((t) => t.id === b.id)!.due).toBe(next);
    expect(await screen.findByText("Day finished. 1 item is in tomorrow's plan.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "End of the day" })).not.toBeInTheDocument();
  });

  it("End the day opens the check before its time, and Close hides it", async () => {
    await startApp({ tasks: [task("Pay rent", { due: today() })], dayReviews: {}, settings: { ...DEFAULT_SETTINGS, auto_brief: false, work_end: "23:59" } });
    await userEvent.click(screen.getByRole("button", { name: "End the day" }));
    const card = await screen.findByRole("region", { name: "End of the day" });
    expect(within(card).getByText("Nothing finished today.")).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Close" }));
    expect(screen.queryByRole("region", { name: "End of the day" })).not.toBeInTheDocument();
  });

  it("the weekly review checks the goals and takes a pick for each item that did not move, with an undo", async () => {
    const t = task("Update the docs");
    const d = task("Old idea");
    const gh = githubItem(5, "assigned", "Old bug");
    const stale = (key: string, title: string, place = "") => ({ key, title, place, last: 1, days: 30 });
    const goals = [
      { id: 980, text: "Ship 2.0", created: 1, updated: 1 },
      { id: 981, text: "Hire a designer", created: 1, updated: 1 },
    ];
    const fake = await startApp({
      tasks: [t, d],
      github: [gh],
      goals: { goals, checked: 1, stale: false },
      goalDone: { 980: 2 },
      weekReviews: [addDays(today(), -7)],
      stale: [stale(`task:${t.id}`, t.title), stale(`task:${d.id}`, d.title), stale(`github:${gh.id}`, gh.title, "GitHub acme/shop")],
    });
    const card = await screen.findByRole("region", { name: "Weekly review" });
    expect(within(card).getByText("2 items done this week")).toBeInTheDocument();
    expect(within(card).getByText("Nothing done for it this week")).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Still right" }));
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "confirm_goals")).toBe(true));

    // Plan: the task waits until that day, and goes into its plan. Undo puts it back.
    const row = (title: string) => within(card).getByText(title).closest("li") as HTMLElement;
    await userEvent.click(within(row("Update the docs")).getByRole("button", { name: "Plan" }));
    const day = addDays(today(), 3);
    fireEvent.change(within(card).getByLabelText("Day for: Update the docs"), { target: { value: day } });
    await userEvent.click(within(card).getByRole("button", { name: "OK" }));
    await waitFor(() => expect(fake.tasks[0].start).toBe(day));
    expect(fake.plans[day].items).toEqual([{ key: `task:${t.id}` }]);
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].start).toBe(null));
    expect(fake.plans[day], "the pick made the plan, so the undo takes it away").toBeUndefined();
    expect(await within(card).findByText("Update the docs")).toBeInTheDocument();

    // Give to someone: it goes on the waiting-for list. GitHub does not change.
    await userEvent.click(within(row("Old bug")).getByRole("button", { name: "Give to…" }));
    await userEvent.type(within(card).getByRole("textbox", { name: "Who does it: Old bug" }), "Ana{Enter}");
    await waitFor(() => expect(fake.waiting.map((w) => [w.what, w.who])).toEqual([["Old bug", "Ana"]]));
    expect(fake.github).toHaveLength(1);
    await waitFor(() => expect(within(card).queryByText("Old bug")).not.toBeInTheDocument());

    // Drop closes a local task.
    await userEvent.click(within(row("Old idea")).getByRole("button", { name: "Drop" }));
    await waitFor(() => expect(fake.tasks[1].done).toBe(true));

    await userEvent.click(within(card).getByRole("button", { name: "Finish the review" }));
    await waitFor(() => expect(fake.weekReviews).toContain(today()));
    expect(await screen.findByText(/Weekly review done. The next one is on/)).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Weekly review" })).not.toBeInTheDocument();
  });

  it("a pick brings in the next item when the review has more items than it shows", async () => {
    const a = task("First old task");
    const b = task("Second old task");
    const stale = (t: ReturnType<typeof task>) => ({ key: `task:${t.id}`, title: t.title, place: "", last: 1, days: 30 });
    await startApp({ tasks: [a, b], weekReviews: [addDays(today(), -7)], stale: [stale(a), stale(b)], staleMax: 1 });
    const card = await screen.findByRole("region", { name: "Weekly review" });
    expect(within(card).getByText("1 more. They come in the next review.")).toBeInTheDocument();
    expect(within(card).queryByText("Second old task")).not.toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Do" }));
    expect(await within(card).findByText("Second old task")).toBeInTheDocument();
    expect(within(card).queryByText("First old task")).not.toBeInTheDocument();
    expect(within(card).queryByText(/more\. They come/)).not.toBeInTheDocument();
  });

  it("a plan that only a review made still asks how to plan the day, and keeps its items", async () => {
    const t = task("Moved from yesterday");
    const fake = await startApp({ tasks: [t], plans: { [today()]: { day: today(), mode: "review", items: [{ key: `task:${t.id}` }], updated: 1 } } });
    expect(screen.getByText("No plan for today yet.")).toBeInTheDocument();
    expect(screen.getByText(/Your review put 1 item in it\./)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "I plan it myself" }));
    await waitFor(() => expect(fake.plans[today()]).toMatchObject({ mode: "self", items: [{ key: `task:${t.id}` }] }));
  });

  it("the weekly review does not show when it is not due, and its button opens it", async () => {
    await startApp();
    expect(screen.queryByRole("region", { name: "Weekly review" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Weekly review" }));
    const card = await screen.findByRole("region", { name: "Weekly review" });
    expect(within(card).getByText(/Nothing is stuck/)).toBeInTheDocument();
    expect(within(card).getByText(/You have no goals/)).toBeInTheDocument();
  });
});

describe("goals", () => {
  it("adds, changes, and removes goals on the home screen", async () => {
    const fake = await startApp();
    const card = screen.getByRole("region", { name: "Goals" });
    expect(within(card).getByText(/Add 3 to 5 goals/)).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Add goals" }));
    await userEvent.type(within(card).getByRole("textbox", { name: "New goal" }), "Ship 2.0{Enter}");
    await userEvent.type(within(card).getByRole("textbox", { name: "New goal" }), "Hire a designer{Enter}");
    const first = await within(card).findByRole("textbox", { name: "Goal: Ship 2.0" });
    await userEvent.clear(first);
    await userEvent.type(first, "Ship 2.0 by 30 Nov{Enter}");
    await waitFor(() => expect(fake.goals.goals[0].text).toBe("Ship 2.0 by 30 Nov"));
    // An empty box does not remove the goal, so the box shows it again.
    const box = await within(card).findByRole("textbox", { name: "Goal: Ship 2.0 by 30 Nov" });
    await userEvent.clear(box);
    await userEvent.tab();
    expect(box).toHaveValue("Ship 2.0 by 30 Nov");
    expect(fake.goals.goals[0].text).toBe("Ship 2.0 by 30 Nov");
    await userEvent.click(within(card).getByRole("button", { name: "Remove goal: Hire a designer" }));
    await userEvent.click(within(card).getByRole("button", { name: "Done" }));
    expect(within(card).getByText("Ship 2.0 by 30 Nov")).toBeInTheDocument();
    expect(fake.goals.goals.map((g) => g.text)).toEqual(["Ship 2.0 by 30 Nov"]);
  });

  it("a goal removed during its edit stays removed", async () => {
    const goals = [{ id: 970, text: "Ship 2.0", created: 1, updated: 1 }];
    const fake = await startApp({ goals: { goals, checked: 1, stale: false } });
    const slowEdit = deferred<void>();
    fake.onEditGoal = () => slowEdit.promise;
    const card = screen.getByRole("region", { name: "Goals" });
    await userEvent.click(within(card).getByRole("button", { name: "Edit" }));
    await userEvent.type(within(card).getByRole("textbox", { name: "Goal: Ship 2.0" }), " now");
    // The click on ✕ ends the edit first (blur). The edit's answer is slow.
    await userEvent.click(within(card).getByRole("button", { name: "Remove goal: Ship 2.0" }));
    slowEdit.resolve();
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "remove_goal")).toBe(true));
    await waitFor(() => expect(within(card).queryByRole("textbox", { name: /^Goal:/ })).not.toBeInTheDocument());
    expect(fake.goals.goals).toEqual([]);
  });

  it("stops at 5 goals", async () => {
    const goals = [1, 2, 3, 4, 5].map((id) => ({ id: 900 + id, text: `Goal ${id}`, created: 1, updated: 1 }));
    await startApp({ goals: { goals, checked: 1, stale: false } });
    await userEvent.click(within(screen.getByRole("region", { name: "Goals" })).getByRole("button", { name: "Edit" }));
    expect(screen.queryByRole("textbox", { name: "New goal" })).not.toBeInTheDocument();
    expect(screen.getByText(/Remove one to add another/)).toBeInTheDocument();
  });

  it("asks to check old goals, and Still right stops the question", async () => {
    const old = Math.floor(Date.now() / 1000) - 40 * 86400;
    const fake = await startApp({ goals: { goals: [{ id: 950, text: "Ship 2.0", created: old, updated: old }], checked: old, stale: true } });
    expect(screen.getByText(/You set these goals 40 days ago/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Still right" }));
    await waitFor(() => expect(screen.queryByText(/You set these goals/)).not.toBeInTheDocument());
    expect(fake.calls.some((c) => c.cmd === "confirm_goals")).toBe(true);
  });

  it("shows the goal of each item that the brief ranked, or No goal", async () => {
    await startApp({
      goals: { goals: [{ id: 960, text: "Ship 2.0", created: 1, updated: 1 }], checked: Math.floor(Date.now() / 1000), stale: false },
      github: [githubItem(9, "review_requested", "Bulk edit"), githubItem(10, "review_requested", "Fix typo")],
      newKeys: ["github:acme/shop#9", "github:acme/shop#10"],
      briefs: {
        [today()]: {
          day: today(),
          written: 1,
          text: "Two reviews.",
          why: { "github:acme/shop#9": "It blocks the release.", "github:acme/shop#10": "It waits 3 days." },
          goal: { "github:acme/shop#9": 960 },
        },
      },
    });
    const list = await screen.findByRole("list", { name: "New" });
    const bulk = within(list).getByText("Bulk edit").closest("li")!;
    expect(within(bulk).getByText("◎ Ship 2.0")).toBeInTheDocument();
    const typo = within(list).getByText("Fix typo").closest("li")!;
    expect(within(typo).getByText("No goal")).toBeInTheDocument();
  });
});

describe("corrections of the brief", () => {
  /** Three GitHub items from acme/shop, each with a reason from today's brief. */
  const ranked = () => ({
    github: [githubItem(9, "review_requested", "Bulk edit"), githubItem(10, "review_requested", "Fix typo"), githubItem(11, "review_requested", "New FAQ")],
    newKeys: ["github:acme/shop#9", "github:acme/shop#10", "github:acme/shop#11"],
    briefs: {
      [today()]: {
        day: today(),
        written: 1,
        text: "Three reviews.",
        why: { "github:acme/shop#9": "It blocks the release.", "github:acme/shop#10": "It waits 3 days.", "github:acme/shop#11": "It is new." },
      },
    },
  });

  it("Not today removes the reason of the item, with an undo", async () => {
    const fake = await startApp(ranked());
    expect(await screen.findByText("It blocks the release.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Not today: Bulk edit" }));
    expect(await screen.findByText("AIfred will not count this for today: Bulk edit")).toBeInTheDocument();
    expect(screen.queryByText("It blocks the release.")).not.toBeInTheDocument();
    expect(fake.corrections).toMatchObject([{ key: "github:acme/shop#9", kind: "not_today", day: today() }]);
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("It blocks the release.")).toBeInTheDocument();
    expect(fake.corrections).toEqual([]);
  });

  it("three Wrong priority clicks from one place teach AIfred a memory fact", async () => {
    const fake = await startApp(ranked());
    for (const t of ["Bulk edit", "Fix typo"]) {
      await userEvent.click(await screen.findByRole("button", { name: `Wrong priority: ${t}` }));
      expect(await screen.findByText(`AIfred will rank this lower from now on: ${t}`)).toBeInTheDocument();
    }
    await userEvent.click(screen.getByRole("button", { name: "Wrong priority: New FAQ" }));
    expect(await screen.findByText(/AIfred learned: GitHub acme\/shop: these items are less important/)).toBeInTheDocument();
    expect(fake.memory.filter((f) => !f.forgotten)).toHaveLength(1);
    // The undo removes the last correction and forgets the fact.
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.memory.filter((f) => !f.forgotten)).toHaveLength(0));
    expect(await screen.findByText("It is new.")).toBeInTheDocument();
  });
});

describe("work time", () => {
  it("Start shows the timer, and Done says the real time next to the guess", async () => {
    let clock = 1_000_000;
    const fake = await startApp({ tasks: [task("Pay rent")], estimates: {}, now: () => clock });
    const key = `task:${fake.tasks[0].id}`;
    fake.estimates[key] = { minutes: 30, by: "you" };
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    const bar = await screen.findByRole("region", { name: "Working on" });
    expect(within(bar).getByText("Pay rent")).toBeInTheDocument();
    clock += 45 * 60;
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(await screen.findByText("Finished: Pay rent. It took 45 m, the guess was 30 m.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Working on" })).not.toBeInTheDocument();
    expect(fake.tasks[0].done).toBe(true);
    expect(fake.workDone[key]).toMatchObject({ minutes: 45, guess: 30 });
    // The backend keeps the time when the task is done. The window does not save it a second time.
    expect(fake.calls.filter((c) => c.cmd === "finish_work")).toEqual([]);
    // Undo opens the task again, and the real time does not count.
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].done).toBe(false));
    await waitFor(() => expect(fake.workDone[key]).toBeUndefined());
  });

  it("Stop keeps the item open, and the timer comes back after a restart", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")], sessions: [{ key: "github:acme/shop#7", started: Math.floor(Date.now() / 1000) - 600, ended: null }] });
    const bar = await screen.findByRole("region", { name: "Working on" });
    expect(within(bar).getByText(/^10 m/)).toBeInTheDocument();
    // The item has no time guess. The default time is not a guess, so the bar shows none.
    expect(within(bar).queryByText(/guessed/)).not.toBeInTheDocument();
    await userEvent.click(within(bar).getByRole("button", { name: "Stop" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Working on" })).not.toBeInTheDocument());
    expect(fake.sessions[0].ended).not.toBeNull();
    expect(screen.getByText("Fix cart")).toBeInTheDocument();
  });

  it("Done on a GitHub item keeps its time, and the item stays until it closes on GitHub", async () => {
    let clock = 1_000_000;
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")], now: () => clock });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    clock += 20 * 60;
    await userEvent.click(within(await screen.findByRole("region", { name: "Working on" })).getByRole("button", { name: "Done" }));
    expect(await screen.findByText("Finished: Fix cart. It took 20 m.")).toBeInTheDocument();
    expect(fake.workDone["github:acme/shop#7"]).toMatchObject({ minutes: 20, guess: null });
    expect(screen.getByText("Fix cart")).toBeInTheDocument();
  });

  it("a failed save of the work time keeps the timer, so Done can run again", async () => {
    let clock = 1_000_000;
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")], now: () => clock });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    clock += 20 * 60;
    fake.failFinishWork = "the disk is full";
    const bar = await screen.findByRole("region", { name: "Working on" });
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(await screen.findByText("Finished: Fix cart. The work time is not saved: the disk is full")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Working on" })).toBeInTheDocument();
    fake.failFinishWork = undefined;
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(await screen.findByText("Finished: Fix cart. It took 20 m.")).toBeInTheDocument();
    expect(screen.queryByRole("region", { name: "Working on" })).not.toBeInTheDocument();
  });

  it("a task that AIfred closes in a chat saves its time once, also when the save fails", async () => {
    let clock = 1_000_000;
    const fake = await startApp({
      tasks: [task("Pay rent")],
      now: () => clock,
      onAsk: () => {
        // AIfred closes the task with its tool while it answers.
        fake.tasks[0].done = true;
        return answer("Done.");
      },
    });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    await screen.findByRole("region", { name: "Working on" });
    clock += 10 * 60;
    fake.failFinishWork = "the disk is full";
    await userEvent.type(homeBox(), "I paid the rent{Enter}");
    expect(await screen.findByText("The work time is not saved: the disk is full")).toBeInTheDocument();
    // A new list after the next answer does not save it again or show the error again.
    fake.calls.length = 0;
    await userEvent.type(homeBox(), "thanks{Enter}");
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "tasks")).toBe(true));
    expect(fake.calls.filter((c) => c.cmd === "finish_work")).toEqual([]);
    expect(screen.getAllByText("The work time is not saved: the disk is full")).toHaveLength(1);
    expect(screen.getByRole("region", { name: "Working on" })).toBeInTheDocument();
  });

  it("a timer that ran for more than 10 hours does not count, and the toast says so", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")], sessions: [{ key: "github:acme/shop#7", started: Math.floor(Date.now() / 1000) - 11 * 3600, ended: null }] });
    await userEvent.click(within(await screen.findByRole("region", { name: "Working on" })).getByRole("button", { name: "Done" }));
    expect(await screen.findByText("Finished: Fix cart. One timer ran for more than 10 hours, so AIfred did not count it.")).toBeInTheDocument();
    expect(fake.workDone["github:acme/shop#7"]).toBeUndefined();
  });

  it("an item that left the list keeps a Done button on its timer", async () => {
    const fake = await startApp({ sessions: [{ key: "github:acme/shop#9", started: Math.floor(Date.now() / 1000) - 600, ended: null }] });
    const bar = await screen.findByRole("region", { name: "Working on" });
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Working on" })).not.toBeInTheDocument());
    expect(fake.sessions[0].ended).not.toBeNull();
  });

  it("a deleted task stops its timer", async () => {
    const fake = await startApp({ tasks: [task("Old idea")] });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    await screen.findByRole("region", { name: "Working on" });
    await openItem("Old idea");
    await userEvent.click(within(details()).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(screen.queryByRole("region", { name: "Working on" })).not.toBeInTheDocument());
    expect(fake.sessions).toEqual([]);
  });

  it("an email with work time: one toast has the time and an Undo for both", async () => {
    let clock = 1_000_000;
    const fake = await startApp({ mail: { fetched_at: clock, reply: [mail("r1", "Price for the plugin", "reply")], sent: [] }, now: () => clock });
    const open = await screen.findByRole("list", { name: "Open" });
    await userEvent.click(within(open).getByRole("button", { name: "Start" }));
    clock += 12 * 60;
    await userEvent.click(within(open).getByRole("button", { name: "No answer needed: Price for the plugin" }));
    expect(await screen.findByText("Hidden: Price for the plugin. It comes back when a new message comes. It took 12 m.")).toBeInTheDocument();
    expect(fake.workDone["gmail:r1"]).toMatchObject({ minutes: 12 });
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("Price for the plugin")).toBeInTheDocument();
    await waitFor(() => expect(fake.workDone["gmail:r1"]).toBeUndefined());
    expect(fake.mailHidden).toEqual([]);
  });
});

describe("the home box", () => {
  it("sends a to-do line to AIfred, which adds the task", async () => {
    const fake = await startApp({
      onAsk: (p) => {
        // AIfred adds the task with its add_task tool while it answers.
        fake.tasks.push(task("Pay rent", { due: addDays(today(), 1), priority: "high", tags: ["home"] }));
        return answer(`Added: Pay rent (${p})`);
      },
    });
    await userEvent.type(homeBox(), "pay rent tomorrow high #home{Enter}");
    expect(await within(panel()!).findByText("Added: Pay rent (pay rent tomorrow high #home)")).toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "add_task"), "the window adds no task itself").toBe(false);
    // The list loads again after the answer, so the new task shows.
    expect(await screen.findByText("Pay rent")).toBeInTheDocument();
    expect(homeBox()).toHaveValue("");
  });

  it("Tab switches to Add task: the line is a task at once, with no AI call, and with an undo", async () => {
    const fake = await startApp();
    await userEvent.click(homeBox());
    expect(screen.getByRole("button", { name: "Ask AIfred" })).toBeInTheDocument();
    await userEvent.keyboard("{Tab}");
    expect(homeBox()).toHaveFocus();
    expect(screen.getByRole("button", { name: "Add task" })).toBeInTheDocument();
    // A new task uses no model: no model picker, a green box, and plain words.
    expect(within(homeForm()).queryByRole("combobox")).not.toBeInTheDocument();
    expect(homeForm()).toHaveClass("task");
    expect(homeBox()).toHaveAttribute("placeholder", "Add a task");
    await userEvent.keyboard("pay rent tomorrow high #home");
    expect(screen.getByText("Pay rent", { selector: "b" })).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByText("Added: Pay rent")).toBeInTheDocument();
    expect(fake.tasks.map((t) => [t.title, t.due, t.priority, t.tags])).toEqual([["Pay rent", addDays(today(), 1), "high", ["home"]]]);
    expect(fake.calls.some((c) => c.cmd === "ask")).toBe(false);
    expect(homeBox()).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks).toEqual([]));
    // Tab again: back to Ask AIfred.
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Tab}");
    expect(screen.getByRole("button", { name: "Ask AIfred" })).toBeInTheDocument();
    expect(within(homeForm()).getByRole("combobox")).toBeInTheDocument();
    expect(homeForm()).not.toHaveClass("task");
    expect(homeBox()).toHaveAttribute("placeholder", "Ask AIfred");
  });

  it("the chip also switches the mode, the mode stays on other pages, and Shift+Tab leaves the box", async () => {
    await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Ask AIfred" }));
    expect(screen.getByRole("button", { name: "Add task" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "AIfred's jobs" }));
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(screen.getByRole("button", { name: "Add task" })).toBeInTheDocument();
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
    expect(homeBox()).not.toHaveFocus();
    expect(screen.getByRole("button", { name: "Add task" }), "Shift+Tab does not switch").toBeInTheDocument();
    // Esc, then Tab: the focus moves forward, and the mode stays.
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Escape}{Tab}");
    expect(homeBox()).not.toHaveFocus();
    expect(screen.getByRole("button", { name: "Add task" }), "Tab after Esc does not switch").toBeInTheDocument();
  });

  it("a task can be added while AIfred answers", async () => {
    const hold = deferred<Answer>();
    const fake = await startApp({ onAsk: () => hold.promise });
    await userEvent.type(homeBox(), "what is due?{Enter}");
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Tab}call Sam{Enter}");
    await waitFor(() => expect(fake.tasks.map((t) => t.title)).toEqual(["Call Sam"]));
    hold.resolve(answer("Nothing."));
  });

  it("a file on a new task stays on the task", async () => {
    const fake = await startApp({ uploadName: "receipt.pdf" });
    await userEvent.upload(screen.getAllByLabelText("Attach files")[0], new File(["%PDF"], "receipt.pdf"));
    await screen.findByText("receipt.pdf");
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Tab}file the receipt{Enter}");
    await waitFor(() => expect(fake.tasks[0]?.files.map((f) => f.name)).toEqual(["receipt.pdf"]));
  });

  it("plan my day opens the Day plan", async () => {
    const fake = await startApp();
    await userEvent.type(homeBox(), "Plan my day");
    expect(screen.getByText("Opens Day plan")).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "ask_side")).toBe(true));
    // Back on Home, the box is empty.
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(homeBox()).toHaveValue("");
  });

  it("sends a question to AIfred and opens the conversation on the right", async () => {
    const fake = await startApp({ tasks: [task("Call Sam")], onAsk: (p) => answer(`Answer to: ${p}`) });
    await userEvent.type(homeBox(), "what is due today?{Enter}");
    const side = panel()!;
    expect(await within(side).findByText("Answer to: what is due today?")).toBeInTheDocument();
    expect(within(side).getByText("what is due today?")).toBeInTheDocument();
    expect(screen.getByText("Call Sam")).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask")!.args).toMatchObject({ prompt: "what is due today?", model: "default", files: [] });
  });

  it("Shift+Enter makes a new line and does not send", async () => {
    const fake = await startApp();
    await userEvent.type(homeBox(), "line one{Shift>}{Enter}{/Shift}line two");
    expect(homeBox()).toHaveValue("line one\nline two");
    expect(fake.calls.some((c) => c.cmd === "ask" || c.cmd === "add_task")).toBe(false);
  });

  it("sends attached files with the question", async () => {
    const fake = await startApp({ uploadName: "notes.txt" });
    await userEvent.upload(screen.getAllByLabelText("Attach files")[0], new File(["hello"], "notes.txt", { type: "text/plain" }));
    expect(await screen.findByText("notes.txt")).toBeInTheDocument();
    await userEvent.type(homeBox(), "what is in this file?{Enter}");
    await within(panel()!).findByText(/You said/);
    const id = fake.uploads[0].id;
    expect(fake.calls.find((c) => c.cmd === "ask")!.args.files).toEqual([id]);
    expect(within(panel()!).getByText("notes.txt")).toBeInTheDocument();
  });

  it("a file sent with a to-do line goes to AIfred with the line", async () => {
    const fake = await startApp({ uploadName: "receipt.pdf" });
    await userEvent.upload(screen.getAllByLabelText("Attach files")[0], new File(["%PDF"], "receipt.pdf"));
    await screen.findByText("receipt.pdf");
    await userEvent.type(homeBox(), "file the receipt{Enter}");
    await within(panel()!).findByText(/You said/);
    expect(fake.calls.find((c) => c.cmd === "ask")!.args).toMatchObject({ prompt: "file the receipt", files: [fake.uploads[0].id] });
  });

});

describe("the conversation", () => {
  it("shows progress while AIfred works, then the answer with its facts", async () => {
    const hold = deferred<Answer>();
    await startApp({
      onAsk: (_p, progress) => {
        progress({ kind: "tool_call", server: "tasks", tool: "overview", input: {} });
        return hold.promise;
      },
    });
    await userEvent.type(homeBox(), "what is open?{Enter}");
    expect(await within(panel()!).findByText("✓ Reading all your open work")).toBeInTheDocument();
    expect(screen.getAllByRole("button", { name: "Send" }).every((b) => (b as HTMLButtonElement).disabled)).toBe(true);
    hold.resolve(answer("Two things.", { memory: ["Sam sends the invoice"] }));
    expect(await within(panel()!).findByText("Two things.")).toBeInTheDocument();
    await userEvent.click(within(panel()!).getByText("Used 1 fact from memory"));
    expect(within(panel()!).getByText("Sam sends the invoice")).toBeVisible();
  });

  it("shows an error in the panel when the question fails", async () => {
    await startApp({ onAsk: () => Promise.reject("claude ended without an answer") });
    await userEvent.type(homeBox(), "what is open?{Enter}");
    expect(await within(panel()!).findByRole("alert")).toHaveTextContent("claude ended without an answer");
  });

  it("the model you pick is saved and used for the next message", async () => {
    const fake = await startApp({ chatModel: "haiku" });
    const picker = screen.getAllByRole("combobox", { name: "Model" })[0];
    await waitFor(() => expect(picker).toHaveValue("haiku"));
    await userEvent.selectOptions(picker, "sonnet");
    await waitFor(() => expect(fake.chatModel).toBe("sonnet"));
    await userEvent.type(homeBox(), "what is open?{Enter}");
    await within(panel()!).findByText(/You said/);
    expect(fake.calls.find((c) => c.cmd === "ask")!.args.model).toBe("sonnet");
    expect(within(panel()!).getByText(/1\.5s · Sonnet/)).toBeInTheDocument();
  });

  it("goes back to the old model when the save fails", async () => {
    await startApp({ fail: { set_chat_model: "database is locked" } });
    const picker = screen.getAllByRole("combobox", { name: "Model" })[0];
    await userEvent.selectOptions(picker, "opus");
    expect(await screen.findByText("Cannot change the model: database is locked")).toBeInTheDocument();
    expect(picker).toHaveValue("");
  });

  it("the rail opens an old conversation, and New starts a new one", async () => {
    const fake = await startApp({
      history: [
        { ts: 1, role: "user", text: "Hi", backend: "claude" },
        { ts: 2, role: "assistant", text: "Hello!", backend: "claude" },
      ],
    });
    await userEvent.click(screen.getByRole("button", { name: "Conversation" }));
    expect(within(panel()!).getByText("Hello!")).toBeInTheDocument();
    await userEvent.click(within(panel()!).getByRole("button", { name: "New" }));
    await waitFor(() => expect(panel()).not.toBeInTheDocument());
    expect(fake.calls.some((c) => c.cmd === "new_conversation")).toBe(true);
  });

  it("reloads the list after the answer, so a task that AIfred added shows", async () => {
    const fake = await startApp({
      onAsk: () => {
        fake.tasks.push(task("Reply to Sam", { added_by: "aifred" }));
        return answer("Added.");
      },
    });
    await userEvent.type(homeBox(), "remind me to reply to Sam?{Enter}");
    expect(await screen.findByRole("button", { name: "Open details: Reply to Sam" })).toBeInTheDocument();
  });
});

describe("the task panel", () => {
  it("changes the title, the due day, the priority, the tags, and the description of your task", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await openItem("Pay rent");
    const d = details();
    const title = within(d).getByRole("textbox", { name: "Task title" });
    await userEvent.clear(title);
    await userEvent.type(title, "Pay the rent{Enter}");
    await waitFor(() => expect(fake.tasks[0].title).toBe("Pay the rent"));
    fireEvent.change(within(d).getByLabelText("Due date"), { target: { value: "2026-12-01" } });
    await waitFor(() => expect(fake.tasks[0].due).toBe("2026-12-01"));
    await userEvent.selectOptions(within(d).getByLabelText("Priority"), "urgent");
    await waitFor(() => expect(fake.tasks[0].priority).toBe("urgent"));
    await userEvent.type(within(d).getByLabelText("Tags"), "home, #money{Enter}");
    await waitFor(() => expect(fake.tasks[0].tags).toEqual(["home", "money"]));
    await userEvent.type(within(d).getByLabelText("Task description"), "Before Friday.");
    await userEvent.tab();
    await waitFor(() => expect(fake.tasks[0].notes).toBe("Before Friday."));
  });

  it("adds, ticks, and removes steps", async () => {
    const fake = await startApp({ tasks: [task("Book flights")] });
    await openItem("Book flights");
    await userEvent.type(within(details()).getByLabelText("New step"), "Pick dates{Enter}");
    await waitFor(() => expect(fake.tasks[0].checklist).toEqual([{ text: "Pick dates", done: false }]));
    await userEvent.click(await within(details()).findByRole("button", { name: "Step done: Pick dates" }));
    await waitFor(() => expect(fake.tasks[0].checklist[0].done).toBe(true));
    await userEvent.click(within(details()).getByRole("button", { name: "Remove step: Pick dates" }));
    await waitFor(() => expect(fake.tasks[0].checklist).toEqual([]));
  });

  it("deletes a task and closes the panel", async () => {
    const fake = await startApp({ tasks: [task("Old idea")] });
    await openItem("Old idea");
    await userEvent.click(within(details()).getByRole("button", { name: "Delete" }));
    await waitFor(() => expect(fake.tasks).toEqual([]));
    expect(screen.queryByRole("dialog", { name: "Task details" })).not.toBeInTheDocument();
  });

  it("shows a GitHub item read-only, with its description, summary, and links", async () => {
    const g = { ...githubItem(7, "assigned", "Fix cart"), links: [{ id: "acme/shop#8", repo: "acme/shop", number: 8, title: "Fix it", url: "https://github.com/acme/shop/pull/8", pull_request: true }] };
    const fake = await startApp({ github: [g], details: { "github:acme/shop#7": "The cart **loses** items." }, summaries: { "github:acme/shop#7": "Items vanish on reload." } });
    await openItem("Fix cart");
    const d = details();
    expect(await within(d).findByText("loses")).toBeInTheDocument();
    expect(within(d).getByText("Items vanish on reload.")).toBeInTheDocument();
    expect(within(d).getByRole("link", { name: "Open in GitHub ↗" })).toHaveAttribute("href", g.url);
    expect(within(d).getByRole("link", { name: "PR #8" })).toBeInTheDocument();
    expect(within(d).queryByRole("button", { name: /Mark as finished|Close in ClickUp/ })).not.toBeInTheDocument();
    expect(fake.calls.map((c) => c.cmd).filter((c) => c.startsWith("github_") && c !== "github_items")).toEqual([]);
  });

  it("asks AIfred about the item in its own chat, with the fast model first", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")], onAskSide: (p) => answer(`About it: ${p}`) });
    await openItem("Pay rent");
    const key = `task:${fake.tasks[0].id}`;
    await userEvent.type(within(details()).getByRole("textbox", { name: "Ask AIfred about this item" }), "when is it due?{Enter}");
    expect(await within(details()).findByText("About it: when is it due?")).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask_side")!.args).toMatchObject({ thread: `item:${key}`, model: "haiku" });
    expect(fake.history).toEqual([]);
  });

  it("a file with no text in the item chat asks AIfred to look at the file", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")], uploadName: "bill.pdf" });
    await openItem("Pay rent");
    await userEvent.upload(within(details()).getByLabelText("Attach files"), new File(["%PDF"], "bill.pdf"));
    await within(details()).findByText("bill.pdf");
    await userEvent.click(within(details()).getByRole("button", { name: "Ask" }));
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "ask_side")?.args).toMatchObject({ prompt: "Look at these files.", files: [fake.uploads[0].id] }));
  });

  it("the item chat remembers Default as its model", async () => {
    localStorage.setItem("aifred-model-item", "");
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await openItem("Pay rent");
    await userEvent.type(within(details()).getByRole("textbox", { name: "Ask AIfred about this item" }), "hi{Enter}");
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "ask_side")?.args).toMatchObject({ model: "default" }));
    localStorage.removeItem("aifred-model-item");
  });

  it("the time guess changes in steps of 15 minutes", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await openItem("Pay rent");
    expect(within(details()).getByText("30 m")).toBeInTheDocument();
    await userEvent.click(within(details()).getByRole("button", { name: "More time" }));
    expect(within(details()).getByText("45 m")).toBeInTheDocument();
    await waitFor(() => expect(fake.estimates[`task:${fake.tasks[0].id}`]).toEqual({ minutes: 45, by: "you" }));
  });
});

describe("tabs on the right side", () => {
  it("show the conversation and the task when both are open, and mark a new answer", async () => {
    const hold = deferred<Answer>();
    await startApp({ tasks: [task("Pay rent")], onAsk: () => hold.promise });
    await userEvent.type(homeBox(), "what is open?{Enter}");
    await openItem("Pay rent");
    const tabs = screen.getByRole("tablist", { name: "Open on the right" });
    expect(within(tabs).getByRole("tab", { name: /Pay rent/ })).toHaveAttribute("aria-selected", "true");
    expect(panel()).not.toBeVisible();
    hold.resolve(answer("Two things."));
    // Both panels have the tabs. One of them is hidden.
    expect((await screen.findAllByTitle("New answer")).length).toBeGreaterThan(0);
    await userEvent.click(screen.getAllByRole("tab", { name: /Conversation/ })[0]);
    expect(panel()).toBeVisible();
    expect(within(panel()!).getByText("Two things.")).toBeInTheDocument();
    expect(screen.queryAllByTitle("New answer")).toEqual([]);
  });

  it("the width of the side changes with the arrow keys, is kept, and double-click resets it", async () => {
    await startApp({ tasks: [task("Pay rent")] });
    await openItem("Pay rent");
    const edge = within(details()).getByRole("separator");
    // jsdom has no layout: the panel is 0 px wide, so the width starts at the smallest, 300 px.
    fireEvent.keyDown(edge, { key: "ArrowLeft" });
    expect(localStorage.getItem("aifred-side-w")).toBe("300");
    fireEvent.doubleClick(edge);
    expect(localStorage.getItem("aifred-side-w")).toBeNull();
  });
});

describe("day plan", () => {
  it("I plan it myself: add items, change the time, and take them out", async () => {
    const fake = await startApp({ tasks: [task("Pay rent"), task("Call Sam")], github: [githubItem(7, "review_requested", "Fix cart")] });
    await userEvent.click(screen.getByRole("button", { name: "Day plan" }));
    await userEvent.click(screen.getByRole("button", { name: /I plan it myself/ }));
    expect(await screen.findByText("Your plan is empty")).toBeInTheDocument();
    const off = screen.getByRole("list", { name: "Not in today's plan" });
    await userEvent.click(within(within(off).getByText("Fix cart").closest("li")!).getByRole("button", { name: /Add/ }));
    await waitFor(() => expect(fake.plans[today()].items).toEqual([{ key: "github:acme/shop#7" }]));
    expect(fake.plans[today()].mode).toBe("self");
    expect(screen.getAllByText("9:00").length).toBeGreaterThan(0);
    await userEvent.click(screen.getAllByRole("button", { name: "More time" })[0]);
    expect(await screen.findByText("9:00–9:45")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Take out of the plan: Fix cart" }));
    await waitFor(() => expect(fake.plans[today()].items).toEqual([]));
  });

  it("puts plan items after a meeting, and Prep asks AIfred about the meeting", async () => {
    const t = task("Pay rent");
    const fake = await startApp({
      tasks: [t],
      plans: { [today()]: { day: today(), mode: "self", items: [{ key: `task:${t.id}` }], updated: 1 } },
      calendar: { fetched_at: Math.floor(Date.now() / 1000), events: [meeting("m1", "Weekly sync", "09:00", "10:00")] },
    });
    expect(await screen.findByText(/Calendar/, { selector: ".sync span" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Day plan" }));
    expect(await screen.findByText("Weekly sync")).toBeInTheDocument();
    expect(screen.getByText("Josh, Ana")).toBeInTheDocument();
    // The task does not go over the meeting: it starts when the meeting ends.
    expect(screen.getByText("10:00–10:30")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Prepare for: Weekly sync" }));
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "ask_side")?.args.prompt).toMatch(/Prepare me for my calendar event today at 9:00–10:00 \(event id [^)]+\)/));
  });

  it("shows a calendar error in the top bar", async () => {
    await startApp({ calendarError: "gws failed: not logged in" });
    expect(await screen.findByTitle("gws failed: not logged in")).toHaveTextContent("Calendar: error");
  });

  it("Plan it for me asks AIfred in the plan chat, and shows the plan it saved", async () => {
    const fake = await startApp({ tasks: [task("Pay rent", { priority: "urgent" })] });
    const key = `task:${fake.tasks[0].id}`;
    fake.onAskSide = (p, _progress, args) => {
      fake.plans[args.thread.slice(5)] = { day: args.thread.slice(5), mode: "ai", items: [{ key, why: "It is urgent." }], updated: 1 };
      return answer(`I put [the rent](item:${key}) first.`);
    };
    await userEvent.click(screen.getByRole("button", { name: "Plan it for me" }));
    const chat = screen.getByRole("complementary", { name: "Plan chat" });
    expect(await within(chat).findByText(/first\./)).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask_side")!.args).toMatchObject({ thread: `plan:${today()}`, prompt: "Plan my day." });
    expect(await screen.findByText("✦ Made by AIfred")).toBeInTheDocument();
    expect(screen.getByText("It is urgent.")).toBeInTheDocument();
    // The link in the answer opens the item, with tabs to go back to the chat.
    await userEvent.click(within(chat).getByRole("button", { name: "the rent" }));
    expect(screen.getByRole("tab", { name: /Plan chat/ })).toHaveAttribute("aria-selected", "false");
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(screen.getByText("Pay rent", { selector: ".strip b" })).toBeInTheDocument();
    expect(screen.getByText("AIfred made this plan", { exact: false })).toBeInTheDocument();
  });

  it("Start over removes the plan and the plan chat", async () => {
    const rent = task("Pay rent");
    const fake = await startApp({
      tasks: [rent],
      plans: { [today()]: { day: today(), mode: "self", items: [{ key: `task:${rent.id}` }], updated: 1 } },
      side: { [`plan:${today()}`]: [{ id: 1, ts: 1, role: "user", text: "Plan my day", files: [] }] },
    });
    await userEvent.click(screen.getByRole("button", { name: "Day plan" }));
    expect(await screen.findByText("Made by you")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Start over" }));
    expect(await screen.findByText("How do you want to plan today?")).toBeInTheDocument();
    expect(fake.plans[today()]).toBeUndefined();
    expect(fake.side[`plan:${today()}`]).toBeUndefined();
  });

  it("a plan change keeps the plan items of a list that did not load", async () => {
    const rent = task("Pay rent");
    const bill = task("Pay bill");
    const gh = "github:acme/shop#7";
    const fake = await startApp({
      tasks: [rent, bill],
      fail: { github_items: "Cannot reach GitHub" },
      plans: { [today()]: { day: today(), mode: "self", items: [{ key: gh, why: "A review." }, { key: `task:${rent.id}` }], updated: 1 } },
    });
    await userEvent.click(screen.getByRole("button", { name: "Day plan" }));
    expect(await screen.findByText("Made by you")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Take out of the plan: Pay rent" }));
    await waitFor(() => expect(fake.plans[today()].items).toEqual([{ key: gh, why: "A review." }]));
  });

  it("a plan change waits until the plan of the day is loaded", async () => {
    const load = deferred<void>();
    const fake = await startApp({ tasks: [task("Pay rent")], onDayPlan: () => load.promise });
    await userEvent.click(await screen.findByRole("button", { name: "+ Plan" }));
    expect(await screen.findByText(/still loading/)).toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "set_day_plan")).toBe(false);
    load.resolve();
    await userEvent.click(screen.getByRole("button", { name: "+ Plan" }));
    await waitFor(() => expect(fake.plans[today()].items).toEqual([{ key: `task:${fake.tasks[0].id}` }]));
  });

  it("+ Plan on the home list adds the item to today's plan", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await userEvent.click(await screen.findByRole("button", { name: "+ Plan" }));
    expect(await screen.findByText("Planned 9:00")).toBeInTheDocument();
    expect(fake.plans[today()].items).toEqual([{ key: `task:${fake.tasks[0].id}` }]);
  });
});

describe("GitHub list", () => {
  it("asks GitHub once at start, and again with the reload button", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")] });
    await screen.findByText("Fix cart");
    expect(fake.githubFetches).toBe(1);
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() => expect(fake.githubFetches).toBe(2));
  });

  it("reloads on the timer from the settings, and never when it is 'only when I click reload'", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({ github: [githubItem(7, "assigned")], settings: { ...DEFAULT_SETTINGS, refresh_minutes: 5 } });
      await waitFor(() => expect(fake.githubFetches).toBe(1));
      await vi.advanceTimersByTimeAsync(5 * 60_000);
      await waitFor(() => expect(fake.githubFetches).toBe(2));
    } finally {
      vi.useRealTimers();
    }
  });

  it("an item closed on GitHub leaves the list on the next reload", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")] });
    await screen.findByText("Fix cart");
    fake.github = [];
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() => expect(screen.queryByText("Fix cart")).not.toBeInTheDocument());
  });

  it("shows the saved list with a warning when GitHub cannot be reached", async () => {
    await startApp({ github: [githubItem(7, "assigned", "Fix cart")], githubWarning: "Cannot reach GitHub, so this is the saved list." });
    expect(await screen.findByText("Fix cart")).toBeInTheDocument();
    expect(screen.getByText("Cannot reach GitHub, so this is the saved list.")).toBeInTheDocument();
  });
});

describe("ClickUp", () => {
  const connected = { clickupToken: "pk_test" };

  it("makes no ClickUp call when it is not connected", async () => {
    const fake = await startApp({ clickup: [clickupItem("86a", "Write notes")] });
    expect(screen.queryByText("Write notes")).not.toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "clickup_items")).toBe(false);
  });

  it("the tick closes the task in ClickUp, and Undo puts the old status back", async () => {
    const fake = await startApp({ ...connected, clickup: [clickupItem("86a", "Write notes", { status: "review" })] });
    await userEvent.click(await screen.findByRole("button", { name: "Close in ClickUp: Write notes" }));
    expect(await screen.findByText("Closed in ClickUp: Write notes")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Open details: Write notes" })).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo: Write notes" }));
    expect(await screen.findByRole("button", { name: "Open details: Write notes" })).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "clickup_undo_done")!.args).toEqual({ id: "86a", status: "review" });
  });

  it("a second close keeps the undo of the first", async () => {
    const fake = await startApp({ ...connected, clickup: [clickupItem("86a", "Write notes", { status: "review" }), clickupItem("86b", "Call Bob")] });
    await userEvent.click(await screen.findByRole("button", { name: "Close in ClickUp: Write notes" }));
    await userEvent.click(await screen.findByRole("button", { name: "Close in ClickUp: Call Bob" }));
    expect(await screen.findByText("Closed in ClickUp: Call Bob")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo: Write notes" }));
    await waitFor(() => expect(fake.calls.find((c) => c.cmd === "clickup_undo_done")!.args).toEqual({ id: "86a", status: "review" }));
    expect(screen.getByRole("button", { name: "Undo: Call Bob" })).toBeInTheDocument();
  });

  it("a failed close keeps the task and says why", async () => {
    await startApp({ ...connected, clickup: [clickupItem("86a", "Write notes")], fail: { clickup_mark_done: "ClickUp said 403: no access" } });
    await userEvent.click(await screen.findByRole("button", { name: "Close in ClickUp: Write notes" }));
    expect(await screen.findByRole("alert")).toHaveTextContent("ClickUp said 403: no access");
    expect(screen.getByRole("button", { name: "Open details: Write notes" })).toBeInTheDocument();
  });

  it("connects with a token on the Settings page, then loads the tasks", async () => {
    const fake = await startApp({ clickup: [clickupItem("86a", "Write notes")] });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.type(screen.getByLabelText("ClickUp"), "pk_123");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText("Connected as okky.")).toBeInTheDocument();
    expect(fake.clickupToken).toBe("pk_123");
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(await screen.findByText("Write notes")).toBeInTheDocument();
  });
});

describe("Google", () => {
  it("saves the registration, connects, loads the calendar, and disconnects", async () => {
    const fake = await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const g = screen.getByRole("region", { name: "Google" });
    await userEvent.type(within(g).getByLabelText("Client ID"), "bad");
    await userEvent.type(within(g).getByLabelText("Client secret"), "s");
    await userEvent.click(within(g).getByRole("button", { name: "Save registration" }));
    expect(await within(g).findByText(/does not look like a Google client ID/)).toBeInTheDocument();
    await userEvent.clear(within(g).getByLabelText("Client ID"));
    await userEvent.type(within(g).getByLabelText("Client ID"), "1-a.apps.googleusercontent.com");
    await userEvent.click(within(g).getByRole("button", { name: "Save registration" }));
    const loads = () => fake.calls.filter((c) => c.cmd === "calendar_events").length;
    const before = loads();
    await userEvent.click(await within(g).findByRole("button", { name: "Connect Google" }));
    expect(await within(g).findByText("me@x.com")).toBeInTheDocument();
    expect(within(g).getByText("Calendar: allowed")).toBeInTheDocument();
    await waitFor(() => expect(loads()).toBeGreaterThan(before));
    expect(await screen.findByText(/Calendar/, { selector: ".sync span" })).toBeInTheDocument();
    await userEvent.click(within(g).getByRole("button", { name: "Disconnect" }));
    expect(await within(g).findByRole("button", { name: "Connect Google" })).toBeInTheDocument();
    expect(fake.google.connected).toBe(false);
  });

  it("lets the user start the login again while one waits", async () => {
    const first = deferred<void>();
    let logins = 0;
    await startApp({
      google: { client_id: "1-a.apps.googleusercontent.com", secret_set: true, connected: false, email: null, features: [["Calendar", false]], optional: ["Gmail"], off: [] },
      onConnectGoogle: () => (++logins === 1 ? first.promise : undefined),
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const g = screen.getByRole("region", { name: "Google" });
    await userEvent.click(await within(g).findByRole("button", { name: "Connect Google" }));
    // The user closed the browser tab: the button still works.
    const waiting = await within(g).findByRole("button", { name: "Finish the login in your browser…" });
    expect(waiting).toBeEnabled();
    await userEvent.click(waiting);
    expect(await within(g).findByText("me@x.com")).toBeInTheDocument();
    // The first login stops. Its error does not hide the new result.
    first.reject("the login stopped, because a new login started");
    await new Promise((r) => setTimeout(r, 0));
    expect(within(g).queryByText(/login stopped/)).not.toBeInTheDocument();
    expect(within(g).getByText("Connected as me@x.com.")).toBeInTheDocument();
  });

  it("asks to connect again when the login does not allow the calendar", async () => {
    await startApp({ google: { client_id: "1-a.apps.googleusercontent.com", secret_set: true, connected: true, email: "me@x.com", features: [["Calendar", false]], optional: ["Gmail"], off: [] } });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByText(/Google needs more access for Calendar/)).toBeInTheDocument();
  });
});

describe("Gmail and the waiting list", () => {
  const now = () => Math.floor(Date.now() / 1000);

  it("an email that waits for your answer is an item, and 'No answer needed' hides it with an undo", async () => {
    const fake = await startApp({ mail: { fetched_at: now(), reply: [mail("r1", "Price for the plugin", "reply", { unread: true })], sent: [] } });
    const open = await screen.findByRole("list", { name: "Open" });
    expect(within(open).getByText("Price for the plugin")).toBeInTheDocument();
    expect(within(open).getByText(/Email · from Ana Lee/)).toBeInTheDocument();
    expect(within(open).getByText("Unread · Waits 3 days")).toBeInTheDocument();
    await userEvent.click(within(open).getByRole("button", { name: "No answer needed: Price for the plugin" }));
    await waitFor(() => expect(screen.queryByText("Price for the plugin")).not.toBeInTheDocument());
    expect(fake.mailHidden).toEqual(["r1"]);
    expect(await screen.findByText(/Hidden: Price for the plugin. It comes back when a new message comes./)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("Price for the plugin")).toBeInTheDocument();
    expect(fake.mailHidden).toEqual([]);
  });

  it("a Gmail load that started before 'No answer needed' does not show the email again", async () => {
    const fake = await startApp({ mail: { fetched_at: now(), reply: [mail("r1", "Price for the plugin", "reply")], sent: [] } });
    const open = await screen.findByRole("list", { name: "Open" });
    await within(open).findByText("Price for the plugin");
    // A reload starts. Its list still has the email, and its answer waits.
    const hold = deferred<void>();
    fake.onGmailThreads = () => hold.promise;
    await userEvent.click(screen.getByRole("button", { name: "Reload the list" }));
    await waitFor(() => expect(fake.calls.filter((c) => c.cmd === "gmail_threads" && c.args.force).length).toBe(1));
    await userEvent.click(within(open).getByRole("button", { name: "No answer needed: Price for the plugin" }));
    await waitFor(() => expect(screen.queryByText("Price for the plugin")).not.toBeInTheDocument());
    fake.onGmailThreads = undefined;
    hold.resolve();
    // The old answer is not shown: the window reads the saved list again.
    await waitFor(() => expect(fake.calls.some((c) => c.cmd === "gmail_saved")).toBe(true));
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText("Price for the plugin")).not.toBeInTheDocument();
  });

  it("a double click on Track adds the email once", async () => {
    const fake = await startApp({ mail: { fetched_at: now(), reply: [], sent: [mail("s1", "Contract draft", "sent")] } });
    const card = await screen.findByRole("region", { name: "Waiting for" });
    const track = within(card).getByRole("button", { name: "Track" });
    fireEvent.click(track);
    fireEvent.click(track);
    await within(card).findByRole("list", { name: "Waiting for list" });
    expect(fake.calls.filter((c) => c.cmd === "add_waiting").length).toBe(1);
    expect(fake.waiting.length).toBe(1);
  });

  it("a deleted email item goes back to the suggestions with no Gmail download", async () => {
    const fake = await startApp({ mail: { fetched_at: now(), reply: [], sent: [mail("s1", "Contract draft", "sent")] } });
    const card = await screen.findByRole("region", { name: "Waiting for" });
    await userEvent.click(within(card).getByRole("button", { name: "Track" }));
    const list = await within(card).findByRole("list", { name: "Waiting for list" });
    const loads = fake.calls.filter((c) => c.cmd === "gmail_threads").length;
    await userEvent.click(within(list).getByRole("button", { name: /^Delete: / }));
    const sugg = await within(card).findByRole("list", { name: "Emails with no answer yet" });
    expect(within(sugg).getByText("Contract draft")).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "gmail_threads").length).toBe(loads);
  });

  it("shows an email in the panel with the day it waits since", async () => {
    await startApp({ mail: { fetched_at: now(), reply: [mail("r1", "Price for the plugin", "reply", { direct: false, messages: 2 })], sent: [] } });
    await openItem("Price for the plugin");
    const d = details();
    expect(within(d).getByText("Waits since")).toBeInTheDocument();
    expect(within(d).getByText(/2 messages · you are in Cc/)).toBeInTheDocument();
    expect(within(d).getByRole("button", { name: "No answer needed" })).toBeInTheDocument();
  });

  it("tracks a sent email, closes the item, and adds one by hand", async () => {
    const fake = await startApp({
      mail: { fetched_at: now(), reply: [], sent: [mail("s1", "Contract draft", "sent"), mail("s2", "Lunch?", "sent", { people: ["Bo"] })] },
    });
    const card = await screen.findByRole("region", { name: "Waiting for" });
    const sugg = within(card).getByRole("list", { name: "Emails with no answer yet" });
    expect(within(sugg).getByText(/to Ana Lee · sent/)).toBeInTheDocument();
    await userEvent.click(within(sugg).getAllByRole("button", { name: "Track" })[0]);
    const list = await within(card).findByRole("list", { name: "Waiting for list" });
    expect(within(list).getByText(/An answer to "Contract draft"/)).toBeInTheDocument();
    await waitFor(() => expect(within(card).queryByText("Contract draft")).not.toBeInTheDocument());
    expect(fake.waiting[0].thread).toBe("s1");

    // Hide the other suggestion: you wait for nothing.
    await userEvent.click(within(card).getByRole("button", { name: "Hide" }));
    await waitFor(() => expect(within(card).queryByRole("list", { name: "Emails with no answer yet" })).not.toBeInTheDocument());
    expect(fake.mailHidden).toEqual(["s2"]);

    // A check day. Typing the year gives days like 0002-01-04 first: only the
    // last day saves, once.
    const box = within(list).getByLabelText(/Check day: An answer to/);
    const before = fake.calls.filter((c) => c.cmd === "update_waiting").length;
    for (const v of ["0002-01-04", "0020-01-04", "0203-01-04", "2030-01-04"]) fireEvent.change(box, { target: { value: v } });
    await waitFor(() => expect(fake.waiting[0].follow_up).toBe("2030-01-04"));
    expect(fake.calls.filter((c) => c.cmd === "update_waiting").length - before).toBe(1);
    expect(await within(list).findByText(/Check Fri 4 Jan 2030/)).toBeInTheDocument();

    await userEvent.click(within(list).getByRole("button", { name: "Got it" }));
    expect(await within(list).findByText("closed by you")).toBeInTheDocument();
    expect(fake.waiting[0].done).toBe(true);

    await userEvent.click(within(card).getByRole("button", { name: "Add" }));
    await userEvent.type(within(card).getByLabelText("What you wait for"), "The signed contract");
    await userEvent.type(within(card).getByLabelText("Who you wait for"), "Cy");
    await userEvent.click(within(card).getByRole("button", { name: "Add" }));
    expect(await within(card).findByText(/The signed contract/)).toBeInTheDocument();
    expect(fake.waiting.map((w) => w.who)).toEqual(["Ana Lee", "Cy"]);
  });

  it("shows only a small link when there is nothing to wait for", async () => {
    await startApp();
    await userEvent.click(await screen.findByRole("button", { name: "+ Waiting for someone?" }));
    expect(screen.getByRole("region", { name: "Waiting for" })).toBeInTheDocument();
    expect(screen.getByLabelText("What you wait for")).toHaveFocus();
  });

  it("shows a Gmail error, and asks to connect again when the login does not allow Gmail", async () => {
    await startApp({
      mailError: "Gmail said: Rate limit",
      google: { client_id: "1-a.apps.googleusercontent.com", secret_set: true, connected: true, email: "me@x.com", features: [["Calendar", true], ["Gmail", false]], optional: ["Gmail"], off: [] },
    });
    expect(await screen.findByText("Gmail: Gmail said: Rate limit")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByText(/Google needs more access for Gmail/)).toBeInTheDocument();
  });
});

describe("the Gmail switch", () => {
  it("turns Gmail off: no emails, no error, and no 'connect again' note", async () => {
    const fake = await startApp({
      mailError: "Gmail API has not been used in project 123 or it is disabled",
      google: { client_id: "1-a.apps.googleusercontent.com", secret_set: true, connected: true, email: "me@x.com", features: [["Calendar", true], ["Gmail", false]], optional: ["Gmail"], off: [] },
    });
    expect(await screen.findByText(/Gmail: Gmail API has not been used/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const g = screen.getByRole("region", { name: "Google" });
    expect(await within(g).findByText(/Google needs more access for Gmail/)).toBeInTheDocument();
    await userEvent.click(within(g).getByRole("checkbox", { name: "Use Gmail" }));
    expect(await within(g).findByText("Gmail is off. AIfred removed the saved data of Gmail.")).toBeInTheDocument();
    expect(fake.google.off).toEqual(["Gmail"]);
    expect(within(g).queryByText(/Google needs more access/)).not.toBeInTheDocument();
    expect(within(g).getByText("Gmail: turned off")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    await waitFor(() => expect(screen.queryByText(/Gmail: Gmail API/)).not.toBeInTheDocument());
    expect(screen.queryByText(/Gmail/, { selector: ".sync span" })).not.toBeInTheDocument();
  });

  it("turns Gmail on again and asks to connect again", async () => {
    await startApp({
      google: { client_id: "1-a.apps.googleusercontent.com", secret_set: true, connected: true, email: "me@x.com", features: [["Calendar", true], ["Gmail", false]], optional: ["Gmail"], off: ["Gmail"] },
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const g = screen.getByRole("region", { name: "Google" });
    await userEvent.click(await within(g).findByRole("checkbox", { name: "Use Gmail" }));
    expect(await within(g).findByText("Gmail is on. Connect again, and allow it.")).toBeInTheDocument();
  });
});

describe("settings page", () => {
  it("saves the reload time and the work day", async () => {
    const fake = await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.selectOptions(screen.getByLabelText("Reload GitHub and ClickUp items"), "Only when I click reload");
    fireEvent.change(screen.getByLabelText("Work day starts"), { target: { value: "08:30" } });
    await userEvent.selectOptions(screen.getByLabelText("Lunch length"), "No lunch break");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(fake.settings).toMatchObject({ refresh_minutes: 0, work_start: "08:30", lunch_minutes: 0 });
  });

  it("shows a failed save", async () => {
    await startApp({ fail: { set_settings: "read-only folder" } });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.selectOptions(screen.getByLabelText("Reload GitHub and ClickUp items"), "Every hour");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Not saved: read-only folder")).toBeInTheDocument();
  });

  it("checks for a new version and restarts into it", async () => {
    const fake = await startApp({ updateTo: "0.2.0" });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(await screen.findByText(/Version 0.2.0 is downloaded/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Restart now" }));
    await waitFor(() => expect(fake.restarts).toBe(1));
  });
});

describe("memory page", () => {
  const openMemory = () => userEvent.click(screen.getByRole("button", { name: "What AIfred knows about you" }));

  it("adds a fact, and forgets and restores one", async () => {
    const fake = await startApp({ memory: [fact("Old job at Acme", { added_by: "aifred" })] });
    await openMemory();
    await userEvent.type(screen.getByPlaceholderText("Add a fact and press Enter"), "I like short answers.{Enter}");
    expect(await screen.findByText("I like short answers.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Forget: Old job at Acme" }));
    await userEvent.click(await screen.findByRole("button", { name: /Forgotten \(1\)/ }));
    await userEvent.click(screen.getByRole("button", { name: "Restore: Old job at Acme" }));
    await waitFor(() => expect(fake.memory[0].forgotten).toBeNull());
  });

  it("a failed save says so and keeps the text", async () => {
    await startApp({ fail: { add_fact: "database is locked" } });
    await openMemory();
    await userEvent.type(screen.getByPlaceholderText("Add a fact and press Enter"), "Note{Enter}");
    expect(await screen.findByText("Not saved: database is locked")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Add a fact and press Enter")).toHaveValue("Note");
  });
});

describe("AIfred's jobs page", () => {
  const openJobs = () => userEvent.click(screen.getByRole("button", { name: "AIfred's jobs" }));

  it("shows the jobs and the latest runs, and pauses and deletes a job", async () => {
    const now = Math.floor(Date.now() / 1000);
    const fake = await startApp({
      jobs: [
        job("j1", "Timesheet reminder"),
        job("j2", "Watch PR #123", { action: "agent", message: "Tell me when PR #123 is merged.", status: "done", can_start: false }),
        job("j3", "Daily check", { action: "agent", status: "off" }),
      ],
      jobRuns: [
        { id: 2, job: "j2", planned: now - 60, started: now - 60, finished: now - 50, state: "ok", late: true, note: "", output: "PR #123 is merged.", cost_usd: 0.02 },
        { id: 1, job: "j1", planned: now - 99, started: null, finished: null, state: "skipped", late: false, note: "AIfred was not running at this time.", output: "", cost_usd: null },
      ],
    });
    await openJobs();
    expect(await screen.findByText("Timesheet reminder")).toBeInTheDocument();
    expect(screen.getByText(/Every Monday at 09:00/)).toBeInTheDocument();
    // A job that ended waits under "Ended". Its run shows with its answer.
    expect(screen.queryByRole("button", { name: "Start: Watch PR #123" })).not.toBeInTheDocument();
    expect(screen.getByText("PR #123 is merged.")).toBeInTheDocument();
    expect(screen.getByText(/late, planned/)).toBeInTheDocument();
    expect(screen.queryByText("AIfred was not running at this time."), "skipped runs are hidden").not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Ended (2)" }));
    // A job with no more times has no Start: it needs a new time first.
    expect(screen.queryByRole("button", { name: "Start: Watch PR #123" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Start: Daily check" })).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Pause: Timesheet reminder" }));
    await waitFor(() => expect(fake.jobs[0].status).toBe("paused"));
    expect(await screen.findByRole("button", { name: "Start: Timesheet reminder" })).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Delete: Timesheet reminder" }));
    await waitFor(() => expect(screen.queryByText("Timesheet reminder")).not.toBeInTheDocument());
    expect(fake.jobs.map((j) => j.id)).toEqual(["j2", "j3"]);
  });

  it("says when there are no jobs", async () => {
    await startApp();
    await openJobs();
    expect(await screen.findByText("AIfred has no jobs now.")).toBeInTheDocument();
  });

  it("a load error goes away when the next load works", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({ fail: { jobs: "database is locked" } });
      await openJobs();
      expect(await screen.findByText("Cannot load the jobs: database is locked")).toBeInTheDocument();
      delete fake.fail.jobs;
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() => expect(screen.queryByText(/Cannot load the jobs/)).not.toBeInTheDocument());
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows why the jobs cannot load", async () => {
    await startApp({ fail: { jobs: "database is locked" } });
    await openJobs();
    expect(await screen.findByText("Cannot load the jobs: database is locked")).toBeInTheDocument();
  });
});

describe("errors the window must show", () => {
  it("a task file that cannot load shows the reason, not an empty list", async () => {
    await startApp({ fail: { tasks: "cannot read the database" } });
    expect(await screen.findByText("Cannot load your tasks: cannot read the database")).toBeInTheDocument();
    expect(screen.queryByText(/Nothing to do/)).not.toBeInTheDocument();
  });

  it("a failed start-up load shows the reason", async () => {
    fakeBackend({ fail: { history: "bad history file" } });
    render(<App />);
    expect(await screen.findByText("Cannot load the conversation: bad history file")).toBeInTheDocument();
  });
});
