// The full window with a fake backend: Home (brief and list), the task
// panel, the chats, Calendar, Settings, and Memory.
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { invoke } from "@tauri-apps/api/core";
import { emit } from "@tauri-apps/api/event";
import App from "./App";
import { DEFAULT_SETTINGS, type Answer, type Brief, type Mission } from "./api";
import { addDays, dayOf, shortDay } from "./items";
import {
  answer,
  clickupItem,
  deferred,
  fact,
  fakeBackend,
  githubItem,
  job,
  mail,
  meeting,
  task,
} from "./test/fakeBackend";

// The panel stays in the page, hidden, behind the task tab. A role query skips a hidden element.
const panel = () =>
  document.querySelector<HTMLElement>('aside[aria-label="Conversation"]');
const details = () => screen.getByRole("dialog", { name: "Task details" });
const homeBox = () =>
  screen.getByRole("textbox", { name: "Add a task or ask AIfred" });
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

const openItem = async (title: string) =>
  userEvent.click(
    await screen.findByRole("button", { name: `Open details: ${title}` }),
  );

describe("home", () => {
  it("shows the brief, and the items of all sources in groups, with no chat panel", async () => {
    await startApp({
      clickupToken: "pk_test",
      tasks: [
        task("Pay rent", { due: today(), priority: "high" }),
        task("Old bill", { due: addDays(today(), -2) }),
      ],
      github: [githubItem(7, "review_requested", "Fix cart")],
      clickup: [clickupItem("86a", "Write notes", { custom_id: "DEV-42" })],
    });
    expect(screen.getByRole("region", { name: "Brief" })).toBeInTheDocument();
    const now = await screen.findByRole("list", { name: "Needs you now" });
    expect(within(now).getByText("Old bill")).toBeInTheDocument();
    expect(within(now).getByText("2 days late")).toBeInTheDocument();
    expect(
      within(screen.getByRole("list", { name: "Today" })).getByText("Pay rent"),
    ).toBeInTheDocument();
    const open = await screen.findByRole("list", { name: "Open" });
    expect(within(open).getByText("Fix cart")).toBeInTheDocument();
    expect(within(open).getByText("Review")).toBeInTheDocument();
    expect(await within(open).findByText("Write notes")).toBeInTheDocument();
    expect(
      within(open).getByText(/DEV-42 · Rymera · Sprint 12/),
    ).toBeInTheDocument();
    expect(panel()).not.toBeInTheDocument();
  });

  it("puts new GitHub items under New, with the reason from the brief", async () => {
    await startApp({
      github: [githubItem(9, "review_requested", "Bulk edit")],
      newKeys: ["github:acme/shop#9"],
      briefs: {
        [today()]: {
          day: today(),
          written: 1,
          text: "Review [the bulk edit](item:github:acme/shop#9).",
          why: { "github:acme/shop#9": "It blocks the release." },
        },
      },
    });
    const list = await screen.findByRole("list", { name: "New" });
    expect(within(list).getByText("Bulk edit")).toBeInTheDocument();
    expect(
      within(list).getByText("It blocks the release."),
    ).toBeInTheDocument();
  });

  it("writes the brief by itself once a day when the setting is on, and the item links open the item", async () => {
    const fake = await startApp({
      settings: { ...DEFAULT_SETTINGS, auto_brief: true },
      tasks: [task("Pay rent")],
      onWriteBrief: (day) => ({
        day,
        written: 1,
        text: "One thing: [the rent](item:task:t-x).",
        why: {},
      }),
    });
    const key = `task:${fake.tasks[0].id}`;
    // The second answer names the task. Change it only after the first
    // write: the app can start that write at any time.
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1),
    );
    fake.onWriteBrief = (day) => ({
      day,
      written: 1,
      text: `One thing: [the rent](item:${key}).`,
      why: {},
    });
    // The first answer named an item that is not in the list: plain text, no button.
    expect(await screen.findByText("the rent")).not.toHaveClass("ref");
    await userEvent.click(screen.getByRole("button", { name: "Write again" }));
    await userEvent.click(
      await screen.findByRole("button", { name: "the rent" }),
    );
    expect(
      within(details()).getByRole("textbox", { name: "Task title" }),
    ).toHaveValue("Pay rent");
    expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(2);
  });

  it("while AIfred prepares the day, the home page says so and does not show the task list", async () => {
    let done: (b: Brief) => void = () => {};
    const fake = await startApp({
      settings: { ...DEFAULT_SETTINGS, auto_brief: true },
      tasks: [task("Pay rent")],
      onWriteBrief: () => new Promise<Brief>((r) => (done = r)),
    });
    const card = await screen.findByRole("region", {
      name: "Preparing your day",
    });
    expect(within(card).getByRole("status")).toHaveTextContent(
      /looks up your tasks/,
    );
    expect(screen.queryByText("Pay rent")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Brief" }),
    ).not.toBeInTheDocument();
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1),
    );
    done({ day: today(), written: 1, text: "Pay the rent.", why: {} });
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Preparing your day" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      await screen.findByRole("region", { name: "Brief" }),
    ).toBeInTheDocument();
  });

  it("on start, the home page waits for the lists that the questions need, so the task list does not flash", async () => {
    let release: () => void = () => {};
    const held = new Promise<void>((r) => (release = r));
    await startApp({
      tasks: [task("Pay rent")],
      github: [githubItem(7, "assigned", "Sync")],
      onGithubItems: () => held,
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: ["github:acme/shop#7"],
          held: true,
        },
      },
    });
    expect(
      await screen.findByRole("region", { name: "Loading your day" }),
    ).toBeInTheDocument();
    expect(screen.queryByText("Pay rent")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Help AIfred understand" }),
    ).not.toBeInTheDocument();
    release();
    expect(
      await screen.findByRole("region", { name: "Help AIfred understand" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Loading your day" }),
    ).not.toBeInTheDocument();
  });

  it("a later reload of a list keeps the question card and the text in its box", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent")],
      github: [githubItem(7, "assigned", "Sync")],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: ["github:acme/shop#7"],
        },
      },
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    const box = within(card).getByRole("textbox", {
      name: "Your answer about Sync",
    });
    await userEvent.type(box, "half done");
    // A reload starts, and its answer waits.
    const hold = deferred<void>();
    fake.onGithubItems = () => hold.promise;
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() => expect(fake.githubFetches).toBe(2));
    expect(
      screen.queryByRole("region", { name: "Loading your day" }),
    ).not.toBeInTheDocument();
    expect(
      within(
        screen.getByRole("region", { name: "Help AIfred understand" }),
      ).getByRole("textbox", { name: "Your answer about Sync" }),
    ).toHaveValue("half done");
    hold.resolve();
  });

  it("does not write the brief by itself when the setting is off, and shows a failed write", async () => {
    const fake = await startApp({
      onWriteBrief: () => Promise.reject("claude is not logged in"),
    });
    expect(
      screen.getByText(/has not written today's brief yet/),
    ).toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "write_brief")).toBe(false);
    await userEvent.click(
      screen.getByRole("button", { name: "Write the brief" }),
    );
    expect(
      await screen.findByText(
        "Cannot write the brief: claude is not logged in",
      ),
    ).toBeInTheDocument();
  });

  it("keeps tasks with a later start day under Later, and done tasks under Finished", async () => {
    await startApp({
      tasks: [
        task("Book flights", { start: addDays(today(), 5) }),
        task("Old one", { done: true, done_at: 5 }),
      ],
    });
    expect(
      await screen.findByText("Nothing to do today. Some tasks start later."),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Show later (1)" }),
    );
    expect(
      within(screen.getByRole("list", { name: "Later" })).getByText(
        "Book flights",
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Show finished (1)" }),
    );
    expect(
      within(screen.getByRole("list", { name: "Finished" })).getByText(
        "Old one",
      ),
    ).toBeInTheDocument();
  });

  it("a finished task has an undo", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await userEvent.click(
      await screen.findByRole("button", { name: "Finish: Pay rent" }),
    );
    expect(await screen.findByText("Finished: Pay rent")).toBeInTheDocument();
    expect(fake.tasks[0].done).toBe(true);
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].done).toBe(false));
  });

  it("Tomorrow moves a task to tomorrow, with an undo", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent", { due: today() })],
    });
    await userEvent.click(
      await screen.findByRole("button", { name: "Tomorrow" }),
    );
    await waitFor(() => expect(fake.tasks[0].due).toBe(addDays(today(), 1)));
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].due).toBe(today()));
  });

  it("Tomorrow moves a planned task to the plan of tomorrow, and the undo puts it back", async () => {
    const t = task("Pay rent", { due: today() });
    const key = `task:${t.id}`;
    const next = addDays(today(), 1);
    const fake = await startApp({
      tasks: [t],
      plans: {
        [today()]: {
          day: today(),
          mode: "self",
          items: [{ key, at: 600 }],
          updated: 1,
        },
      },
    });
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "Tomorrow" }))[0],
    );
    await waitFor(() =>
      expect(fake.plans[next]?.items.map((i) => i.key)).toEqual([key]),
    );
    expect(fake.plans[today()].items).toEqual([]);
    expect(fake.tasks[0].due).toBe(next);
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([{ key, at: 600 }]),
    );
    expect(fake.plans[next].items).toEqual([]);
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
      plans: {
        [today()]: {
          day: today(),
          mode: "self",
          items: [{ key: `task:${c.id}` }],
          updated: 1,
        },
      },
      settings: {
        ...DEFAULT_SETTINGS,
        auto_brief: false,
        work_start: "00:00",
        work_end: hhmm(10),
      },
    });
    const card = await screen.findByRole("region", { name: "End of the day" });
    expect(within(card).getByText("Send invoice")).toBeInTheDocument();
    const rent = within(card).getByRole("checkbox", {
      name: "Move to tomorrow: Pay rent",
    });
    const notes = within(card).getByRole("checkbox", {
      name: "Move to tomorrow: Write notes",
    });
    expect(rent).toBeChecked();
    await userEvent.click(notes);
    await userEvent.click(
      within(card).getByRole("button", { name: "Finish the day" }),
    );
    const next = addDays(today(), 1);
    await waitFor(() =>
      expect(fake.plans[next]?.items).toEqual([{ key: `task:${b.id}` }]),
    );
    expect(fake.tasks.find((t) => t.id === b.id)!.due).toBe(next);
    expect(
      await screen.findByText("Day finished. 1 item is in tomorrow's plan."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "End of the day" }),
    ).not.toBeInTheDocument();
  });

  it("End the day opens the check before its time, and Close hides it", async () => {
    await startApp({
      tasks: [task("Pay rent", { due: today() })],
      dayReviews: {},
      settings: { ...DEFAULT_SETTINGS, auto_brief: false, work_end: "23:59" },
    });
    await userEvent.click(screen.getByRole("button", { name: "End the day" }));
    const card = await screen.findByRole("region", { name: "End of the day" });
    expect(
      within(card).getByText("Nothing finished today."),
    ).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Close" }));
    expect(
      screen.queryByRole("region", { name: "End of the day" }),
    ).not.toBeInTheDocument();
  });

  it("the weekly review checks the goals and takes a pick for each item that did not move, with an undo", async () => {
    const t = task("Update the docs");
    const d = task("Old idea");
    const gh = githubItem(5, "assigned", "Old bug");
    const stale = (key: string, title: string, place = "") => ({
      key,
      title,
      place,
      last: 1,
      days: 30,
    });
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
      stale: [
        stale(`task:${t.id}`, t.title),
        stale(`task:${d.id}`, d.title),
        stale(`github:${gh.id}`, gh.title, "GitHub acme/shop"),
      ],
    });
    const card = await screen.findByRole("region", { name: "Weekly review" });
    expect(
      within(card).getByText("2 items done this week"),
    ).toBeInTheDocument();
    expect(
      within(card).getByText("Nothing done for it this week"),
    ).toBeInTheDocument();
    await userEvent.click(
      within(card).getByRole("button", { name: "Still right" }),
    );
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "confirm_goals")).toBe(true),
    );

    // Plan: the task waits until that day, and goes into its plan. Undo puts it back.
    const row = (title: string) =>
      within(card).getByText(title).closest("li") as HTMLElement;
    await userEvent.click(
      within(row("Update the docs")).getByRole("button", { name: "Plan" }),
    );
    const day = addDays(today(), 3);
    fireEvent.change(within(card).getByLabelText("Day for: Update the docs"), {
      target: { value: day },
    });
    await userEvent.click(within(card).getByRole("button", { name: "OK" }));
    await waitFor(() => expect(fake.tasks[0].start).toBe(day));
    expect(fake.plans[day].items).toEqual([{ key: `task:${t.id}` }]);
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks[0].start).toBe(null));
    expect(
      fake.plans[day],
      "the pick made the plan, so the undo takes it away",
    ).toBeUndefined();
    expect(
      await within(card).findByText("Update the docs"),
    ).toBeInTheDocument();

    // Give to someone: it goes on the waiting-for list. GitHub does not change.
    await userEvent.click(
      within(row("Old bug")).getByRole("button", { name: "Give to…" }),
    );
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Who does it: Old bug" }),
      "Ana{Enter}",
    );
    await waitFor(() =>
      expect(fake.waiting.map((w) => [w.what, w.who])).toEqual([
        ["Old bug", "Ana"],
      ]),
    );
    expect(fake.github).toHaveLength(1);
    await waitFor(() =>
      expect(within(card).queryByText("Old bug")).not.toBeInTheDocument(),
    );

    // Drop closes a local task.
    await userEvent.click(
      within(row("Old idea")).getByRole("button", { name: "Drop" }),
    );
    await waitFor(() => expect(fake.tasks[1].done).toBe(true));

    await userEvent.click(
      within(card).getByRole("button", { name: "Finish the review" }),
    );
    await waitFor(() => expect(fake.weekReviews).toContain(today()));
    expect(
      await screen.findByText(/Weekly review done. The next one is on/),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Weekly review" }),
    ).not.toBeInTheDocument();
  });

  it("a pick brings in the next item when the review has more items than it shows", async () => {
    const a = task("First old task");
    const b = task("Second old task");
    const stale = (t: ReturnType<typeof task>) => ({
      key: `task:${t.id}`,
      title: t.title,
      place: "",
      last: 1,
      days: 30,
    });
    await startApp({
      tasks: [a, b],
      weekReviews: [addDays(today(), -7)],
      stale: [stale(a), stale(b)],
      staleMax: 1,
    });
    const card = await screen.findByRole("region", { name: "Weekly review" });
    expect(
      within(card).getByText("1 more. They come in the next review."),
    ).toBeInTheDocument();
    expect(within(card).queryByText("Second old task")).not.toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Do" }));
    expect(
      await within(card).findByText("Second old task"),
    ).toBeInTheDocument();
    expect(within(card).queryByText("First old task")).not.toBeInTheDocument();
    expect(
      within(card).queryByText(/more\. They come/),
    ).not.toBeInTheDocument();
  });

  it("a plan that only a review made still asks how to plan the day, and keeps its items", async () => {
    const t = task("Moved from yesterday");
    const fake = await startApp({
      tasks: [t],
      plans: {
        [today()]: {
          day: today(),
          mode: "review",
          items: [{ key: `task:${t.id}` }],
          updated: 1,
        },
      },
    });
    expect(screen.getByText("No plan for today yet.")).toBeInTheDocument();
    expect(
      screen.getByText(/Your review put 1 item in it\./),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "I plan it myself" }),
    );
    await waitFor(() =>
      expect(fake.plans[today()]).toMatchObject({
        mode: "self",
        items: [{ key: `task:${t.id}` }],
      }),
    );
  });

  it("the weekly review does not show when it is not due, and its button opens it", async () => {
    await startApp();
    expect(
      screen.queryByRole("region", { name: "Weekly review" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Weekly review" }),
    );
    const card = await screen.findByRole("region", { name: "Weekly review" });
    expect(within(card).getByText(/Nothing is stuck/)).toBeInTheDocument();
    expect(within(card).getByText(/You have no goals/)).toBeInTheDocument();
  });
});

describe("subtasks", () => {
  const lists = () => ({
    clickupToken: "pk_test",
    github: [githubItem(12, "your_pr", "DEV-42: round the tax")],
    clickup: [clickupItem("86a", "Checkout totals", { custom_id: "DEV-42" })],
  });

  it("a tree of subtasks shows as one row", async () => {
    await startApp({
      ...lists(),
      links: [{ key: "github:acme/shop#12", main: "clickup:86a" }],
    });
    const open = await screen.findByRole("list", { name: "Open" });
    expect(within(open).getAllByRole("listitem")).toHaveLength(1);
    expect(within(open).getByText("Subtasks")).toBeInTheDocument();
    // The source under the row opens that item.
    await userEvent.click(
      within(open).getByRole("button", {
        name: "Open details: DEV-42: round the tax",
      }),
    );
    expect(
      within(details()).getByText("main row", { exact: false }),
    ).toBeInTheDocument();
    // The "Same work?" card is gone.
    expect(
      screen.queryByRole("region", { name: "Same work?" }),
    ).not.toBeInTheDocument();
  });

  it("Tomorrow does not show on a row that a late linked item keeps in place", async () => {
    await startApp({
      ...lists(),
      tasks: [task("Tax work", { id: "t1", due: today() })],
      clickup: [
        clickupItem("86a", "Checkout totals", {
          custom_id: "DEV-42",
          due: Date.now() - 3 * 86400_000,
        }),
      ],
      links: [{ key: "clickup:86a", main: "task:t1" }],
    });
    const open = await screen.findByRole("list", { name: "Needs you now" });
    expect(within(open).getByText("Tax work")).toBeInTheDocument();
    expect(
      within(open).queryByRole("button", { name: "Tomorrow" }),
    ).not.toBeInTheDocument();
  });

  it("a deleted task gives its subtasks to its parent, on screen too", async () => {
    const fake = await startApp({
      ...lists(),
      tasks: [task("Tax work", { id: "t1" })],
      links: [
        { key: "task:t1", main: "clickup:86a" },
        { key: "github:acme/shop#12", main: "task:t1" },
      ],
    });
    await openItem("Tax work");
    await userEvent.click(
      within(details()).getByRole("button", { name: "Delete" }),
    );
    await waitFor(() =>
      expect(fake.links).toEqual([
        { key: "github:acme/shop#12", main: "clickup:86a" },
      ]),
    );
    await openItem("Checkout totals");
    const group = await within(details()).findByRole("list", {
      name: "Task tree",
    });
    await waitFor(() =>
      expect(
        within(group).queryByText(/Not in your list now/),
      ).not.toBeInTheDocument(),
    );
    expect(within(group).getAllByRole("listitem")).toHaveLength(2);
  });

  it("a move fails like the real app: an unknown item, a loop, or a tree that is too large", async () => {
    const fake = await startApp({
      ...lists(),
      tasks: [task("Tax work", { id: "t1" })],
    });
    await expect(
      invoke("link_items", { main: "task:t1", key: "github:none#1" }),
    ).rejects.toBe("AIfred does not know the item github:none#1");
    fake.links = [{ key: "task:t1", main: "clickup:86a" }];
    await expect(
      invoke("link_items", { main: "task:t1", key: "clickup:86a" }),
    ).rejects.toBe(
      "task:t1 is a subtask of clickup:86a. An item cannot go under its own subtask",
    );
    fake.links = Array.from({ length: 9 }, (_, n) => ({
      key: `task:x${n}`,
      main: "clickup:86a",
    }));
    fake.tasks.push(
      ...Array.from({ length: 9 }, (_, n) => task(`X ${n}`, { id: `x${n}` })),
    );
    await expect(
      invoke("link_items", { main: "clickup:86a", key: "task:t1" }),
    ).rejects.toBe("one tree can have at most 10 items");
  });

  it("adds a subtask from the item panel, moves it out, and undoes each change", async () => {
    const fake = await startApp({
      ...lists(),
      tasks: [task("Tax rounding notes")],
    });
    await openItem("Checkout totals");
    await userEvent.click(
      within(details()).getByRole("button", { name: "+ Add a subtask" }),
    );
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Find an item to add as a subtask",
      }),
      "tax",
    );
    // The search finds the open items by title, not the item itself.
    await userEvent.click(
      within(details()).getByRole("button", {
        name: "Add as subtask: DEV-42: round the tax",
      }),
    );
    expect(fake.links).toMatchObject([
      { key: "github:acme/shop#12", main: "clickup:86a", by: "you" },
    ]);
    expect(
      await screen.findByText(
        "DEV-42: round the tax is now a subtask of Checkout totals",
      ),
    ).toBeInTheDocument();
    const group = await within(details()).findByRole("list", {
      name: "Task tree",
    });
    expect(within(group).getByText("This item")).toBeInTheDocument();
    await userEvent.click(
      within(group).getByRole("button", {
        name: "Move out: DEV-42: round the tax",
      }),
    );
    await waitFor(() => expect(fake.links).toEqual([]));
    expect(
      within(details()).queryByRole("list", { name: "Task tree" }),
    ).not.toBeInTheDocument();
    // The undo puts it back under its old parent.
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.links).toMatchObject([
        { key: "github:acme/shop#12", main: "clickup:86a" },
      ]),
    );
  });

  it("adds a blocker from the item panel, marks the row, and undoes a remove", async () => {
    const fake = await startApp({
      ...lists(),
      tasks: [task("Deploy the tax fix", { id: "t1" })],
    });
    await openItem("Deploy the tax fix");
    await userEvent.click(
      within(details()).getByRole("button", { name: "+ Add a blocker" }),
    );
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Find an item that must finish first",
      }),
      "tax",
    );
    // The search does not show the item itself.
    expect(
      within(details()).queryByRole("button", {
        name: "Blocked by: Deploy the tax fix",
      }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      within(details()).getByRole("button", {
        name: "Blocked by: DEV-42: round the tax",
      }),
    );
    expect(fake.blockers).toMatchObject([
      { key: "task:t1", blocker: "github:acme/shop#12", by: "you" },
    ]);
    expect(
      await screen.findByText(
        "Deploy the tax fix now waits for DEV-42: round the tax",
      ),
    ).toBeInTheDocument();
    // The row in the list shows the open blocker.
    const row = document.getElementById("row-task:t1")!;
    // The mark opens the blocker. Its panel shows what waits for it.
    await userEvent.click(
      within(row).getByRole("button", {
        name: "Open blocker: DEV-42: round the tax",
      }),
    );
    const blocks = await within(details()).findByRole("list", {
      name: "Blocks",
    });
    await userEvent.click(
      within(blocks).getByRole("button", {
        name: "Does not block: Deploy the tax fix",
      }),
    );
    await waitFor(() =>
      expect(fake.blockers).toMatchObject([{ removed: true }]),
    );
    expect(
      within(row).queryByRole("button", { name: /Open blocker/ }),
    ).not.toBeInTheDocument();
    // The undo puts the link back.
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.blockers).toEqual([
        {
          key: "task:t1",
          blocker: "github:acme/shop#12",
          by: "you",
          why: "",
          removed: false,
          at: expect.any(Number),
        },
      ]),
    );
  });

  it("the blocker search does not show items that wait for the item, also through other items", async () => {
    await startApp({
      ...lists(),
      tasks: [
        task("Deploy the tax fix", { id: "t1" }),
        task("Write tax notes", { id: "t2" }),
      ],
      // Deploy waits for DEV-42, and the notes wait for Deploy.
      blockers: [
        { key: "task:t1", blocker: "github:acme/shop#12", by: "you", why: "" },
        { key: "task:t2", blocker: "task:t1", by: "you", why: "" },
      ],
    });
    await openItem("DEV-42: round the tax");
    await userEvent.click(
      within(details()).getByRole("button", { name: "+ Add a blocker" }),
    );
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Find an item that must finish first",
      }),
      "tax",
    );
    expect(
      within(details()).getByText("No open item matches."),
    ).toBeInTheDocument();
  });

  it("the blocker search shows an item whose old link goes through a finished item", async () => {
    await startApp({
      ...lists(),
      tasks: [
        task("Deploy the tax fix", { id: "t1", done: true }),
        task("Write tax notes", { id: "t2" }),
      ],
      // The finished Deploy waited for DEV-42, and the notes wait for Deploy. That old link is asleep.
      blockers: [
        { key: "task:t1", blocker: "github:acme/shop#12", by: "you", why: "" },
        { key: "task:t2", blocker: "task:t1", by: "you", why: "" },
      ],
    });
    await openItem("DEV-42: round the tax");
    await userEvent.click(
      within(details()).getByRole("button", { name: "+ Add a blocker" }),
    );
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Find an item that must finish first",
      }),
      "notes",
    );
    expect(within(details()).getByText("Write tax notes")).toBeInTheDocument();
  });

  it("a link that AIfred set because you asked in a chat says so in the panel", async () => {
    await startApp({
      ...lists(),
      tasks: [task("Deploy the tax fix", { id: "t1" })],
      blockers: [
        { key: "task:t1", blocker: "github:acme/shop#12", by: "chat", why: "" },
      ],
    });
    await openItem("Deploy the tax fix");
    const list = await within(details()).findByRole("list", {
      name: "Blocked by",
    });
    expect(list).toHaveTextContent("Set by AIfred in a chat");
  });

  it("a blocker that is not in the list shows in the panel, but not in the count", async () => {
    await startApp({
      ...lists(),
      tasks: [task("Deploy the tax fix", { id: "t1" })],
      blockers: [
        { key: "task:t1", blocker: "github:acme/shop#99", by: "you", why: "" },
      ],
    });
    await openItem("Deploy the tax fix");
    const list = await within(details()).findByRole("list", {
      name: "Blocked by",
    });
    expect(within(list).getByText("Not in your list now")).toBeInTheDocument();
    // The count is the same as the row mark: no open blocker.
    expect(
      within(details())
        .getByText("Blocked by", { selector: ".d-sec" })
        .querySelector(".c"),
    ).toBeNull();
    expect(
      within(document.getElementById("row-task:t1")!).queryByRole("button", {
        name: /Open blocker/,
      }),
    ).not.toBeInTheDocument();
  });

  it("a ClickUp disconnect loads the blockers again", async () => {
    const fake = await startApp({
      ...lists(),
      clickupToken: "pk_test",
      tasks: [task("Deploy the tax fix", { id: "t1" })],
      blockers: [
        { key: "task:t1", blocker: "clickup:86a", by: "you", why: "" },
      ],
    });
    const loads = () =>
      fake.calls.filter((c) => c.cmd === "item_blockers").length;
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    const before = loads();
    await userEvent.click(
      await screen.findByRole("button", { name: "Disconnect" }),
    );
    await waitFor(() => expect(loads()).toBeGreaterThan(before));
  });

  it("a finished blocker does not mark the row, and AIfred's reason shows in the panel", async () => {
    await startApp({
      ...lists(),
      tasks: [
        task("Deploy", { id: "t1" }),
        task("Write notes", { id: "t2", done: true }),
      ],
      blockers: [
        { key: "task:t1", blocker: "task:t2", by: "you", why: "" },
        {
          key: "task:t1",
          blocker: "github:acme/shop#12",
          by: "aifred",
          why: "The PR has the fix.",
        },
      ],
    });
    const row = await waitFor(() => document.getElementById("row-task:t1")!);
    await waitFor(() =>
      expect(
        within(row).getByRole("button", {
          name: "Open blocker: DEV-42: round the tax",
        }),
      ).toBeInTheDocument(),
    );
    expect(
      within(row).queryByRole("button", { name: "Open blocker: Write notes" }),
    ).not.toBeInTheDocument();
    await openItem("Deploy");
    const list = await within(details()).findByRole("list", {
      name: "Blocked by",
    });
    expect(list).toHaveTextContent("finished");
    expect(list).toHaveTextContent("AIfred: The PR has the fix.");
  });

  it("a blocker fails like the real app: an item cannot wait for itself or for an item that waits for it", async () => {
    const fake = await startApp({
      ...lists(),
      tasks: [task("Deploy", { id: "t1" })],
    });
    await expect(
      invoke("add_blocker", { key: "task:t1", blocker: "task:t1" }),
    ).rejects.toBe("an item cannot wait for itself");
    fake.blockers = [
      { key: "github:acme/shop#12", blocker: "task:t1", by: "you", why: "" },
    ];
    await expect(
      invoke("add_blocker", { key: "task:t1", blocker: "github:acme/shop#12" }),
    ).rejects.toBe(
      "github:acme/shop#12 already waits for task:t1. Two items cannot wait for each other",
    );
  });

  it("a new brief that marks a task done shows the task as done", async () => {
    const invoice = task("Send the invoice");
    const fake = await startApp({ tasks: [invoice] });
    expect(await screen.findByText("Send the invoice")).toBeInTheDocument();
    const loads = () => fake.calls.filter((c) => c.cmd === "tasks").length;
    // Like matches::apply in the brief call: the task is done, with a note.
    fake.onWriteBrief = async (day) => {
      fake.tasks[0].done = true;
      fake.tasks[0].done_at = 9;
      return {
        day,
        written: 9,
        text: "One thing.",
        why: {},
        done: [
          {
            id: 1,
            key: `task:${invoice.id}`,
            title: "Send the invoice",
            step: "",
            by_key: "clickup:86z",
            by_title: "Invoice for Ana",
            why: "The same invoice.",
          },
        ],
      };
    };
    const before = loads();
    await userEvent.click(
      screen.getByRole("button", { name: /Write the brief|Write again/ }),
    );
    expect(
      await screen.findByLabelText("What AIfred marked done"),
    ).toBeInTheDocument();
    // The task list loads again, so the task shows as done.
    await waitFor(() => expect(loads()).toBeGreaterThan(before));
  });

  it("the brief says which task AIfred marked done, and Undo opens it again", async () => {
    const invoice = task("Send the invoice", { done: true, done_at: 5 });
    const fake = await startApp({
      tasks: [invoice],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          done: [
            {
              id: 4,
              key: `task:${invoice.id}`,
              title: "Send the invoice",
              step: "",
              by_key: "clickup:86z",
              by_title: "Invoice for Ana",
              why: "The same invoice closed in ClickUp.",
            },
          ],
        },
      },
    });
    const note = await screen.findByLabelText("What AIfred marked done");
    expect(note).toHaveTextContent(
      "I marked Send the invoice done, because Invoice for Ana is finished. The same invoice closed in ClickUp.",
    );
    await userEvent.click(
      within(note).getByRole("button", { name: "Undo: Send the invoice" }),
    );
    await waitFor(() => expect(fake.tasks[0].done).toBe(false));
    expect(fake.calls.find((c) => c.cmd === "undo_done_match")?.args).toEqual({
      id: 4,
    });
    expect(
      screen.queryByLabelText("What AIfred marked done"),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByText(
        'Open again: Send the invoice. AIfred does not mark it done for "Invoice for Ana" again.',
      ),
    ).toBeInTheDocument();
  });

  it("the brief says what AIfred sorted, and Undo puts the item back", async () => {
    const move = {
      id: 1,
      ts: 2,
      key: "github:acme/shop#12",
      title: "DEV-42: round the tax",
      main: "clickup:86a",
      main_title: "Checkout totals",
      why: "The pull request names DEV-42.",
    };
    const fake = await startApp({
      ...lists(),
      links: [
        {
          key: "github:acme/shop#12",
          main: "clickup:86a",
          by: "aifred",
          why: move.why,
        },
      ],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          moves: [move],
        },
      },
    });
    const note = await screen.findByLabelText("What AIfred sorted");
    expect(note).toHaveTextContent(
      "I made DEV-42: round the tax a subtask of Checkout totals. The pull request names DEV-42.",
    );
    // The item names open the items.
    await userEvent.click(
      within(note).getByRole("button", { name: "Checkout totals" }),
    );
    expect(
      await within(details()).findByRole("list", { name: "Task tree" }),
    ).toBeInTheDocument();
    await userEvent.click(
      within(note).getByRole("button", { name: "Undo: DEV-42: round the tax" }),
    );
    await waitFor(() => expect(fake.links).toEqual([]));
    expect(fake.calls.find((c) => c.cmd === "undo_moves")?.args).toEqual({
      ids: [1],
    });
    expect(
      screen.queryByLabelText("What AIfred sorted"),
    ).not.toBeInTheDocument();
    expect(
      await screen.findByText(
        "Put back: DEV-42: round the tax. AIfred does not move it again.",
      ),
    ).toBeInTheDocument();
  });

  it("many moves of AIfred are one list, with one line for each parent and reason", async () => {
    const m = (
      id: number,
      key: string,
      title: string,
      main: string,
      main_title: string,
      why: string,
    ) => ({ id, ts: 2, key, title, main, main_title, why });
    await startApp({
      ...lists(),
      tasks: [task("Tax notes", { id: "t1" }), task("Tax email", { id: "t2" })],
      links: [
        { key: "github:acme/shop#12", main: "clickup:86a", by: "aifred" },
        { key: "task:t1", main: "clickup:86a", by: "aifred" },
        { key: "task:t2", main: "task:t1", by: "aifred" },
      ],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          moves: [
            m(
              1,
              "github:acme/shop#12",
              "DEV-42: round the tax",
              "clickup:86a",
              "Checkout totals",
              "Both are about the tax.",
            ),
            m(
              2,
              "task:t1",
              "Tax notes",
              "clickup:86a",
              "Checkout totals",
              "Both are about the tax.",
            ),
            m(
              3,
              "task:t2",
              "Tax email",
              "task:t1",
              "Tax notes",
              "The email is about the notes.",
            ),
            // The user moved it again: the brief does not show this move.
            m(
              4,
              "task:t2",
              "Tax email",
              "clickup:86a",
              "Checkout totals",
              "Old.",
            ),
          ],
        },
      },
    });
    const note = await screen.findByLabelText("What AIfred sorted");
    // The list is folded, so it does not make the brief long.
    const fold = note.querySelector("details")!;
    expect(fold).not.toHaveAttribute("open");
    expect(note).toHaveTextContent("I reorganized 3 tasks.");
    await userEvent.click(within(note).getByText("I reorganized 3 tasks."));
    expect(fold).toHaveAttribute("open");
    const lines = within(note).getAllByRole("listitem");
    expect(lines).toHaveLength(2);
    expect(lines[0]).toHaveTextContent(
      "DEV-42: round the tax and Tax notes are now subtasks of Checkout totals. Both are about the tax.",
    );
    expect(lines[1]).toHaveTextContent(
      "Tax email is now a subtask of Tax notes. The email is about the notes.",
    );
  });

  it("a sort in the background shows at once, and a long note shows 5 lines first", async () => {
    const titles = ["A", "B", "C", "D", "E", "F", "G"];
    const fake = await startApp({
      ...lists(),
      tasks: titles.map((t) => task(t, { id: t })),
      briefs: {
        [today()]: { day: today(), written: 3, text: "One thing.", why: {} },
      },
    });
    await screen.findByText("One thing.");
    expect(
      screen.queryByLabelText("What AIfred sorted"),
    ).not.toBeInTheDocument();
    // Each task goes under its own parent, so each is one line.
    const parents = [
      "clickup:86a",
      "github:acme/shop#12",
      "task:A",
      "task:B",
      "task:C",
      "task:D",
      "task:E",
    ];
    const moves = titles.slice(1).map((t, n) => ({
      id: n + 1,
      ts: 4,
      key: `task:${t}`,
      title: t,
      main: parents[n],
      main_title: parents[n],
      why: `Reason ${n + 1}.`,
    }));
    fake.links = moves.map((m) => ({
      key: m.key,
      main: m.main,
      by: "aifred" as const,
      why: m.why,
    }));
    fake.briefs[today()] = { ...fake.briefs[today()], moves };
    await emit("items-sorted");
    const note = await screen.findByLabelText("What AIfred sorted");
    expect(within(note).getAllByRole("listitem")).toHaveLength(5);
    await userEvent.click(
      within(note).getByRole("button", { name: "Show 1 more" }),
    );
    expect(within(note).getAllByRole("listitem")).toHaveLength(6);
  });

  it("a subtask can have its own subtasks, and the tree shows each level", async () => {
    await startApp({
      ...lists(),
      tasks: [task("Tax rounding notes", { id: "t1" })],
      links: [
        { key: "github:acme/shop#12", main: "clickup:86a" },
        {
          key: "task:t1",
          main: "github:acme/shop#12",
          by: "aifred",
          why: "The notes are about the pull request",
        },
      ],
    });
    const open = await screen.findByRole("list", { name: "Open" });
    expect(within(open).getAllByRole("listitem")).toHaveLength(1);
    await openItem("Checkout totals");
    const tree = await within(details()).findByRole("list", {
      name: "Task tree",
    });
    const rows = within(tree).getAllByRole("listitem");
    expect(rows.map((r) => r.style.paddingLeft)).toEqual([
      "0px",
      "16px",
      "32px",
    ]);
    expect(
      within(tree).getByText(/AIfred: The notes are about the pull request/),
    ).toBeInTheDocument();
    // The top item has no parent, so it cannot move out.
    expect(
      within(tree).queryByRole("button", { name: "Move out: Checkout totals" }),
    ).not.toBeInTheDocument();
  });
});

describe("goals", () => {
  it("adds, changes, and removes goals on the home screen", async () => {
    const fake = await startApp();
    const card = screen.getByRole("region", { name: "Goals" });
    expect(within(card).getByText(/Add 3 to 5 goals/)).toBeInTheDocument();
    await userEvent.click(
      within(card).getByRole("button", { name: "Add goals" }),
    );
    await userEvent.type(
      within(card).getByRole("textbox", { name: "New goal" }),
      "Ship 2.0{Enter}",
    );
    await userEvent.type(
      within(card).getByRole("textbox", { name: "New goal" }),
      "Hire a designer{Enter}",
    );
    const first = await within(card).findByRole("textbox", {
      name: "Goal: Ship 2.0",
    });
    await userEvent.clear(first);
    await userEvent.type(first, "Ship 2.0 by 30 Nov{Enter}");
    await waitFor(() =>
      expect(fake.goals.goals[0].text).toBe("Ship 2.0 by 30 Nov"),
    );
    // An empty box does not remove the goal, so the box shows it again.
    const box = await within(card).findByRole("textbox", {
      name: "Goal: Ship 2.0 by 30 Nov",
    });
    await userEvent.clear(box);
    await userEvent.tab();
    expect(box).toHaveValue("Ship 2.0 by 30 Nov");
    expect(fake.goals.goals[0].text).toBe("Ship 2.0 by 30 Nov");
    await userEvent.click(
      within(card).getByRole("button", {
        name: "Remove goal: Hire a designer",
      }),
    );
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
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Goal: Ship 2.0" }),
      " now",
    );
    // The click on ✕ ends the edit first (blur). The edit's answer is slow.
    await userEvent.click(
      within(card).getByRole("button", { name: "Remove goal: Ship 2.0" }),
    );
    slowEdit.resolve();
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "remove_goal")).toBe(true),
    );
    await waitFor(() =>
      expect(
        within(card).queryByRole("textbox", { name: /^Goal:/ }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.goals.goals).toEqual([]);
  });

  it("stops at 5 goals", async () => {
    const goals = [1, 2, 3, 4, 5].map((id) => ({
      id: 900 + id,
      text: `Goal ${id}`,
      created: 1,
      updated: 1,
    }));
    await startApp({ goals: { goals, checked: 1, stale: false } });
    await userEvent.click(
      within(screen.getByRole("region", { name: "Goals" })).getByRole(
        "button",
        { name: "Edit" },
      ),
    );
    expect(
      screen.queryByRole("textbox", { name: "New goal" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/Remove one to add another/)).toBeInTheDocument();
  });

  it("asks to check old goals, and Still right stops the question", async () => {
    const old = Math.floor(Date.now() / 1000) - 40 * 86400;
    const fake = await startApp({
      goals: {
        goals: [{ id: 950, text: "Ship 2.0", created: old, updated: old }],
        checked: old,
        stale: true,
      },
    });
    expect(
      screen.getByText(/You set these goals 40 days ago/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Still right" }));
    await waitFor(() =>
      expect(screen.queryByText(/You set these goals/)).not.toBeInTheDocument(),
    );
    expect(fake.calls.some((c) => c.cmd === "confirm_goals")).toBe(true);
  });

  it("shows the goal of each item that the brief ranked, or No goal", async () => {
    await startApp({
      goals: {
        goals: [{ id: 960, text: "Ship 2.0", created: 1, updated: 1 }],
        checked: Math.floor(Date.now() / 1000),
        stale: false,
      },
      github: [
        githubItem(9, "review_requested", "Bulk edit"),
        githubItem(10, "review_requested", "Fix typo"),
      ],
      newKeys: ["github:acme/shop#9", "github:acme/shop#10"],
      briefs: {
        [today()]: {
          day: today(),
          written: 1,
          text: "Two reviews.",
          why: {
            "github:acme/shop#9": "It blocks the release.",
            "github:acme/shop#10": "It waits 3 days.",
          },
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

describe("suggested work", () => {
  it("shows the next step from the brief, and who does it", async () => {
    await startApp({
      github: [
        githubItem(9, "review_requested", "Bulk edit"),
        githubItem(10, "review_requested", "Price list"),
      ],
      newKeys: ["github:acme/shop#9", "github:acme/shop#10"],
      briefs: {
        [today()]: {
          day: today(),
          written: 1,
          text: "Two things.",
          why: {
            "github:acme/shop#9": "It blocks the release.",
            "github:acme/shop#10": "Ana asks for it.",
          },
          next: {
            "github:acme/shop#9": "Review the bulk edit and approve it.",
            "github:acme/shop#10": "Reply to Ana with the price.",
          },
          draft: { "github:acme/shop#10": "email" },
        },
      },
    });
    const list = await screen.findByRole("list", { name: "New" });
    const bulk = within(list).getByText("Bulk edit").closest("li")!;
    expect(
      within(bulk).getByText("Review the bulk edit and approve it."),
    ).toBeInTheDocument();
    expect(within(bulk).getByText("You do this")).toBeInTheDocument();
    const price = within(list).getByText("Price list").closest("li")!;
    expect(
      within(price).getByText("AIfred can draft the email later"),
    ).toBeInTheDocument();
    await openItem("Price list");
    expect(
      within(details()).getByText("Reply to Ana with the price."),
    ).toBeInTheDocument();
  });

  it("adds the picked steps to the checklist of a task, with an undo", async () => {
    const fake = await startApp({
      tasks: [
        task("Plan the trip", {
          id: "t1",
          checklist: [{ text: "Pick dates", done: true }],
        }),
      ],
      onSuggestSteps: () => [
        { text: "Email Ana about the hotel", draft: "email" },
        { text: "Book the train" },
        { text: "Pack" },
      ],
    });
    await openItem("Plan the trip");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest steps" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Suggested steps",
    });
    expect(
      within(box).getByText("AIfred can draft the email later"),
    ).toBeInTheDocument();
    expect(
      within(box).getByText(/the checklist of this task/),
    ).toBeInTheDocument();
    await userEvent.click(
      within(box).getByRole("checkbox", { name: "Keep step: Pack" }),
    );
    await userEvent.click(
      within(box).getByRole("button", { name: "Add 2 steps" }),
    );
    expect(
      await screen.findByText('Added 2 steps to "Plan the trip"'),
    ).toBeInTheDocument();
    expect(fake.tasks[0].checklist.map((c) => c.text)).toEqual([
      "Pick dates",
      "Email Ana about the hotel",
      "Book the train",
    ]);
    expect(
      within(details()).queryByRole("group", { name: "Suggested steps" }),
    ).not.toBeInTheDocument();
    // A step that you tick before the Undo stays.
    fake.tasks[0].checklist[2].done = true;
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.tasks[0].checklist).toEqual([
        { text: "Pick dates", done: true },
        { text: "Book the train", done: true },
      ]),
    );
  });

  it("puts steps for a ClickUp item on a new task under it", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("86a", "Checkout totals")],
      onSuggestSteps: () => [
        { text: "Read the tax log" },
        { text: "Fix the rounding" },
      ],
    });
    await openItem("Checkout totals");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest steps" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Suggested steps",
    });
    expect(
      within(box).getByText(/a new task of yours, under this item/),
    ).toBeInTheDocument();
    // A second click while the save runs does not save again.
    const slow = deferred<void>();
    fake.onAddSteps = () => slow.promise;
    const add = within(box).getByRole("button", { name: "Add 2 steps" });
    await userEvent.click(add);
    expect(add).toBeDisabled();
    await userEvent.click(add);
    slow.resolve();
    expect(
      await screen.findByText(
        'Made the task "Checkout totals" with 2 steps, under this item',
      ),
    ).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "add_steps")).toHaveLength(1);
    expect(fake.tasks).toHaveLength(1);
    expect(fake.links).toMatchObject([
      { key: `task:${fake.tasks[0].id}`, main: "clickup:86a", by: "you" },
    ]);
    const open = screen.getByRole("list", { name: "Open" });
    await waitFor(() =>
      expect(within(open).getByText("0/2 steps")).toBeInTheDocument(),
    );
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks).toEqual([]));
    await waitFor(() => expect(fake.links).toEqual([]));
  });

  it("names the open task of the same work as the place of the steps", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("86a", "Checkout totals")],
      tasks: [task("Fix totals", { id: "t1" })],
      links: [{ key: "task:t1", main: "clickup:86a" }],
      onSuggestSteps: () => [{ text: "Read the tax log" }],
    });
    await openItem("Checkout totals");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest steps" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Suggested steps",
    });
    expect(
      within(box).getByText(/the checklist of your task "Fix totals"/),
    ).toBeInTheDocument();
    await userEvent.click(
      within(box).getByRole("button", { name: "Add 1 step" }),
    );
    expect(
      await screen.findByText('Added 1 step to "Fix totals"'),
    ).toBeInTheDocument();
    expect(fake.tasks[0].checklist).toEqual([
      { text: "Read the tax log", done: false },
    ]);
  });

  it("does not put the steps on a task that AIfred placed under the item", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("86a", "Checkout totals")],
      tasks: [task("Email Ana about refund", { id: "t1" })],
      links: [
        {
          key: "task:t1",
          main: "clickup:86a",
          by: "aifred",
          why: "Same customer.",
        },
      ],
      onSuggestSteps: () => [{ text: "Read the tax log" }],
    });
    await openItem("Checkout totals");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest steps" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Suggested steps",
    });
    expect(
      within(box).getByText(/a new task of yours, under this item/),
    ).toBeInTheDocument();
    await userEvent.click(
      within(box).getByRole("button", { name: "Add 1 step" }),
    );
    await waitFor(() => expect(fake.tasks).toHaveLength(2));
    expect(fake.tasks.find((t) => t.id === "t1")?.checklist).toEqual([]);
  });

  it("shows an error when AIfred cannot suggest steps", async () => {
    await startApp({
      tasks: [task("Plan the trip")],
      onSuggestSteps: () => {
        throw "AIfred found no steps for this item";
      },
    });
    await openItem("Plan the trip");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest steps" }),
    );
    expect(
      await within(details()).findByText(
        "Cannot suggest steps: AIfred found no steps for this item",
      ),
    ).toBeInTheDocument();
  });
});

describe("email replies", () => {
  const now = () => Math.floor(Date.now() / 1000);
  const drafts = {
    client_id: "1-a.apps.googleusercontent.com",
    secret_set: true,
    connected: true,
    email: "me@x.com",
    features: [
      ["Calendar", true],
      ["Gmail", true],
      ["Gmail drafts", true],
    ] as [string, boolean][],
    optional: ["Gmail", "Gmail drafts"],
    off: [],
  };

  it("writes a reply that you change, saves it as a Gmail draft, and logs it with an undo", async () => {
    const fake = await startApp({
      google: drafts,
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
      onWriteReply: (_k, note) => `Hi Ana, ${note}`,
    });
    await openItem("Price for the plugin");
    await userEvent.type(
      within(details()).getByRole("textbox", { name: "What to say" }),
      "the price is 500 EUR",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    expect(
      within(box).getByText(
        /To: Ana Lee <ana@y.com> · Re: Price for the plugin/,
      ),
    ).toBeInTheDocument();
    const text = within(box).getByRole("textbox", { name: "Reply text" });
    expect(text).toHaveValue("Hi Ana, the price is 500 EUR");
    await userEvent.type(text, ". Bo");
    // Nothing is in Gmail before the click.
    expect(fake.actions).toEqual([]);
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    );
    expect(
      await screen.findByText(
        "Saved the reply as a draft in Gmail. It is not sent.",
      ),
    ).toBeInTheDocument();
    expect(fake.actions.map((a) => a.after)).toEqual([
      "Hi Ana, the price is 500 EUR. Bo",
    ]);
    const log = await within(details()).findByRole("list", {
      name: "AIfred's actions on this item",
    });
    expect(
      within(log).getByText(
        "Reply draft to ana@y.com: Re: Price for the plugin",
      ),
    ).toBeInTheDocument();
    await userEvent.click(within(log).getByRole("button", { name: "Undo" }));
    expect(await within(log).findByText(/^Undone/)).toBeInTheDocument();
    expect(fake.actions[0].undone_at).not.toBeNull();
    // The settings page has the same log.
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(await screen.findByRole("tab", { name: "Activity" }));
    const all = await screen.findByRole("region", { name: "AIfred's actions" });
    expect(
      within(all).getByText(
        "Reply draft to ana@y.com: Re: Price for the plugin",
      ),
    ).toBeInTheDocument();
  });

  it("a second Undo click while the first one runs does not start a second Undo", async () => {
    let finish: () => void = () => {};
    const fake = await startApp({
      google: drafts,
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    );
    const log = await within(details()).findByRole("list", {
      name: "AIfred's actions on this item",
    });
    fake.beforeUndo = () => new Promise<void>((r) => (finish = r));
    await userEvent.click(within(log).getByRole("button", { name: "Undo" }));
    // The list button is off while the Undo runs. The Undo of the toast joins the same Undo.
    expect(
      within(log).getByRole("button", { name: "Undoing…" }),
    ).toBeDisabled();
    const toast = screen
      .getByText("Saved the reply as a draft in Gmail. It is not sent.")
      .closest("div")!;
    await userEvent.click(within(toast).getByRole("button", { name: "Undo" }));
    finish();
    expect(await within(log).findByText(/^Undone/)).toBeInTheDocument();
    expect(fake.undoCalls).toBe(1);
    expect(screen.queryByText(/Cannot undo/)).not.toBeInTheDocument();
  });

  it("the reply label names the person that the reply goes to", async () => {
    await startApp({
      google: drafts,
      // Ana wrote, but her email asks for replies to Sales (Reply-To).
      mail: {
        fetched_at: now(),
        reply: [
          mail("r1", "Price for the plugin", "reply", { reply_to: "Sales" }),
        ],
        sent: [],
      },
      briefs: {
        [today()]: {
          day: today(),
          written: 1,
          text: "One thing.",
          why: {},
          next: { "gmail:r1": "Send Ana the price." },
          draft: { "gmail:r1": "email" },
        },
      },
    });
    await openItem("Price for the plugin");
    expect(
      within(details()).getByText("AIfred can write a reply to Sales"),
    ).toBeInTheDocument();
  });

  it("Undo says that it deleted nothing when you sent the draft already", async () => {
    const fake = await startApp({
      google: drafts,
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    );
    const log = await within(details()).findByRole("list", {
      name: "AIfred's actions on this item",
    });
    fake.draftGone = true;
    await userEvent.click(within(log).getByRole("button", { name: "Undo" }));
    expect(
      await screen.findByText(
        /^AIfred deleted nothing: .*You sent it or deleted it\.$/,
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("Deleted the draft in Gmail."),
    ).not.toBeInTheDocument();
  });

  it("a save that ends after you started a new reply does not remove the new reply", async () => {
    let finish: () => void = () => {};
    const fake = await startApp({
      google: drafts,
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    let box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    fake.beforeSave = () => new Promise<void>((r) => (finish = r));
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    );
    // While the save waits: Cancel, and write a new reply.
    await userEvent.click(within(box).getByRole("button", { name: "Cancel" }));
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    box = await within(details()).findByRole("group", { name: "Reply draft" });
    finish();
    expect(
      await screen.findByText(
        "Saved the reply as a draft in Gmail. It is not sent.",
      ),
    ).toBeInTheDocument();
    expect(
      within(details()).getByRole("group", { name: "Reply draft" }),
    ).toBeInTheDocument();
  });

  it("you cannot change the reply text while it saves, so no change is lost", async () => {
    let finish: () => void = () => {};
    const fake = await startApp({
      google: drafts,
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    fake.beforeSave = () => new Promise<void>((r) => (finish = r));
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    );
    const text = within(box).getByRole("textbox", { name: "Reply text" });
    expect(text).toHaveAttribute("readonly");
    await userEvent.type(text, " More.");
    expect(text).toHaveValue("Hi Ana, thanks.");
    finish();
    expect(
      await screen.findByText(
        "Saved the reply as a draft in Gmail. It is not sent.",
      ),
    ).toBeInTheDocument();
  });

  it("cannot save a draft before you turn on Gmail drafts", async () => {
    const fake = await startApp({
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    expect(
      within(box).getByText(/tick "Use Gmail drafts" in Settings/),
    ).toBeInTheDocument();
    expect(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    ).toBeDisabled();
    await userEvent.click(within(box).getByRole("button", { name: "Cancel" }));
    expect(
      within(details()).queryByRole("group", { name: "Reply draft" }),
    ).not.toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "save_reply_draft")).toBe(false);
  });

  it("shows why a save failed, and keeps your text", async () => {
    const fake = await startApp({
      google: drafts,
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Write a reply" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Reply draft",
    });
    // A new message came in the thread after AIfred wrote the reply.
    fake.mail!.reply[0].email = "bo@y.com";
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as Gmail draft" }),
    );
    expect(
      await within(box).findByText(/Cannot save: .*the email changed/),
    ).toBeInTheDocument();
    expect(
      within(box).getByRole("textbox", { name: "Reply text" }),
    ).toHaveValue("Hi Ana, thanks.");
    expect(fake.actions).toEqual([]);
  });

  it("offers no reply on an item that is not an email", async () => {
    await startApp({ tasks: [task("Plan the trip")] });
    await openItem("Plan the trip");
    expect(
      within(details()).queryByRole("button", { name: "✦ Write a reply" }),
    ).not.toBeInTheDocument();
  });
});

describe("corrections of the brief", () => {
  /** Three GitHub items from acme/shop, each with a reason from today's brief. */
  const ranked = () => ({
    github: [
      githubItem(9, "review_requested", "Bulk edit"),
      githubItem(10, "review_requested", "Fix typo"),
      githubItem(11, "review_requested", "New FAQ"),
    ],
    newKeys: [
      "github:acme/shop#9",
      "github:acme/shop#10",
      "github:acme/shop#11",
    ],
    briefs: {
      [today()]: {
        day: today(),
        written: 1,
        text: "Three reviews.",
        why: {
          "github:acme/shop#9": "It blocks the release.",
          "github:acme/shop#10": "It waits 3 days.",
          "github:acme/shop#11": "It is new.",
        },
      },
    },
  });

  it("Not today removes the reason of the item, with an undo", async () => {
    const fake = await startApp(ranked());
    expect(
      await screen.findByText("It blocks the release."),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Not today: Bulk edit" }),
    );
    expect(
      await screen.findByText(
        "AIfred will not count this for today: Bulk edit",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByText("It blocks the release."),
    ).not.toBeInTheDocument();
    expect(fake.corrections).toMatchObject([
      { key: "github:acme/shop#9", kind: "not_today", day: today() },
    ]);
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(
      await screen.findByText("It blocks the release."),
    ).toBeInTheDocument();
    expect(fake.corrections).toEqual([]);
  });

  it("Not today on a tree row corrects the subtask whose reason the row shows", async () => {
    const r = ranked();
    // Only the subtask "Fix typo" has a reason. Its row is "Bulk edit".
    const b = r.briefs[today()];
    (b as { why: Record<string, string> }).why = {
      "github:acme/shop#10": "It waits 3 days.",
    };
    const fake = await startApp({
      ...r,
      links: [
        {
          key: "github:acme/shop#10",
          main: "github:acme/shop#9",
          by: "aifred",
          why: "Same PR.",
        },
      ],
    });
    expect(await screen.findByText("It waits 3 days.")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Not today: Fix typo" }),
    );
    expect(
      await screen.findByText("AIfred will not count this for today: Fix typo"),
    ).toBeInTheDocument();
    expect(fake.corrections).toMatchObject([
      { key: "github:acme/shop#10", kind: "not_today" },
    ]);
    expect(screen.queryByText("It waits 3 days.")).not.toBeInTheDocument();
  });

  it("three Wrong priority clicks from one place teach AIfred a memory fact", async () => {
    const fake = await startApp(ranked());
    for (const t of ["Bulk edit", "Fix typo"]) {
      await userEvent.click(
        await screen.findByRole("button", { name: `Wrong priority: ${t}` }),
      );
      expect(
        await screen.findByText(
          `AIfred will rank this lower from now on: ${t}`,
        ),
      ).toBeInTheDocument();
    }
    await userEvent.click(
      screen.getByRole("button", { name: "Wrong priority: New FAQ" }),
    );
    expect(
      await screen.findByText(
        /AIfred learned: GitHub acme\/shop: these items are less important/,
      ),
    ).toBeInTheDocument();
    expect(fake.memory.filter((f) => !f.forgotten)).toHaveLength(1);
    // The undo removes the last correction and forgets the fact.
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.memory.filter((f) => !f.forgotten)).toHaveLength(0),
    );
    expect(await screen.findByText("It is new.")).toBeInTheDocument();
  });
});

describe("missions on the home page", () => {
  /** Two tasks and a GitHub item in one mission, and a task in no mission. */
  const setup = () => {
    const a = task("Write the post", { id: "ta" });
    const b = task("Send the post", { id: "tb" });
    const other = task("Pay rent", { id: "tc" });
    const gh = githubItem(7, "assigned", "Fix the post page");
    const m: Mission = {
      id: 5,
      title: "Ship the launch post",
      doer: "you",
      day: today(),
      rank: 1,
      status: "open",
      items: [
        { key: "task:ta", title: "Write the post", open: true, by: "aifred" },
        { key: "task:tb", title: "Send the post", open: true, by: "aifred" },
        {
          key: "github:acme/shop#7",
          title: "Fix the post page",
          open: true,
          by: "aifred",
        },
      ],
      checklist: [
        { text: "Draft the post", done: false },
        { text: "Ask Ana to check it", done: false },
      ],
      progress: {
        steps_done: 0,
        steps: 2,
        items_closed: 0,
        items: 3,
        percent: 0,
      },
    };
    return {
      tasks: [a, b, other],
      github: [gh],
      briefs: {
        [today()]: {
          day: today(),
          written: 1,
          text: "One mission.",
          why: { "task:ta": "The launch is on Friday." },
          next: { "task:ta": "Write the first draft." },
          missions: [m],
        },
      },
    };
  };

  it("shows the missions, and the other items behind one line", async () => {
    await startApp(setup());
    const list = await screen.findByRole("list", { name: "Today's missions" });
    expect(within(list).getByText("Ship the launch post")).toBeInTheDocument();
    expect(
      within(list).getByText("The launch is on Friday."),
    ).toBeInTheDocument();
    expect(
      within(list).getByText("Write the first draft."),
    ).toBeInTheDocument();
    // The items of the mission are not rows of their own. The other item is behind the line.
    expect(
      screen.queryByRole("button", { name: "Open details: Pay rent" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Show other open items (1)" }),
    );
    expect(
      await screen.findByRole("button", { name: "Open details: Pay rent" }),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Open details: Send the post" }),
    ).not.toBeInTheDocument();
  });

  it("a tree with one subtask in a mission still shows its other open subtasks", async () => {
    const s = setup();
    s.tasks.push(task("Plan the launch", { id: "tp" }));
    // "Write the post" is a mission. "Pay rent" is its sister subtask in no mission.
    await startApp({
      ...s,
      links: [
        { key: "task:ta", main: "task:tp", by: "aifred", why: "Launch work." },
        { key: "task:tc", main: "task:tp", by: "aifred", why: "Launch work." },
      ],
    });
    await screen.findByRole("list", { name: "Today's missions" });
    // The mission card names its parent ("Part of"), so count the buttons.
    const parents = () =>
      screen.queryAllByRole("button", { name: "Open details: Plan the launch" })
        .length;
    const before = parents();
    expect(
      screen.queryByRole("button", { name: "Open details: Pay rent" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Show other open items (1)" }),
    );
    await waitFor(() => expect(parents()).toBe(before + 1));
    expect(
      screen.getByRole("button", { name: "Open details: Pay rent" }),
    ).toBeInTheDocument();
    // The subtask in the mission does not show twice.
    expect(
      screen.queryByRole("button", { name: "Open details: Write the post" }),
    ).not.toBeInTheDocument();
  });

  it("when the row item is a mission, its open sister subtask is the row", async () => {
    const s = setup();
    // "Write the post" (a mission) is the parent of "Pay rent" (no mission).
    await startApp({
      ...s,
      links: [
        { key: "task:tc", main: "task:ta", by: "aifred", why: "Launch work." },
      ],
    });
    await screen.findByRole("list", { name: "Today's missions" });
    const posts = () =>
      screen.queryAllByRole("button", { name: "Open details: Write the post" })
        .length;
    const before = posts();
    await userEvent.click(
      screen.getByRole("button", { name: "Show other open items (1)" }),
    );
    expect(
      await screen.findByRole("button", { name: "Open details: Pay rent" }),
    ).toBeInTheDocument();
    expect(posts()).toBe(before);
  });

  it("an urgent item in no mission does not hide behind the line", async () => {
    const s = setup();
    s.tasks.push(task("Renew the domain", { id: "td", priority: "urgent" }));
    await startApp(s);
    await screen.findByRole("list", { name: "Today's missions" });
    expect(screen.getByText("Needs you now")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Open details: Renew the domain" }),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Show other open items (1)" }),
    ).toBeInTheDocument();
  });

  it("an urgent subtask in no mission shows when the top item of its tree is a mission", async () => {
    const s = setup();
    s.tasks.push(task("Renew the domain", { id: "td", priority: "urgent" }));
    // "Write the post" (a mission) is the parent of the urgent "Renew the domain" (no mission).
    await startApp({
      ...s,
      links: [
        { key: "task:td", main: "task:ta", by: "aifred", why: "Launch work." },
      ],
    });
    const now = await screen.findByRole("list", { name: "Needs you now" });
    expect(
      within(now).getByRole("button", {
        name: "Open details: Renew the domain",
      }),
    ).toBeInTheDocument();
  });

  it("a tree row does not keep the reason of its subtask in a mission", async () => {
    const s = setup();
    s.tasks.push(task("Plan the launch", { id: "tp", priority: "urgent" }));
    // "Write the post" (a mission, with the brief's reason) is under the urgent
    // "Plan the launch" (no mission). "Needs you now" shows the reasons of its rows.
    await startApp({
      ...s,
      links: [
        { key: "task:ta", main: "task:tp", by: "aifred", why: "Launch work." },
        { key: "task:tc", main: "task:tp", by: "aifred", why: "Launch work." },
      ],
    });
    const now = await screen.findByRole("list", { name: "Needs you now" });
    expect(
      within(now).getByRole("button", {
        name: "Open details: Plan the launch",
      }),
    ).toBeInTheDocument();
    // Only the mission card shows the reason and the next action.
    expect(screen.getAllByText("The launch is on Friday.")).toHaveLength(1);
    expect(screen.getAllByText("Write the first draft.")).toHaveLength(1);
  });

  it("when the first item closed, the next open item gives the reason", async () => {
    const s = setup();
    const b = s.briefs[today()];
    b.missions[0].items[0].open = false;
    s.tasks[0].done = true;
    (b.why as Record<string, string>)["task:tb"] = "Ana waits for it.";
    await startApp(s);
    const list = await screen.findByRole("list", { name: "Today's missions" });
    expect(within(list).getByText("Ana waits for it.")).toBeInTheDocument();
    expect(
      within(list).queryByText("Write the first draft."),
    ).not.toBeInTheDocument();
  });

  it("shows the parent of the first item as the context of the mission", async () => {
    const s = setup();
    await startApp({
      ...s,
      links: [{ key: "task:ta", main: "task:tc", by: "aifred", why: "" }],
    });
    const list = await screen.findByRole("list", { name: "Today's missions" });
    expect(within(list).getByText(/Part of/)).toBeInTheDocument();
    await userEvent.click(
      within(list).getByRole("button", { name: "Open details: Pay rent" }),
    );
    expect(
      await within(details()).findByDisplayValue("Pay rent"),
    ).toBeInTheDocument();
  });

  it("takes an item out of a mission, with an undo", async () => {
    const fake = await startApp(setup());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const panel = screen.getByRole("group", {
      name: "Mission: Ship the launch post",
    });
    await userEvent.click(
      within(panel).getByRole("button", {
        name: "Take out of the mission: Send the post",
      }),
    );
    expect(
      await screen.findByText(
        /Taken out of Ship the launch post: Send the post\. AIfred does not put it back\./,
      ),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(
        within(panel).queryByRole("button", {
          name: "Open details: Send the post",
        }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.missionOut).toEqual(["5:task:tb"]);
    // The undo puts it back in its place.
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(
        within(panel).getByRole("button", {
          name: "Open details: Send the post",
        }),
      ).toBeInTheDocument(),
    );
    expect(fake.missionOut).toEqual([]);
    expect(fake.briefs[today()]!.missions![0]!.items.map((i) => i.key)).toEqual(
      ["task:ta", "task:tb", "github:acme/shop#7"],
    );
  });

  it("the last open item of a mission cannot go out", async () => {
    const s = setup();
    const m = s.briefs[today()].missions[0];
    m.items = [m.items[0]!, { ...m.items[1]!, open: false }];
    s.tasks[1].done = true;
    await startApp(s);
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const panel = screen.getByRole("group", {
      name: "Mission: Ship the launch post",
    });
    expect(
      within(panel).queryByRole("button", { name: /Take out of the mission/ }),
    ).not.toBeInTheDocument();
  });

  it("adds an item to a mission, with an undo", async () => {
    const fake = await startApp(setup());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const panel = screen.getByRole("group", {
      name: "Mission: Ship the launch post",
    });
    await userEvent.selectOptions(
      within(panel).getByRole("combobox", {
        name: "Add an item to the mission: Ship the launch post",
      }),
      "task:tc",
    );
    expect(
      await screen.findByText(
        /Added to Ship the launch post: Pay rent\. AIfred keeps it there\./,
      ),
    ).toBeInTheDocument();
    expect(await within(panel).findByText("You added it")).toBeInTheDocument();
    expect(fake.briefs[today()]!.missions![0]!.items.at(-1)).toMatchObject({
      key: "task:tc",
      by: "you",
    });
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(within(panel).queryByText("You added it")).not.toBeInTheDocument(),
    );
    expect(fake.briefs[today()]!.missions![0]!.items).toHaveLength(3);
  });

  it("a fast second tick waits for the first answer", async () => {
    const fake = await startApp(setup());
    let free = () => {};
    fake.onMissionStep = () => new Promise<void>((r) => (free = r));
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const panel = screen.getByRole("group", {
      name: "Mission: Ship the launch post",
    });
    // The second click comes before the first answer. It must not send "done" a second time.
    await userEvent.dblClick(
      within(panel).getByRole("button", { name: "Step done: Draft the post" }),
    );
    free();
    expect(await within(panel).findByText(/= 20%/)).toBeInTheDocument();
    expect(
      fake.calls.filter((c) => c.cmd === "set_mission_step"),
    ).toMatchObject([{ args: { done: true } }]);
  });

  it("a tick in the checklist changes the progress, not the items", async () => {
    const fake = await startApp(setup());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const panel = screen.getByRole("group", {
      name: "Mission: Ship the launch post",
    });
    expect(
      within(panel).getByText(
        /Progress: 0 of 2 steps done and 0 of 3 items closed/,
      ),
    ).toBeInTheDocument();
    await userEvent.click(
      within(panel).getByRole("button", { name: "Step done: Draft the post" }),
    );
    expect(await within(panel).findByText(/= 20%/)).toBeInTheDocument();
    expect(
      fake.calls.filter((c) => c.cmd === "set_mission_step"),
    ).toMatchObject([{ args: { id: 5, index: 0, done: true } }]);
    expect(fake.tasks.every((t) => !t.done)).toBe(true);
  });

  it("Done asks which items to close, and GitHub items cannot close", async () => {
    const fake = await startApp(setup());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    const ask = screen.getByRole("group", {
      name: "Finish mission: Ship the launch post",
    });
    expect(
      within(ask).getByRole("checkbox", { name: "Fix the post page" }),
    ).toBeDisabled();
    // Keep "Send the post" open.
    await userEvent.click(
      within(ask).getByRole("checkbox", { name: "Send the post" }),
    );
    await userEvent.click(
      within(ask).getByRole("button", { name: "Finish mission" }),
    );
    await waitFor(() =>
      expect(fake.tasks.find((t) => t.id === "ta")?.done).toBe(true),
    );
    expect(fake.tasks.find((t) => t.id === "tb")?.done).toBe(false);
    expect(
      screen.queryByRole("list", { name: "Today's missions" }),
    ).not.toBeInTheDocument();
    // With no mission left, the home page shows the list in groups again.
    expect(
      await screen.findByRole("button", {
        name: "Open details: Send the post",
      }),
    ).toBeInTheDocument();
    // The one undo opens the mission and its closed task again.
    expect(
      screen.getByText(
        "Mission done: Ship the launch post. Undo opens it and its closed item again.",
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(
      await screen.findByRole("list", { name: "Today's missions" }),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(fake.tasks.find((t) => t.id === "ta")?.done).toBe(false),
    );
  });

  it("a ClickUp item with a running timer does not hide the Undo of the mission", async () => {
    const s = setup();
    s.briefs[today()].missions[0].items.push({
      key: "clickup:86a",
      title: "Checkout totals",
      open: true,
      by: "aifred",
    });
    const fake = await startApp({
      ...s,
      clickupToken: "pk_test",
      clickup: [clickupItem("86a", "Checkout totals")],
      sessions: [
        {
          key: "clickup:86a",
          started: Math.floor(Date.now() / 1000) - 600,
          ended: null,
        },
      ],
    });
    // The work ends after the mission is done, as it can in the app.
    let endWork = () => {};
    fake.beforeFinishWork = () => new Promise<void>((r) => (endWork = r));
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const panel = screen.getByRole("group", {
      name: "Mission: Ship the launch post",
    });
    await userEvent.click(within(panel).getByRole("button", { name: "Done" }));
    const ask = screen.getByRole("group", {
      name: "Finish mission: Ship the launch post",
    });
    await userEvent.click(
      within(ask).getByRole("button", { name: "Finish mission" }),
    );
    expect(
      await screen.findByText(/^Mission done: Ship the launch post\./),
    ).toBeInTheDocument();
    expect(
      fake.calls.some(
        (c) => c.cmd === "finish_work" && c.args.key === "clickup:86a",
      ),
    ).toBe(true);
    endWork();
    await waitFor(() => expect(fake.workDone["clickup:86a"]).toBeDefined());
    // Let the app handle the answer.
    await act(() => new Promise((r) => setTimeout(r, 20)));
    // The work ends with no toast of its own: the toast of the mission stays, with its Undo.
    expect(
      screen.getByText(/^Mission done: Ship the launch post\./),
    ).toBeInTheDocument();
    expect(screen.queryByText(/^Finished:/)).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Undo" })).toBeInTheDocument();
  });

  it("when an item does not close, the mission stays open", async () => {
    const fake = await startApp(setup());
    fake.fail.set_task_done = "the disk is full";
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    await userEvent.click(screen.getByRole("button", { name: "Done" }));
    await userEvent.click(
      screen.getByRole("button", { name: "Finish mission" }),
    );
    // The reason of the failed close stays in the text.
    expect(
      await screen.findByText(
        /^The mission stays open: AIfred could not close Write the post\. .*the disk is full/,
      ),
    ).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "finish_mission")).toEqual([]);
    expect(
      screen.getByRole("list", { name: "Today's missions" }),
    ).toBeInTheDocument();
  });

  it("a mission opens on the right side, with a chat about the mission", async () => {
    const s = setup();
    const fake = await startApp({
      ...s,
      onAskSide: (p) => answer(`About the mission: ${p}`),
    });
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const side = screen.getByRole("dialog", { name: "Mission details" });
    // The items and the checklist are in the side panel, not in the list.
    expect(
      within(side).getByRole("group", {
        name: "Mission: Ship the launch post",
      }),
    ).toBeInTheDocument();
    expect(
      within(
        screen.getByRole("list", { name: "Today's missions" }),
      ).queryByRole("group"),
    ).not.toBeInTheDocument();
    expect(
      fake.calls.some(
        (c) => c.cmd === "side_history" && c.args.thread === "mission:5",
      ),
    ).toBe(true);
    await userEvent.type(
      within(side).getByRole("textbox", {
        name: "Ask AIfred about this mission",
      }),
      "what is left?{Enter}",
    );
    expect(
      await within(side).findByText("About the mission: what is left?"),
    ).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask_side")!.args).toMatchObject({
      thread: "mission:5",
    });
    expect(fake.history).toEqual([]);
    // An item of the mission opens in its place. The mission card opens the mission again, with its chat.
    await userEvent.click(
      within(side).getByRole("button", {
        name: "Open details: Write the post",
      }),
    );
    expect(
      screen.queryByRole("dialog", { name: "Mission details" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("dialog", { name: "Task details" }),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    expect(
      within(screen.getByRole("dialog", { name: "Mission details" })).getByText(
        "About the mission: what is left?",
      ),
    ).toBeInTheDocument();
    // A second click on the card closes the panel.
    await userEvent.click(
      screen.getByRole("button", {
        name: "Close mission: Ship the launch post",
      }),
    );
    expect(
      screen.queryByRole("dialog", { name: "Mission details" }),
    ).not.toBeInTheDocument();
  });

  it("when the AI in the mission chat finishes the mission, the panel and the answer stay", async () => {
    const s = setup();
    let fake: Awaited<ReturnType<typeof startApp>> | undefined;
    fake = await startApp({
      ...s,
      onAskSide: (p) => {
        // Like finish_mission in the tool server: the mission leaves today's list.
        fake!.briefs[today()]!.missions = [];
        return answer(`Finished. Ask me to undo it. (${p})`);
      },
    });
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const side = screen.getByRole("dialog", { name: "Mission details" });
    await userEvent.type(
      within(side).getByRole("textbox", {
        name: "Ask AIfred about this mission",
      }),
      "finish it{Enter}",
    );
    expect(
      await within(side).findByText("Finished. Ask me to undo it. (finish it)"),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(
        screen.queryByRole("list", { name: "Today's missions" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.getByRole("dialog", { name: "Mission details" }),
    ).toBeInTheDocument();
  });

  it("a finished mission closes its side panel", async () => {
    await startApp(setup());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    const side = screen.getByRole("dialog", { name: "Mission details" });
    await userEvent.click(within(side).getByRole("button", { name: "Done" }));
    await userEvent.click(
      within(side).getByRole("button", { name: "Finish mission" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Mission details" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("Not today hides the mission for today, with an undo", async () => {
    const fake = await startApp(setup());
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Open mission: Ship the launch post",
      }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Not today: Ship the launch post" }),
    );
    expect(
      await screen.findByText(
        "AIfred will not count this for today: Ship the launch post",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText("Ship the launch post")).not.toBeInTheDocument();
    expect(fake.corrections.map((c) => c.key).sort()).toEqual([
      "github:acme/shop#7",
      "task:ta",
      "task:tb",
    ]);
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("Ship the launch post")).toBeInTheDocument();
    expect(fake.corrections).toEqual([]);
  });
});

describe("work time", () => {
  it("Start shows the timer, and Done says the real time next to the guess", async () => {
    let clock = 1_000_000;
    const fake = await startApp({
      tasks: [task("Pay rent")],
      estimates: {},
      now: () => clock,
    });
    const key = `task:${fake.tasks[0].id}`;
    fake.estimates[key] = { minutes: 30, by: "you" };
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    const bar = await screen.findByRole("region", { name: "Working on" });
    expect(within(bar).getByText("Pay rent")).toBeInTheDocument();
    clock += 45 * 60;
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(
      await screen.findByText(
        "Finished: Pay rent. It took 45 m, the guess was 30 m.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Working on" }),
    ).not.toBeInTheDocument();
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
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
      sessions: [
        {
          key: "github:acme/shop#7",
          started: Math.floor(Date.now() / 1000) - 600,
          ended: null,
        },
      ],
    });
    const bar = await screen.findByRole("region", { name: "Working on" });
    expect(within(bar).getByText(/^10 m/)).toBeInTheDocument();
    // The item has no time guess. The default time is not a guess, so the bar shows none.
    expect(within(bar).queryByText(/guessed/)).not.toBeInTheDocument();
    await userEvent.click(within(bar).getByRole("button", { name: "Stop" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Working on" }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.sessions[0].ended).not.toBeNull();
    expect(screen.getByText("Fix cart")).toBeInTheDocument();
  });

  it("Done on a GitHub item keeps its time, and the item stays until it closes on GitHub", async () => {
    let clock = 1_000_000;
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
      now: () => clock,
    });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    clock += 20 * 60;
    await userEvent.click(
      within(
        await screen.findByRole("region", { name: "Working on" }),
      ).getByRole("button", { name: "Done" }),
    );
    expect(
      await screen.findByText("Finished: Fix cart. It took 20 m."),
    ).toBeInTheDocument();
    expect(fake.workDone["github:acme/shop#7"]).toMatchObject({
      minutes: 20,
      guess: null,
    });
    expect(screen.getByText("Fix cart")).toBeInTheDocument();
  });

  it("a failed save of the work time keeps the timer, so Done can run again", async () => {
    let clock = 1_000_000;
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
      now: () => clock,
    });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    clock += 20 * 60;
    fake.failFinishWork = "the disk is full";
    const bar = await screen.findByRole("region", { name: "Working on" });
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(
      await screen.findByText(
        "Finished: Fix cart. The work time is not saved: the disk is full",
      ),
    ).toBeInTheDocument();
    expect(
      screen.getByRole("region", { name: "Working on" }),
    ).toBeInTheDocument();
    fake.failFinishWork = undefined;
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    expect(
      await screen.findByText("Finished: Fix cart. It took 20 m."),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Working on" }),
    ).not.toBeInTheDocument();
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
    expect(
      await screen.findByText("The work time is not saved: the disk is full"),
    ).toBeInTheDocument();
    // A new list after the next answer does not save it again or show the error again.
    fake.calls.length = 0;
    await userEvent.type(homeBox(), "thanks{Enter}");
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "tasks")).toBe(true),
    );
    expect(fake.calls.filter((c) => c.cmd === "finish_work")).toEqual([]);
    expect(
      screen.getAllByText("The work time is not saved: the disk is full"),
    ).toHaveLength(1);
    expect(
      screen.getByRole("region", { name: "Working on" }),
    ).toBeInTheDocument();
  });

  it("a timer that ran for more than 10 hours does not count, and the toast says so", async () => {
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
      sessions: [
        {
          key: "github:acme/shop#7",
          started: Math.floor(Date.now() / 1000) - 11 * 3600,
          ended: null,
        },
      ],
    });
    await userEvent.click(
      within(
        await screen.findByRole("region", { name: "Working on" }),
      ).getByRole("button", { name: "Done" }),
    );
    expect(
      await screen.findByText(
        "Finished: Fix cart. One timer ran for more than 10 hours, so AIfred did not count it.",
      ),
    ).toBeInTheDocument();
    expect(fake.workDone["github:acme/shop#7"]).toBeUndefined();
  });

  it("an item that left the list keeps a Done button on its timer", async () => {
    const fake = await startApp({
      sessions: [
        {
          key: "github:acme/shop#9",
          started: Math.floor(Date.now() / 1000) - 600,
          ended: null,
        },
      ],
    });
    const bar = await screen.findByRole("region", { name: "Working on" });
    await userEvent.click(within(bar).getByRole("button", { name: "Done" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Working on" }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.sessions[0].ended).not.toBeNull();
  });

  it("a deleted task stops its timer", async () => {
    const fake = await startApp({ tasks: [task("Old idea")] });
    await userEvent.click(await screen.findByRole("button", { name: "Start" }));
    await screen.findByRole("region", { name: "Working on" });
    await openItem("Old idea");
    await userEvent.click(
      within(details()).getByRole("button", { name: "Delete" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Working on" }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.sessions).toEqual([]);
  });

  it("an email with work time: one toast has the time and an Undo for both", async () => {
    let clock = 1_000_000;
    const fake = await startApp({
      mail: {
        fetched_at: clock,
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
      now: () => clock,
    });
    const open = await screen.findByRole("list", { name: "Open" });
    await userEvent.click(within(open).getByRole("button", { name: "Start" }));
    clock += 12 * 60;
    await userEvent.click(
      within(open).getByRole("button", {
        name: "No answer needed: Price for the plugin",
      }),
    );
    expect(
      await screen.findByText(
        "Hidden: Price for the plugin. It comes back when a new message comes. It took 12 m.",
      ),
    ).toBeInTheDocument();
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
        fake.tasks.push(
          task("Pay rent", {
            due: addDays(today(), 1),
            priority: "high",
            tags: ["home"],
          }),
        );
        return answer(`Added: Pay rent (${p})`);
      },
    });
    await userEvent.type(homeBox(), "pay rent tomorrow high #home{Enter}");
    expect(
      await within(panel()!).findByText(
        "Added: Pay rent (pay rent tomorrow high #home)",
      ),
    ).toBeInTheDocument();
    expect(
      fake.calls.some((c) => c.cmd === "add_task"),
      "the window adds no task itself",
    ).toBe(false);
    // The list loads again after the answer, so the new task shows.
    expect(await screen.findByText("Pay rent")).toBeInTheDocument();
    expect(homeBox()).toHaveValue("");
  });

  it("Tab switches to Add task: the line is a task at once, with no AI call, and with an undo", async () => {
    const fake = await startApp();
    await userEvent.click(homeBox());
    expect(
      screen.getByRole("button", { name: "Ask AIfred" }),
    ).toBeInTheDocument();
    await userEvent.keyboard("{Tab}");
    expect(homeBox()).toHaveFocus();
    expect(
      screen.getByRole("button", { name: "Add task" }),
    ).toBeInTheDocument();
    // A new task uses no model: no model picker, a green box, and plain words.
    expect(within(homeForm()).queryByRole("combobox")).not.toBeInTheDocument();
    expect(homeForm()).toHaveClass("task");
    expect(homeBox()).toHaveAttribute("placeholder", "Add a task");
    await userEvent.keyboard("pay rent tomorrow high #home");
    expect(screen.getByText("Pay rent", { selector: "b" })).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    expect(await screen.findByText("Added: Pay rent")).toBeInTheDocument();
    expect(fake.tasks.map((t) => [t.title, t.due, t.priority, t.tags])).toEqual(
      [["Pay rent", addDays(today(), 1), "high", ["home"]]],
    );
    expect(fake.calls.some((c) => c.cmd === "ask")).toBe(false);
    expect(homeBox()).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.tasks).toEqual([]));
    // Tab again: back to Ask AIfred.
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Tab}");
    expect(
      screen.getByRole("button", { name: "Ask AIfred" }),
    ).toBeInTheDocument();
    expect(within(homeForm()).getByRole("combobox")).toBeInTheDocument();
    expect(homeForm()).not.toHaveClass("task");
    expect(homeBox()).toHaveAttribute("placeholder", "Ask AIfred");
  });

  it("the chip also switches the mode, the mode stays on other pages, and Shift+Tab leaves the box", async () => {
    await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Ask AIfred" }));
    expect(
      screen.getByRole("button", { name: "Add task" }),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Tasks" }));
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(
      screen.getByRole("button", { name: "Add task" }),
    ).toBeInTheDocument();
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Shift>}{Tab}{/Shift}");
    expect(homeBox()).not.toHaveFocus();
    expect(
      screen.getByRole("button", { name: "Add task" }),
      "Shift+Tab does not switch",
    ).toBeInTheDocument();
    // Esc, then Tab: the focus moves forward, and the mode stays.
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Escape}{Tab}");
    expect(homeBox()).not.toHaveFocus();
    expect(
      screen.getByRole("button", { name: "Add task" }),
      "Tab after Esc does not switch",
    ).toBeInTheDocument();
  });

  it("a task can be added while AIfred answers", async () => {
    const hold = deferred<Answer>();
    const fake = await startApp({ onAsk: () => hold.promise });
    await userEvent.type(homeBox(), "what is due?{Enter}");
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Tab}call Sam{Enter}");
    await waitFor(() =>
      expect(fake.tasks.map((t) => t.title)).toEqual(["Call Sam"]),
    );
    hold.resolve(answer("Nothing."));
  });

  it("a file on a new task stays on the task", async () => {
    const fake = await startApp({ uploadName: "receipt.pdf" });
    await userEvent.upload(
      screen.getAllByLabelText("Attach files")[0],
      new File(["%PDF"], "receipt.pdf"),
    );
    await screen.findByText("receipt.pdf");
    await userEvent.click(homeBox());
    await userEvent.keyboard("{Tab}file the receipt{Enter}");
    await waitFor(() =>
      expect(fake.tasks[0]?.files.map((f) => f.name)).toEqual(["receipt.pdf"]),
    );
  });

  it("plan my day opens the Calendar", async () => {
    const fake = await startApp();
    await userEvent.type(homeBox(), "Plan my day");
    expect(screen.getByText("Opens Calendar")).toBeInTheDocument();
    await userEvent.keyboard("{Enter}");
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "ask_side")).toBe(true),
    );
    // Back on Home, the box is empty.
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(homeBox()).toHaveValue("");
  });

  it("sends a question to AIfred and opens the conversation on the right", async () => {
    const fake = await startApp({
      tasks: [task("Call Sam")],
      onAsk: (p) => answer(`Answer to: ${p}`),
    });
    await userEvent.type(homeBox(), "what is due today?{Enter}");
    const side = panel()!;
    expect(
      await within(side).findByText("Answer to: what is due today?"),
    ).toBeInTheDocument();
    expect(within(side).getByText("what is due today?")).toBeInTheDocument();
    expect(screen.getByText("Call Sam")).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask")!.args).toMatchObject({
      prompt: "what is due today?",
      model: "default",
      files: [],
    });
  });

  it("Shift+Enter makes a new line and does not send", async () => {
    const fake = await startApp();
    await userEvent.type(homeBox(), "line one{Shift>}{Enter}{/Shift}line two");
    expect(homeBox()).toHaveValue("line one\nline two");
    expect(
      fake.calls.some((c) => c.cmd === "ask" || c.cmd === "add_task"),
    ).toBe(false);
  });

  it("sends attached files with the question", async () => {
    const fake = await startApp({ uploadName: "notes.txt" });
    await userEvent.upload(
      screen.getAllByLabelText("Attach files")[0],
      new File(["hello"], "notes.txt", { type: "text/plain" }),
    );
    expect(await screen.findByText("notes.txt")).toBeInTheDocument();
    await userEvent.type(homeBox(), "what is in this file?{Enter}");
    await within(panel()!).findByText(/You said/);
    const id = fake.uploads[0].id;
    expect(fake.calls.find((c) => c.cmd === "ask")!.args.files).toEqual([id]);
    expect(within(panel()!).getByText("notes.txt")).toBeInTheDocument();
  });

  it("a file sent with a to-do line goes to AIfred with the line", async () => {
    const fake = await startApp({ uploadName: "receipt.pdf" });
    await userEvent.upload(
      screen.getAllByLabelText("Attach files")[0],
      new File(["%PDF"], "receipt.pdf"),
    );
    await screen.findByText("receipt.pdf");
    await userEvent.type(homeBox(), "file the receipt{Enter}");
    await within(panel()!).findByText(/You said/);
    expect(fake.calls.find((c) => c.cmd === "ask")!.args).toMatchObject({
      prompt: "file the receipt",
      files: [fake.uploads[0].id],
    });
  });
});

describe("the conversation", () => {
  it("shows progress while AIfred works, then the answer with its facts", async () => {
    const hold = deferred<Answer>();
    await startApp({
      onAsk: (_p, progress) => {
        progress({
          kind: "tool_call",
          server: "tasks",
          tool: "overview",
          input: {},
        });
        return hold.promise;
      },
    });
    await userEvent.type(homeBox(), "what is open?{Enter}");
    expect(
      await within(panel()!).findByText("✓ Reading all your open work"),
    ).toBeInTheDocument();
    expect(
      screen
        .getAllByRole("button", { name: "Send" })
        .every((b) => (b as HTMLButtonElement).disabled),
    ).toBe(true);
    hold.resolve(
      answer("Two things.", {
        memory: ["Sam sends the invoice"],
        summaries: ["2026-11-10: - Booked the flights.", "2026-11-09: - Bali."],
      }),
    );
    expect(
      await within(panel()!).findByText("Two things."),
    ).toBeInTheDocument();
    await userEvent.click(
      within(panel()!).getByText("Used 1 fact and 2 summaries from memory"),
    );
    expect(within(panel()!).getByText("Sam sends the invoice")).toBeVisible();
    expect(within(panel()!).getByText("2026-11-09: - Bali.")).toBeVisible();
  });

  it("a new day keeps only the question and its answer in the panel", async () => {
    await startApp({
      history: [
        { ts: 1, role: "user", text: "Yesterday's question", backend: "fake", files: [] },
        { ts: 1, role: "assistant", text: "Yesterday's answer", backend: "fake", files: [] },
      ],
      onAsk: (p) => answer(`Fresh: ${p}`, { fresh: true }),
    });
    await userEvent.type(homeBox(), "good morning{Enter}");
    expect(await within(panel()!).findByText("Fresh: good morning")).toBeInTheDocument();
    expect(within(panel()!).getByText("good morning")).toBeInTheDocument();
    expect(within(panel()!).queryByText("Yesterday's question")).toBeNull();
    expect(within(panel()!).queryByText("Yesterday's answer")).toBeNull();
  });

  it("a new day while the app runs empties the chat", async () => {
    await startApp({
      history: [
        { ts: 1, role: "user", text: "Yesterday's question", backend: "fake", files: [] },
        { ts: 1, role: "assistant", text: "Yesterday's answer", backend: "fake", files: [] },
      ],
    });
    await userEvent.click(screen.getByRole("button", { name: "Conversation" }));
    expect(await within(panel()!).findByText("Yesterday's answer")).toBeInTheDocument();
    await emit("new-day");
    await waitFor(() =>
      expect(within(panel()!).queryByText("Yesterday's answer")).toBeNull(),
    );
  });

  it("shows an error in the panel when the question fails", async () => {
    await startApp({
      onAsk: () => Promise.reject("claude ended without an answer"),
    });
    await userEvent.type(homeBox(), "what is open?{Enter}");
    expect(await within(panel()!).findByRole("alert")).toHaveTextContent(
      "claude ended without an answer",
    );
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
    expect(
      await screen.findByText("Cannot change the model: database is locked"),
    ).toBeInTheDocument();
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
    await userEvent.click(
      within(panel()!).getByRole("button", { name: "New" }),
    );
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
    expect(
      await screen.findByRole("button", { name: "Open details: Reply to Sam" }),
    ).toBeInTheDocument();
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
    fireEvent.change(within(d).getByLabelText("Due date"), {
      target: { value: "2026-12-01" },
    });
    await waitFor(() => expect(fake.tasks[0].due).toBe("2026-12-01"));
    await userEvent.selectOptions(
      within(d).getByLabelText("Priority"),
      "urgent",
    );
    await waitFor(() => expect(fake.tasks[0].priority).toBe("urgent"));
    await userEvent.type(
      within(d).getByLabelText("Tags"),
      "home, #money{Enter}",
    );
    await waitFor(() => expect(fake.tasks[0].tags).toEqual(["home", "money"]));
    await userEvent.type(
      within(d).getByLabelText("Task description"),
      "Before Friday.",
    );
    await userEvent.tab();
    await waitFor(() => expect(fake.tasks[0].notes).toBe("Before Friday."));
  });

  it("adds, ticks, and removes steps", async () => {
    const fake = await startApp({ tasks: [task("Book flights")] });
    await openItem("Book flights");
    await userEvent.type(
      within(details()).getByLabelText("New step"),
      "Pick dates{Enter}",
    );
    await waitFor(() =>
      expect(fake.tasks[0].checklist).toEqual([
        { text: "Pick dates", done: false },
      ]),
    );
    await userEvent.click(
      await within(details()).findByRole("button", {
        name: "Step done: Pick dates",
      }),
    );
    await waitFor(() => expect(fake.tasks[0].checklist[0].done).toBe(true));
    await userEvent.click(
      within(details()).getByRole("button", {
        name: "Remove step: Pick dates",
      }),
    );
    await waitFor(() => expect(fake.tasks[0].checklist).toEqual([]));
  });

  it("deletes a task and closes the panel", async () => {
    const fake = await startApp({ tasks: [task("Old idea")] });
    await openItem("Old idea");
    await userEvent.click(
      within(details()).getByRole("button", { name: "Delete" }),
    );
    await waitFor(() => expect(fake.tasks).toEqual([]));
    expect(
      screen.queryByRole("dialog", { name: "Task details" }),
    ).not.toBeInTheDocument();
  });

  it("shows a GitHub item read-only, with its description, summary, and links", async () => {
    const g = {
      ...githubItem(7, "assigned", "Fix cart"),
      links: [
        {
          id: "acme/shop#8",
          repo: "acme/shop",
          number: 8,
          title: "Fix it",
          url: "https://github.com/acme/shop/pull/8",
          pull_request: true,
        },
      ],
    };
    const fake = await startApp({
      github: [g],
      details: { "github:acme/shop#7": "The cart **loses** items." },
      summaries: { "github:acme/shop#7": "Items vanish on reload." },
    });
    await openItem("Fix cart");
    const d = details();
    expect(await within(d).findByText("loses")).toBeInTheDocument();
    expect(within(d).getByText("Items vanish on reload.")).toBeInTheDocument();
    expect(
      within(d).getByRole("link", { name: "Open in GitHub ↗" }),
    ).toHaveAttribute("href", g.url);
    expect(within(d).getByRole("link", { name: "PR #8" })).toBeInTheDocument();
    expect(
      within(d).queryByRole("button", {
        name: /Mark as finished|Close in ClickUp/,
      }),
    ).not.toBeInTheDocument();
    expect(
      fake.calls
        .map((c) => c.cmd)
        .filter((c) => c.startsWith("github_") && c !== "github_items"),
    ).toEqual([]);
  });

  it("asks AIfred about the item in its own chat, with the fast model first", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent")],
      onAskSide: (p) => answer(`About it: ${p}`),
    });
    await openItem("Pay rent");
    const key = `task:${fake.tasks[0].id}`;
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "when is it due?{Enter}",
    );
    expect(
      await within(details()).findByText("About it: when is it due?"),
    ).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask_side")!.args).toMatchObject({
      thread: `item:${key}`,
      model: "haiku",
    });
    expect(fake.history).toEqual([]);
  });

  it("a file with no text in the item chat asks AIfred to look at the file", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent")],
      uploadName: "bill.pdf",
    });
    await openItem("Pay rent");
    await userEvent.upload(
      within(details()).getByLabelText("Attach files"),
      new File(["%PDF"], "bill.pdf"),
    );
    await within(details()).findByText("bill.pdf");
    await userEvent.click(
      within(details()).getByRole("button", { name: "Ask" }),
    );
    await waitFor(() =>
      expect(fake.calls.find((c) => c.cmd === "ask_side")?.args).toMatchObject({
        prompt: "Look at these files.",
        files: [fake.uploads[0].id],
      }),
    );
  });

  it("the item chat remembers Default as its model", async () => {
    localStorage.setItem("aifred-model-item", "");
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await openItem("Pay rent");
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "hi{Enter}",
    );
    await waitFor(() =>
      expect(fake.calls.find((c) => c.cmd === "ask_side")?.args).toMatchObject({
        model: "default",
      }),
    );
    localStorage.removeItem("aifred-model-item");
  });

  it("the time guess changes in steps of 15 minutes", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await openItem("Pay rent");
    expect(within(details()).getByText("30 m")).toBeInTheDocument();
    await userEvent.click(
      within(details()).getByRole("button", { name: "More time" }),
    );
    expect(within(details()).getByText("45 m")).toBeInTheDocument();
    await waitFor(() =>
      expect(fake.estimates[`task:${fake.tasks[0].id}`]).toEqual({
        minutes: 45,
        by: "you",
      }),
    );
  });
});

describe("tabs on the right side", () => {
  it("show the conversation and the task when both are open, and mark a new answer", async () => {
    const hold = deferred<Answer>();
    await startApp({ tasks: [task("Pay rent")], onAsk: () => hold.promise });
    await userEvent.type(homeBox(), "what is open?{Enter}");
    await openItem("Pay rent");
    const tabs = screen.getByRole("tablist", { name: "Open on the right" });
    expect(within(tabs).getByRole("tab", { name: /Pay rent/ })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(panel()).not.toBeVisible();
    hold.resolve(answer("Two things."));
    // Both panels have the tabs. One of them is hidden.
    expect((await screen.findAllByTitle("New answer")).length).toBeGreaterThan(
      0,
    );
    await userEvent.click(
      screen.getAllByRole("tab", { name: /Conversation/ })[0],
    );
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
  beforeEach(() => localStorage.removeItem("aifred-cal-view"));
  it("I plan it myself: add items, change the time, and take them out", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent"), task("Call Sam")],
      github: [githubItem(7, "review_requested", "Fix cart")],
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    await userEvent.click(
      screen.getByRole("button", { name: /I plan it myself/ }),
    );
    expect(await screen.findByText("Your plan is empty")).toBeInTheDocument();
    const off = screen.getByRole("list", { name: "Not in today's plan" });
    await userEvent.click(
      within(within(off).getByText("Fix cart").closest("li")!).getByRole(
        "button",
        { name: /Add/ },
      ),
    );
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([
        { key: "github:acme/shop#7" },
      ]),
    );
    expect(fake.plans[today()].mode).toBe("self");
    expect(screen.getAllByText("9:00").length).toBeGreaterThan(0);
    await userEvent.click(
      screen.getAllByRole("button", { name: "More time" })[0],
    );
    expect(await screen.findByText("9:00–9:45")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Take out of the plan: Fix cart" }),
    );
    await waitFor(() => expect(fake.plans[today()].items).toEqual([]));
  });

  it("puts plan items after a meeting, and Prep asks AIfred about the meeting", async () => {
    const t = task("Pay rent");
    const fake = await startApp({
      tasks: [t],
      plans: {
        [today()]: {
          day: today(),
          mode: "self",
          items: [{ key: `task:${t.id}` }],
          updated: 1,
        },
      },
      calendar: {
        fetched_at: Math.floor(Date.now() / 1000),
        events: [meeting("m1", "Weekly sync", "09:00", "10:00")],
      },
    });
    expect(
      await screen.findByText(/Calendar/, { selector: ".sync span" }),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    expect(await screen.findByText("Weekly sync")).toBeInTheDocument();
    expect(screen.getByText("Josh, Ana")).toBeInTheDocument();
    // The task does not go over the meeting: it starts when the meeting ends.
    expect(screen.getByText("10:00–10:30")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Prepare for: Weekly sync" }),
    );
    await waitFor(() =>
      expect(fake.calls.find((c) => c.cmd === "ask_side")?.args.prompt).toMatch(
        /Prepare me for my calendar event today at 9:00–10:00 \(event id [^)]+\)/,
      ),
    );
  });

  it("shows a calendar error in the top bar", async () => {
    await startApp({ calendarError: "gws failed: not logged in" });
    expect(
      await screen.findByTitle("gws failed: not logged in"),
    ).toHaveTextContent("Calendar: error");
  });

  it("Plan it for me asks AIfred in the plan chat, and shows the plan it saved", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent", { priority: "urgent" })],
    });
    const key = `task:${fake.tasks[0].id}`;
    fake.onAskSide = (p, _progress, args) => {
      fake.plans[args.thread.slice(5)] = {
        day: args.thread.slice(5),
        mode: "ai",
        items: [{ key, why: "It is urgent." }],
        updated: 1,
      };
      return answer(`I put [the rent](item:${key}) first.`);
    };
    await userEvent.click(
      screen.getByRole("button", { name: "Plan it for me" }),
    );
    const chat = screen.getByRole("complementary", { name: "Plan chat" });
    expect(await within(chat).findByText(/first\./)).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "ask_side")!.args).toMatchObject({
      thread: `plan:${today()}`,
      prompt: "Plan my day.",
    });
    expect(await screen.findByText("✦ Made by AIfred")).toBeInTheDocument();
    expect(screen.getByText("It is urgent.")).toBeInTheDocument();
    // The link in the answer opens the item, with tabs to go back to the chat.
    await userEvent.click(
      within(chat).getByRole("button", { name: "the rent" }),
    );
    expect(screen.getByRole("tab", { name: /Plan chat/ })).toHaveAttribute(
      "aria-selected",
      "false",
    );
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    expect(
      screen.getByText("Pay rent", { selector: ".strip b" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText("AIfred made this plan", { exact: false }),
    ).toBeInTheDocument();
  });

  it("Start over removes the plan and the plan chat", async () => {
    const rent = task("Pay rent");
    const fake = await startApp({
      tasks: [rent],
      plans: {
        [today()]: {
          day: today(),
          mode: "self",
          items: [{ key: `task:${rent.id}` }],
          updated: 1,
        },
      },
      side: {
        [`plan:${today()}`]: [
          { id: 1, ts: 1, role: "user", text: "Plan my day", files: [] },
        ],
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    expect(await screen.findByText("Made by you")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Start over" }));
    expect(
      await screen.findByText("How do you want to plan today?"),
    ).toBeInTheDocument();
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
      plans: {
        [today()]: {
          day: today(),
          mode: "self",
          items: [{ key: gh, why: "A review." }, { key: `task:${rent.id}` }],
          updated: 1,
        },
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    expect(await screen.findByText("Made by you")).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Take out of the plan: Pay rent" }),
    );
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([
        { key: gh, why: "A review." },
      ]),
    );
  });

  it("a plan change waits until the plan of the day is loaded", async () => {
    const load = deferred<void>();
    const fake = await startApp({
      tasks: [task("Pay rent")],
      onDayPlan: () => load.promise,
    });
    await userEvent.click(
      await screen.findByRole("button", { name: "+ Plan" }),
    );
    expect(await screen.findByText(/still loading/)).toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "set_day_plan")).toBe(false);
    load.resolve();
    await userEvent.click(screen.getByRole("button", { name: "+ Plan" }));
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([
        { key: `task:${fake.tasks[0].id}` },
      ]),
    );
  });

  it("Alt+Down gives a block a fixed time, and its time button takes the time away", async () => {
    const rent = task("Pay rent");
    const key = `task:${rent.id}`;
    const fake = await startApp({
      tasks: [rent],
      plans: {
        [today()]: {
          day: today(),
          mode: "self",
          items: [{ key, why: "Due." }],
          updated: 1,
        },
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    const block = await screen.findByRole("button", {
      name: "Open details: Pay rent",
    });
    fireEvent.keyDown(block, { key: "ArrowDown", altKey: true });
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([
        { key, why: "Due.", at: 555 },
      ]),
    );
    expect(await screen.findByText("9:15–9:45")).toBeInTheDocument();
    expect(
      await screen.findByText("Moved to 9:15: Pay rent"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "No fixed time: Pay rent" }),
    );
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([{ key, why: "Due." }]),
    );
  });

  it("two quick Alt+Down moves save in order, so the last time stays", async () => {
    const rent = task("Pay rent");
    const key = `task:${rent.id}`;
    const first = deferred<void>();
    let n = 0;
    const fake = await startApp({
      tasks: [rent],
      plans: {
        [today()]: { day: today(), mode: "self", items: [{ key }], updated: 1 },
      },
      // The first save is slow. Before the fix, the second save answered first and the first one won.
      onSetDayPlan: () => (n++ === 0 ? first.promise : undefined),
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    fireEvent.keyDown(
      await screen.findByRole("button", { name: "Open details: Pay rent" }),
      { key: "ArrowDown", altKey: true },
    );
    expect(await screen.findByText("9:15–9:45")).toBeInTheDocument();
    fireEvent.keyDown(
      screen.getByRole("button", { name: "Open details: Pay rent" }),
      { key: "ArrowDown", altKey: true },
    );
    expect(await screen.findByText("9:30–10:00")).toBeInTheDocument();
    first.resolve();
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([{ key, at: 570 }]),
    );
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "set_day_plan")).toHaveLength(
        2,
      ),
    );
    expect(screen.getByText("9:30–10:00")).toBeInTheDocument();
  });

  it("a plan load that fails once loads again, so the plan changes work", async () => {
    let n = 0;
    const fake = await startApp({
      tasks: [task("Pay rent")],
      onDayPlan: () =>
        n++ === 0 ? Promise.reject("the database is busy") : undefined,
    });
    // Before the fix, the plan changes stayed off until the app started again.
    await userEvent.click(
      await screen.findByRole("button", { name: "+ Plan" }),
    );
    expect(await screen.findByText(/still loading/)).toBeInTheDocument();
    await waitFor(() => expect(n).toBe(2));
    await userEvent.click(screen.getByRole("button", { name: "+ Plan" }));
    await waitFor(() =>
      expect(fake.plans[today()]?.items).toEqual([
        { key: `task:${fake.tasks[0].id}` },
      ]),
    );
  });

  it("Alt+Right moves a block to the next day in one change, and Undo moves it back", async () => {
    const rent = task("Pay rent");
    const bill = task("Pay bill");
    const [r, b] = [`task:${rent.id}`, `task:${bill.id}`];
    const fake = await startApp({
      tasks: [rent, bill],
      plans: {
        [today()]: {
          day: today(),
          mode: "ai",
          items: [{ key: r }, { key: b }],
          updated: 1,
        },
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    fireEvent.keyDown(
      await screen.findByRole("button", { name: "Open details: Pay rent" }),
      { key: "ArrowRight", altKey: true },
    );
    const next = addDays(today(), 1);
    await waitFor(() => expect(fake.plans[next]?.items).toEqual([{ key: r }]));
    expect(fake.plans[today()].items).toEqual([{ key: b }]);
    expect(fake.calls.filter((c) => c.cmd === "move_plan_item")).toHaveLength(
      1,
    );
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.plans[today()].items).toEqual([{ key: r }, { key: b }]),
    );
    expect(fake.plans[next].items).toEqual([]);
  });

  it("the week view shows 7 days, says which day is too full, and opens a day", async () => {
    const big = task("Write the report");
    const next = addDays(today(), 1);
    await startApp({
      tasks: [big],
      plans: {
        [next]: {
          day: next,
          mode: "self",
          items: [{ key: `task:${big.id}` }],
          updated: 1,
        },
      },
      estimates: { [`task:${big.id}`]: { minutes: 600, by: "you" } },
    });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    await userEvent.click(screen.getByRole("button", { name: "Week" }));
    for (let n = 1; n < 7; n++)
      expect(
        screen.getByRole("group", {
          name: `Calendar of ${shortDay(addDays(today(), n), today())}`,
        }),
      ).toBeInTheDocument();
    expect(
      screen.getByRole("button", {
        name: `Open ${shortDay(today(), today())}`,
      }),
    ).toHaveTextContent("No plan");
    expect(
      within(
        screen.getByRole("button", { name: `Open ${shortDay(next, today())}` }),
      ).getByText(/Too full by/),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: `Open ${shortDay(next, today())}` }),
    );
    expect(screen.getByRole("button", { name: "Day" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      await screen.findByText(/This day is too full by/),
    ).toBeInTheDocument();
  });

  it("the plan chat of another day plans that day", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    await userEvent.click(screen.getByRole("button", { name: "The next day" }));
    const next = addDays(today(), 1);
    expect(
      await screen.findByText(
        `How do you want to plan ${shortDay(next, today())}?`,
      ),
    ).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "The next day" })).toBeEnabled();
    fake.onAskSide = () => answer("Done.");
    await userEvent.click(
      screen.getByRole("button", { name: /Plan it for me/ }),
    );
    await waitFor(() =>
      expect(fake.calls.find((c) => c.cmd === "ask_side")?.args.thread).toBe(
        `plan:${next}`,
      ),
    );
    expect(
      screen.getByText(`Plan chat for ${shortDay(next, today())}`),
    ).toBeInTheDocument();
  });

  it("two fast clicks on Plan it for me start only one answer", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    const slow = deferred<Answer>();
    fake.onAskSide = () => slow.promise;
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    const btn = await screen.findByRole("button", { name: /Plan it for me/ });
    // Two clicks before the next render: the button is not disabled yet.
    act(() => {
      fireEvent.click(btn);
      fireEvent.click(btn);
    });
    const chat = screen.getByRole("complementary", { name: "Plan chat" });
    expect(await within(chat).findByText("Plan my day.")).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "ask_side")).toHaveLength(1);
    slow.resolve(answer("Done planning."));
    expect(await within(chat).findByText("Done planning.")).toBeInTheDocument();
    expect(within(chat).getAllByText("Plan my day.")).toHaveLength(1);
  });

  it("a running plan answer stays with its day, and a second plan waits for it", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    const slow = deferred<Answer>();
    fake.onAskSide = () => slow.promise;
    await userEvent.click(screen.getByRole("button", { name: "Calendar" }));
    await userEvent.click(
      await screen.findByRole("button", { name: /Plan it for me/ }),
    );
    const chat = screen.getByRole("complementary", { name: "Plan chat" });
    expect(await within(chat).findByText("Plan my day.")).toBeInTheDocument();
    // Another day: its plan does not start while the first answer runs.
    await userEvent.click(screen.getByRole("button", { name: "The next day" }));
    expect(within(chat).queryByText("Plan my day.")).not.toBeInTheDocument();
    await userEvent.click(
      await screen.findByRole("button", { name: /Plan it for me/ }),
    );
    expect(
      await screen.findByText(/AIfred is still working on today's plan/),
    ).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "ask_side")).toHaveLength(1);
    // Back to today: the question and the answer are there.
    await userEvent.click(
      screen.getByRole("button", { name: "The day before" }),
    );
    expect(await within(chat).findByText("Plan my day.")).toBeInTheDocument();
    slow.resolve(answer("Done planning."));
    expect(await within(chat).findByText("Done planning.")).toBeInTheDocument();
  });

  it("+ Plan on the home list adds the item to today's plan", async () => {
    const fake = await startApp({ tasks: [task("Pay rent")] });
    await userEvent.click(
      await screen.findByRole("button", { name: "+ Plan" }),
    );
    expect(await screen.findByText("Planned 9:00")).toBeInTheDocument();
    expect(fake.plans[today()].items).toEqual([
      { key: `task:${fake.tasks[0].id}` },
    ]);
  });
});

describe("GitHub list", () => {
  it("asks GitHub once at start, and again with the reload button", async () => {
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
    });
    await screen.findByText("Fix cart");
    expect(fake.githubFetches).toBe(1);
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() => expect(fake.githubFetches).toBe(2));
  });

  it("reloads on the timer from the settings, and never when it is 'only when I click reload'", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({
        github: [githubItem(7, "assigned")],
        settings: { ...DEFAULT_SETTINGS, refresh_minutes: 5 },
      });
      await waitFor(() => expect(fake.githubFetches).toBe(1));
      await vi.advanceTimersByTimeAsync(5 * 60_000);
      await waitFor(() => expect(fake.githubFetches).toBe(2));
    } finally {
      vi.useRealTimers();
    }
  });

  it("an item closed on GitHub leaves the list on the next reload", async () => {
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
    });
    await screen.findByText("Fix cart");
    fake.github = [];
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() =>
      expect(screen.queryByText("Fix cart")).not.toBeInTheDocument(),
    );
  });

  it("shows the saved list with a warning when GitHub cannot be reached", async () => {
    await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
      githubWarning: "Cannot reach GitHub, so this is the saved list.",
    });
    expect(await screen.findByText("Fix cart")).toBeInTheDocument();
    expect(
      screen.getByText("Cannot reach GitHub, so this is the saved list."),
    ).toBeInTheDocument();
  });
});

describe("ClickUp", () => {
  const connected = { clickupToken: "pk_test" };

  it("makes no ClickUp call when it is not connected", async () => {
    const fake = await startApp({
      clickup: [clickupItem("86a", "Write notes")],
    });
    expect(screen.queryByText("Write notes")).not.toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "clickup_items")).toBe(false);
  });

  it("the tick closes the task in ClickUp, and Undo puts the old status back", async () => {
    const fake = await startApp({
      ...connected,
      clickup: [clickupItem("86a", "Write notes", { status: "review" })],
    });
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Close in ClickUp: Write notes",
      }),
    );
    expect(
      await screen.findByText("Closed in ClickUp: Write notes"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Open details: Write notes" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Undo: Write notes" }),
    );
    expect(
      await screen.findByRole("button", { name: "Open details: Write notes" }),
    ).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "clickup_undo_done")!.args).toEqual(
      { id: "86a", status: "review" },
    );
  });

  it("a second close keeps the undo of the first", async () => {
    const fake = await startApp({
      ...connected,
      clickup: [
        clickupItem("86a", "Write notes", { status: "review" }),
        clickupItem("86b", "Call Bob"),
      ],
    });
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Close in ClickUp: Write notes",
      }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Close in ClickUp: Call Bob" }),
    );
    expect(
      await screen.findByText("Closed in ClickUp: Call Bob"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Undo: Write notes" }),
    );
    await waitFor(() =>
      expect(
        fake.calls.find((c) => c.cmd === "clickup_undo_done")!.args,
      ).toEqual({ id: "86a", status: "review" }),
    );
    expect(
      screen.getByRole("button", { name: "Undo: Call Bob" }),
    ).toBeInTheDocument();
  });

  it("a failed close keeps the task and says why", async () => {
    await startApp({
      ...connected,
      clickup: [clickupItem("86a", "Write notes")],
      fail: { clickup_mark_done: "ClickUp said 403: no access" },
    });
    await userEvent.click(
      await screen.findByRole("button", {
        name: "Close in ClickUp: Write notes",
      }),
    );
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "ClickUp said 403: no access",
    );
    expect(
      screen.getByRole("button", { name: "Open details: Write notes" }),
    ).toBeInTheDocument();
  });

  it("connects with a token on the Settings page, then loads the tasks", async () => {
    const fake = await startApp({
      clickup: [clickupItem("86a", "Write notes")],
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
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
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    const g = screen.getByRole("region", { name: "Google" });
    await userEvent.type(within(g).getByLabelText("Client ID"), "bad");
    await userEvent.type(within(g).getByLabelText("Client secret"), "s");
    await userEvent.click(
      within(g).getByRole("button", { name: "Save registration" }),
    );
    expect(
      await within(g).findByText(/does not look like a Google client ID/),
    ).toBeInTheDocument();
    await userEvent.clear(within(g).getByLabelText("Client ID"));
    await userEvent.type(
      within(g).getByLabelText("Client ID"),
      "1-a.apps.googleusercontent.com",
    );
    await userEvent.click(
      within(g).getByRole("button", { name: "Save registration" }),
    );
    const loads = () =>
      fake.calls.filter((c) => c.cmd === "calendar_events").length;
    const before = loads();
    await userEvent.click(
      await within(g).findByRole("button", { name: "Connect Google" }),
    );
    expect(await within(g).findByText("me@x.com")).toBeInTheDocument();
    expect(within(g).getByText("Calendar: allowed")).toBeInTheDocument();
    await waitFor(() => expect(loads()).toBeGreaterThan(before));
    expect(
      await screen.findByText(/Calendar/, { selector: ".sync span" }),
    ).toBeInTheDocument();
    await userEvent.click(
      within(g).getByRole("button", { name: "Disconnect" }),
    );
    expect(
      await within(g).findByRole("button", { name: "Connect Google" }),
    ).toBeInTheDocument();
    expect(fake.google.connected).toBe(false);
  });

  it("lets the user start the login again while one waits", async () => {
    const first = deferred<void>();
    let logins = 0;
    await startApp({
      google: {
        client_id: "1-a.apps.googleusercontent.com",
        secret_set: true,
        connected: false,
        email: null,
        features: [["Calendar", false]],
        optional: ["Gmail"],
        off: [],
      },
      onConnectGoogle: () => (++logins === 1 ? first.promise : undefined),
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    const g = screen.getByRole("region", { name: "Google" });
    await userEvent.click(
      await within(g).findByRole("button", { name: "Connect Google" }),
    );
    // The user closed the browser tab: the button still works.
    const waiting = await within(g).findByRole("button", {
      name: "Finish the login in your browser…",
    });
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
    await startApp({
      google: {
        client_id: "1-a.apps.googleusercontent.com",
        secret_set: true,
        connected: true,
        email: "me@x.com",
        features: [["Calendar", false]],
        optional: ["Gmail"],
        off: [],
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    expect(
      await screen.findByText(/Google needs more access for Calendar/),
    ).toBeInTheDocument();
  });
});

describe("Gmail and the waiting list", () => {
  const now = () => Math.floor(Date.now() / 1000);

  it("an email that waits for your answer is an item, and 'No answer needed' hides it with an undo", async () => {
    const fake = await startApp({
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply", { unread: true })],
        sent: [],
      },
    });
    const open = await screen.findByRole("list", { name: "Open" });
    expect(within(open).getByText("Price for the plugin")).toBeInTheDocument();
    expect(within(open).getByText(/Email · from Ana Lee/)).toBeInTheDocument();
    expect(within(open).getByText("Unread · Waits 3 days")).toBeInTheDocument();
    await userEvent.click(
      within(open).getByRole("button", {
        name: "No answer needed: Price for the plugin",
      }),
    );
    await waitFor(() =>
      expect(
        screen.queryByText("Price for the plugin"),
      ).not.toBeInTheDocument(),
    );
    expect(fake.mailHidden).toEqual(["r1"]);
    expect(
      await screen.findByText(
        /Hidden: Price for the plugin. It comes back when a new message comes./,
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    expect(await screen.findByText("Price for the plugin")).toBeInTheDocument();
    expect(fake.mailHidden).toEqual([]);
  });

  it("a Gmail load that started before 'No answer needed' does not show the email again", async () => {
    const fake = await startApp({
      mail: {
        fetched_at: now(),
        reply: [mail("r1", "Price for the plugin", "reply")],
        sent: [],
      },
    });
    const open = await screen.findByRole("list", { name: "Open" });
    await within(open).findByText("Price for the plugin");
    // A reload starts. Its list still has the email, and its answer waits.
    const hold = deferred<void>();
    fake.onGmailThreads = () => hold.promise;
    await userEvent.click(
      screen.getByRole("button", { name: "Reload the list" }),
    );
    await waitFor(() =>
      expect(
        fake.calls.filter((c) => c.cmd === "gmail_threads" && c.args.force)
          .length,
      ).toBe(1),
    );
    await userEvent.click(
      within(open).getByRole("button", {
        name: "No answer needed: Price for the plugin",
      }),
    );
    await waitFor(() =>
      expect(
        screen.queryByText("Price for the plugin"),
      ).not.toBeInTheDocument(),
    );
    fake.onGmailThreads = undefined;
    hold.resolve();
    // The old answer is not shown: the window reads the saved list again.
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "gmail_saved")).toBe(true),
    );
    await new Promise((r) => setTimeout(r, 0));
    expect(screen.queryByText("Price for the plugin")).not.toBeInTheDocument();
  });

  it("a double click on Track adds the email once", async () => {
    const fake = await startApp({
      mail: {
        fetched_at: now(),
        reply: [],
        sent: [mail("s1", "Contract draft", "sent")],
      },
    });
    const card = await screen.findByRole("region", { name: "Waiting for" });
    const track = within(card).getByRole("button", { name: "Track" });
    fireEvent.click(track);
    fireEvent.click(track);
    await within(card).findByRole("list", { name: "Waiting for list" });
    expect(fake.calls.filter((c) => c.cmd === "add_waiting").length).toBe(1);
    expect(fake.waiting.length).toBe(1);
  });

  it("a deleted email item goes back to the suggestions with no Gmail download", async () => {
    const fake = await startApp({
      mail: {
        fetched_at: now(),
        reply: [],
        sent: [mail("s1", "Contract draft", "sent")],
      },
    });
    const card = await screen.findByRole("region", { name: "Waiting for" });
    await userEvent.click(within(card).getByRole("button", { name: "Track" }));
    const list = await within(card).findByRole("list", {
      name: "Waiting for list",
    });
    const loads = fake.calls.filter((c) => c.cmd === "gmail_threads").length;
    await userEvent.click(
      within(list).getByRole("button", { name: /^Delete: / }),
    );
    const sugg = await within(card).findByRole("list", {
      name: "Emails with no answer yet",
    });
    expect(within(sugg).getByText("Contract draft")).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "gmail_threads").length).toBe(
      loads,
    );
  });

  it("shows an email in the panel with the day it waits since", async () => {
    await startApp({
      mail: {
        fetched_at: now(),
        reply: [
          mail("r1", "Price for the plugin", "reply", {
            direct: false,
            messages: 2,
          }),
        ],
        sent: [],
      },
    });
    await openItem("Price for the plugin");
    const d = details();
    expect(within(d).getByText("Waits since")).toBeInTheDocument();
    expect(
      within(d).getByText(/2 messages · you are in Cc/),
    ).toBeInTheDocument();
    expect(
      within(d).getByRole("button", { name: "No answer needed" }),
    ).toBeInTheDocument();
  });

  it("tracks a sent email, closes the item, and adds one by hand", async () => {
    const fake = await startApp({
      mail: {
        fetched_at: now(),
        reply: [],
        sent: [
          mail("s1", "Contract draft", "sent"),
          mail("s2", "Lunch?", "sent", { people: ["Bo"] }),
        ],
      },
    });
    const card = await screen.findByRole("region", { name: "Waiting for" });
    const sugg = within(card).getByRole("list", {
      name: "Emails with no answer yet",
    });
    expect(within(sugg).getByText(/to Ana Lee · sent/)).toBeInTheDocument();
    await userEvent.click(
      within(sugg).getAllByRole("button", { name: "Track" })[0],
    );
    const list = await within(card).findByRole("list", {
      name: "Waiting for list",
    });
    expect(
      within(list).getByText(/An answer to "Contract draft"/),
    ).toBeInTheDocument();
    await waitFor(() =>
      expect(
        within(card).queryByText("Contract draft"),
      ).not.toBeInTheDocument(),
    );
    expect(fake.waiting[0].thread).toBe("s1");

    // Hide the other suggestion: you wait for nothing.
    await userEvent.click(within(card).getByRole("button", { name: "Hide" }));
    await waitFor(() =>
      expect(
        within(card).queryByRole("list", { name: "Emails with no answer yet" }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.mailHidden).toEqual(["s2"]);

    // A check day. Typing the year gives days like 0002-01-04 first: only the
    // last day saves, once.
    const box = within(list).getByLabelText(/Check day: An answer to/);
    const before = fake.calls.filter((c) => c.cmd === "update_waiting").length;
    for (const v of ["0002-01-04", "0020-01-04", "0203-01-04", "2030-01-04"])
      fireEvent.change(box, { target: { value: v } });
    await waitFor(() => expect(fake.waiting[0].follow_up).toBe("2030-01-04"));
    expect(
      fake.calls.filter((c) => c.cmd === "update_waiting").length - before,
    ).toBe(1);
    expect(
      await within(list).findByText(/Check Fri 4 Jan 2030/),
    ).toBeInTheDocument();

    await userEvent.click(within(list).getByRole("button", { name: "Got it" }));
    expect(await within(list).findByText("closed by you")).toBeInTheDocument();
    expect(fake.waiting[0].done).toBe(true);

    await userEvent.click(within(card).getByRole("button", { name: "Add" }));
    await userEvent.type(
      within(card).getByLabelText("What you wait for"),
      "The signed contract",
    );
    await userEvent.type(within(card).getByLabelText("Who you wait for"), "Cy");
    await userEvent.click(within(card).getByRole("button", { name: "Add" }));
    expect(
      await within(card).findByText(/The signed contract/),
    ).toBeInTheDocument();
    expect(fake.waiting.map((w) => w.who)).toEqual(["Ana Lee", "Cy"]);
  });

  it("shows only a small link when there is nothing to wait for", async () => {
    await startApp();
    await userEvent.click(
      await screen.findByRole("button", { name: "+ Waiting for someone?" }),
    );
    expect(
      screen.getByRole("region", { name: "Waiting for" }),
    ).toBeInTheDocument();
    expect(screen.getByLabelText("What you wait for")).toHaveFocus();
  });

  it("shows a Gmail error, and asks to connect again when the login does not allow Gmail", async () => {
    await startApp({
      mailError: "Gmail said: Rate limit",
      google: {
        client_id: "1-a.apps.googleusercontent.com",
        secret_set: true,
        connected: true,
        email: "me@x.com",
        features: [
          ["Calendar", true],
          ["Gmail", false],
        ],
        optional: ["Gmail"],
        off: [],
      },
    });
    expect(
      await screen.findByText("Gmail: Gmail said: Rate limit"),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    expect(
      await screen.findByText(/Google needs more access for Gmail/),
    ).toBeInTheDocument();
  });
});

describe("the Gmail switch", () => {
  it("turns Gmail off: no emails, no error, and no 'connect again' note", async () => {
    const fake = await startApp({
      mailError: "Gmail API has not been used in project 123 or it is disabled",
      google: {
        client_id: "1-a.apps.googleusercontent.com",
        secret_set: true,
        connected: true,
        email: "me@x.com",
        features: [
          ["Calendar", true],
          ["Gmail", false],
        ],
        optional: ["Gmail"],
        off: [],
      },
    });
    expect(
      await screen.findByText(/Gmail: Gmail API has not been used/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    const g = screen.getByRole("region", { name: "Google" });
    expect(
      await within(g).findByText(/Google needs more access for Gmail/),
    ).toBeInTheDocument();
    await userEvent.click(
      within(g).getByRole("checkbox", { name: "Use Gmail" }),
    );
    expect(
      await within(g).findByText(
        "Gmail is off. AIfred removed the saved data of Gmail.",
      ),
    ).toBeInTheDocument();
    expect(fake.google.off).toEqual(["Gmail"]);
    expect(
      within(g).queryByText(/Google needs more access/),
    ).not.toBeInTheDocument();
    expect(within(g).getByText("Gmail: turned off")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Home" }));
    await waitFor(() =>
      expect(screen.queryByText(/Gmail: Gmail API/)).not.toBeInTheDocument(),
    );
    expect(
      screen.queryByText(/Gmail/, { selector: ".sync span" }),
    ).not.toBeInTheDocument();
  });

  it("turns Gmail on again and asks to connect again", async () => {
    await startApp({
      google: {
        client_id: "1-a.apps.googleusercontent.com",
        secret_set: true,
        connected: true,
        email: "me@x.com",
        features: [
          ["Calendar", true],
          ["Gmail", false],
        ],
        optional: ["Gmail"],
        off: ["Gmail"],
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "Connections" }),
    );
    const g = screen.getByRole("region", { name: "Google" });
    await userEvent.click(
      await within(g).findByRole("checkbox", { name: "Use Gmail" }),
    );
    expect(
      await within(g).findByText("Gmail is on. Connect again, and allow it."),
    ).toBeInTheDocument();
  });
});

describe("settings page", () => {
  it("saves the reload time and the work day", async () => {
    const fake = await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.selectOptions(
      screen.getByLabelText("Reload GitHub and ClickUp items"),
      "Only when I click reload",
    );
    fireEvent.change(screen.getByLabelText("Work day starts"), {
      target: { value: "08:30" },
    });
    await userEvent.selectOptions(
      screen.getByLabelText("Lunch length"),
      "No lunch break",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(fake.settings).toMatchObject({
      refresh_minutes: 0,
      work_start: "08:30",
      lunch_minutes: 0,
    });
  });

  it("shows a failed save", async () => {
    await startApp({ fail: { set_settings: "read-only folder" } });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.selectOptions(
      screen.getByLabelText("Reload GitHub and ClickUp items"),
      "Every hour",
    );
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText("Not saved: read-only folder"),
    ).toBeInTheDocument();
  });

  it("checks for a new version and restarts into it", async () => {
    const fake = await startApp({ updateTo: "0.2.0" });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(await screen.findByRole("tab", { name: "About" }));
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(
      await screen.findByText(/Version 0.2.0 is downloaded/),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Restart now" }));
    await waitFor(() => expect(fake.restarts).toBe(1));
  });
});

describe("memory page", () => {
  const openMemory = () =>
    userEvent.click(
      screen.getByRole("button", { name: "What AIfred knows about you" }),
    );

  it("adds a fact, and forgets and restores one", async () => {
    const fake = await startApp({
      memory: [fact("Old job at Acme", { added_by: "aifred" })],
    });
    await openMemory();
    await userEvent.type(
      screen.getByPlaceholderText("Add a fact and press Enter"),
      "I like short answers.{Enter}",
    );
    expect(
      await screen.findByText("I like short answers."),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Forget: Old job at Acme" }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: /Forgotten \(1\)/ }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Restore: Old job at Acme" }),
    );
    await waitFor(() => expect(fake.memory[0].forgotten).toBeNull());
  });

  it("edits the tags of a fact and filters the list by a tag", async () => {
    const fake = await startApp({
      memory: [
        fact("Zara is our QA lead", { tags: ["people"] }),
        fact("I like short answers."),
      ],
    });
    await openMemory();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit tags: I like short answers." }),
    );
    const box = screen.getByRole("textbox", {
      name: "Tags: I like short answers.",
    });
    await userEvent.type(box, "Style, #answers{Enter}");
    await waitFor(() =>
      expect(fake.memory[1].tags).toEqual(["style", "answers"]),
    );
    const filter = screen.getByRole("group", { name: "Filter by tag" });
    await userEvent.click(
      within(filter).getByRole("button", { name: "#people 1" }),
    );
    expect(screen.getByText("Zara is our QA lead")).toBeInTheDocument();
    expect(screen.queryByText("I like short answers.")).not.toBeInTheDocument();
    await userEvent.click(within(filter).getByRole("button", { name: "All" }));
    expect(screen.getByText("I like short answers.")).toBeInTheDocument();
    // A tag on a fact also filters the list.
    await userEvent.click(screen.getByRole("button", { name: "#style" }));
    expect(screen.queryByText("Zara is our QA lead")).not.toBeInTheDocument();
  });

  it("shows the error for a tag that is too long and keeps the box open", async () => {
    const fake = await startApp({ memory: [fact("I like tea")] });
    await openMemory();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit tags: I like tea" }),
    );
    const box = screen.getByRole("textbox", { name: "Tags: I like tea" });
    await userEvent.type(box, `drinks, ${"a".repeat(41)}{Enter}`);
    expect(
      await screen.findByText(
        /Not saved: the tag "a{41}" is longer than 40 characters/,
      ),
    ).toBeInTheDocument();
    expect(fake.memory[0].tags).toEqual([]);
    expect(box).toBeInTheDocument();
    // Escape closes the box and keeps the old tags.
    await userEvent.type(box, "{Escape}");
    expect(
      screen.queryByRole("textbox", { name: "Tags: I like tea" }),
    ).not.toBeInTheDocument();
    expect(fake.memory[0].tags).toEqual([]);
  });

  it("saves the tags one time when the box loses focus during the save", async () => {
    const fake = await startApp({ memory: [fact("I like tea")] });
    const slow = deferred<void>();
    fake.onSetFactTags = () => slow.promise;
    await openMemory();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit tags: I like tea" }),
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "Tags: I like tea" }),
      "drinks{Enter}",
    );
    // A click away while the save runs.
    await userEvent.click(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    );
    slow.resolve();
    await waitFor(() => expect(fake.memory[0].tags).toEqual(["drinks"]));
    expect(fake.calls.filter((c) => c.cmd === "set_fact_tags")).toHaveLength(1);
  });

  it("does not send a failed tag save again on each click away", async () => {
    const fake = await startApp({ memory: [fact("I like tea")] });
    await openMemory();
    await userEvent.click(
      screen.getByRole("button", { name: "Edit tags: I like tea" }),
    );
    const box = screen.getByRole("textbox", { name: "Tags: I like tea" });
    await userEvent.type(box, `${"a".repeat(41)}{Enter}`);
    expect(await screen.findByText(/Not saved: the tag/)).toBeInTheDocument();
    await userEvent.click(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    );
    await userEvent.click(box);
    await userEvent.click(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    );
    expect(fake.calls.filter((c) => c.cmd === "set_fact_tags")).toHaveLength(1);
    // Enter tries the same value again.
    await userEvent.type(box, "{Enter}");
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "set_fact_tags")).toHaveLength(
        2,
      ),
    );
    // A changed value is saved.
    await userEvent.clear(box);
    await userEvent.type(box, "drinks{Enter}");
    await waitFor(() => expect(fake.memory[0].tags).toEqual(["drinks"]));
  });

  it("saves the fact text one time when the box loses focus during the save", async () => {
    const fake = await startApp({ memory: [fact("I like tea")] });
    const slow = deferred<void>();
    fake.onEditFact = () => slow.promise;
    await openMemory();
    await userEvent.click(screen.getByRole("button", { name: "I like tea" }));
    await userEvent.type(
      screen.getByRole("textbox", { name: "Fact text" }),
      " a lot{Enter}",
    );
    await userEvent.click(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    );
    slow.resolve();
    await waitFor(() => expect(fake.memory[0].text).toBe("I like tea a lot"));
    expect(fake.calls.filter((c) => c.cmd === "edit_fact")).toHaveLength(1);
  });

  it("does not send a failed text save again on a click away, but Enter tries again", async () => {
    const fake = await startApp({
      memory: [fact("I like tea")],
      fail: { edit_fact: "database is locked" },
    });
    await openMemory();
    await userEvent.click(screen.getByRole("button", { name: "I like tea" }));
    const box = screen.getByRole("textbox", { name: "Fact text" });
    await userEvent.type(box, " a lot{Enter}");
    expect(
      await screen.findByText("Not saved: database is locked"),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    );
    await userEvent.click(box);
    await userEvent.click(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    );
    expect(fake.calls.filter((c) => c.cmd === "edit_fact")).toHaveLength(1);
    delete fake.fail.edit_fact;
    await userEvent.click(box);
    await userEvent.keyboard("{Enter}");
    await waitFor(() => expect(fake.memory[0].text).toBe("I like tea a lot"));
    expect(fake.calls.filter((c) => c.cmd === "edit_fact")).toHaveLength(2);
  });

  it("a tag filter goes away when no fact has the tag, and does not come back", async () => {
    const fake = await startApp({
      memory: [
        fact("Zara is our QA lead", { tags: ["testing"] }),
        fact("I like tea", { tags: ["drinks"] }),
      ],
    });
    await openMemory();
    const filter = screen.getByRole("group", { name: "Filter by tag" });
    await userEvent.click(
      within(filter).getByRole("button", { name: "#testing 1" }),
    );
    expect(screen.queryByText("I like tea")).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Forget: Zara is our QA lead" }),
    );
    expect(await screen.findByText("I like tea")).toBeInTheDocument();
    // The tag comes back on another fact. The list still shows all facts.
    await userEvent.click(
      screen.getByRole("button", { name: "Edit tags: I like tea" }),
    );
    await userEvent.type(
      screen.getByRole("textbox", { name: "Tags: I like tea" }),
      ", testing{Enter}",
    );
    await waitFor(() =>
      expect(fake.memory[1].tags).toEqual(["drinks", "testing"]),
    );
    expect(within(filter).getByRole("button", { name: "All" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
  });

  it("deletes a forgotten fact for good only after a second click", async () => {
    const fake = await startApp({
      memory: [fact("Door code 4711", { forgotten: 1 }), fact("I like tea")],
    });
    await openMemory();
    await userEvent.click(
      screen.getByRole("button", { name: /Forgotten \(1\)/ }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Delete for good: Door code 4711" }),
    );
    expect(fake.memory).toHaveLength(2);
    await userEvent.click(
      screen.getByRole("button", {
        name: "Click again to delete for good: Door code 4711",
      }),
    );
    await waitFor(() =>
      expect(fake.memory.map((f) => f.text)).toEqual(["I like tea"]),
    );
    expect(screen.queryByText("Door code 4711")).not.toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: /Forgotten/ }),
    ).not.toBeInTheDocument();
  });

  it("a failed delete says so and keeps the fact", async () => {
    await startApp({
      memory: [fact("Door code 4711", { forgotten: 1 })],
      fail: { delete_fact: "database is locked" },
    });
    await openMemory();
    await userEvent.click(
      screen.getByRole("button", { name: /Forgotten \(1\)/ }),
    );
    await userEvent.click(
      screen.getByRole("button", { name: "Delete for good: Door code 4711" }),
    );
    await userEvent.click(
      screen.getByRole("button", {
        name: "Click again to delete for good: Door code 4711",
      }),
    );
    expect(
      await screen.findByText("Not deleted: database is locked"),
    ).toBeInTheDocument();
    expect(screen.getByText("Door code 4711")).toBeInTheDocument();
  });

  it("a failed save says so and keeps the text", async () => {
    await startApp({ fail: { add_fact: "database is locked" } });
    await openMemory();
    await userEvent.type(
      screen.getByPlaceholderText("Add a fact and press Enter"),
      "Note{Enter}",
    );
    expect(
      await screen.findByText("Not saved: database is locked"),
    ).toBeInTheDocument();
    expect(
      screen.getByPlaceholderText("Add a fact and press Enter"),
    ).toHaveValue("Note");
  });
});

describe("Tasks page", () => {
  // The page keeps the last filter in the window storage.
  beforeEach(() => localStorage.removeItem("aifred-task-filter"));
  const openTasks = async (filter?: string) => {
    await userEvent.click(screen.getByRole("button", { name: "Tasks" }));
    if (filter)
      await userEvent.click(
        within(screen.getByRole("group", { name: "Whose tasks" })).getByRole(
          "button",
          { name: filter },
        ),
      );
  };

  it("starts with your tasks, and the filter shows AIfred's tasks or both", async () => {
    await startApp({
      tasks: [task("Pay rent")],
      jobs: [
        job("j1", "Timesheet reminder"),
        job("brief-on-task-done", "Update the brief when you finish a task", {
          action: "brief",
          trigger: { kind: "event", event: "task_done", wait_secs: 90 },
        }),
      ],
    });
    await openTasks();
    expect(screen.getByRole("button", { name: "Mine" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    const mine = screen.getByRole("region", { name: "Your tasks" });
    expect(await within(mine).findByText("Pay rent")).toBeInTheDocument();
    expect(screen.queryByText("Timesheet reminder")).not.toBeInTheDocument();

    await openTasks("AIfred's");
    expect(await screen.findByText("Timesheet reminder")).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Your tasks" }),
    ).not.toBeInTheDocument();
    // The brief job is not work: it is on the Settings page.
    expect(
      screen.queryByText("Update the brief when you finish a task"),
    ).not.toBeInTheDocument();

    await openTasks("Both");
    expect(
      within(screen.getByRole("region", { name: "Your tasks" })).getByText(
        "Pay rent",
      ),
    ).toBeInTheDocument();
    expect(
      await within(
        screen.getByRole("region", { name: "AIfred's tasks" }),
      ).findByText("Timesheet reminder"),
    ).toBeInTheDocument();
  });

  it("shows the open subtasks of a row as a tree, each level indented", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      github: [githubItem(12, "your_pr", "DEV-42: round the tax")],
      clickup: [clickupItem("86a", "Checkout totals", { custom_id: "DEV-42" })],
      tasks: [
        task("Tax work", { id: "t1" }),
        task("Write the test", { id: "t2" }),
      ],
      links: [
        { key: "task:t1", main: "clickup:86a" },
        { key: "github:acme/shop#12", main: "task:t1" },
        { key: "task:t2", main: "task:t1" },
      ],
    });
    await openTasks();
    const tree = await screen.findByRole("list", {
      name: "Subtasks of Checkout totals",
    });
    const rows = within(tree).getAllByRole("listitem");
    expect(rows.map((r) => r.style.paddingLeft)).toEqual([
      "16px",
      "32px",
      "32px",
    ]);
    expect(
      within(rows[0]).getByRole("button", { name: "Open details: Tax work" }),
    ).toBeInTheDocument();
    // The row shows the tree, not the short "Subtasks" line of the home page.
    expect(
      within(tree.closest("li.row") as HTMLElement).queryByText("Subtasks"),
    ).not.toBeInTheDocument();
    // A tick on a subtask finishes only that subtask.
    await userEvent.click(
      within(tree).getByRole("button", { name: "Finish: Write the test" }),
    );
    await waitFor(() =>
      expect(fake.tasks.find((t) => t.id === "t2")?.done).toBe(true),
    );
    expect(fake.tasks.find((t) => t.id === "t1")?.done).toBe(false);
    await waitFor(() =>
      expect(
        within(tree).queryByText("Write the test"),
      ).not.toBeInTheDocument(),
    );
  });

  it("indents a subtask under its nearest open parent when a parent in the middle is finished", async () => {
    const now = Math.floor(Date.now() / 1000);
    await startApp({
      tasks: [
        task("Top", { id: "t1" }),
        task("Middle", { id: "t2", done: true, done_at: now }),
        task("Under middle", { id: "t3" }),
        task("Second", { id: "t4" }),
      ],
      links: [
        { key: "task:t2", main: "task:t1" },
        { key: "task:t3", main: "task:t2" },
        { key: "task:t4", main: "task:t1" },
      ],
    });
    await openTasks();
    // "Middle" is finished and does not show, so "Under middle" shows one
    // level under "Top", the same as "Second". It must not be two levels in.
    const tree = await screen.findByRole("list", { name: "Subtasks of Top" });
    const rows = within(tree).getAllByRole("listitem");
    expect(
      rows.map((r) => [
        within(r).getByRole("button", { name: /^Open details/ }).textContent,
        r.style.paddingLeft,
      ]),
    ).toEqual([
      ["Under middle", "16px"],
      ["Second", "16px"],
    ]);
  });

  it("marks an opened subtask line, so the page can scroll to it", async () => {
    await startApp({
      tasks: [
        task("Tax work", { id: "t1" }),
        task("Write the test", { id: "t2" }),
      ],
      links: [{ key: "task:t2", main: "task:t1" }],
    });
    await openTasks();
    const tree = await screen.findByRole("list", {
      name: "Subtasks of Tax work",
    });
    const line = within(tree).getByRole("listitem");
    expect(line.id).toBe("row-task:t2");
    expect(line).not.toHaveClass("sel");
    await userEvent.click(
      within(line).getByRole("button", {
        name: "Open details: Write the test",
      }),
    );
    await waitFor(() => expect(line).toHaveClass("sel"));
  });

  it("shows a task that AIfred adds by itself, with no reload", async () => {
    const fake = await startApp({
      tasks: [task("Pay rent")],
      jobs: [job("j1", "Water the plants", { action: "task" })],
    });
    await openTasks("Both");
    expect(await screen.findByText("Adds your task")).toBeInTheDocument();
    fake.tasks.push(task("Water the plants"));
    await emit("tasks-changed");
    const mine = screen.getByRole("region", { name: "Your tasks" });
    expect(
      await within(mine).findByText("Water the plants"),
    ).toBeInTheDocument();
  });

  it("a click on a task opens its panel and stays on the Tasks page", async () => {
    await startApp({ tasks: [task("Pay rent")] });
    await openTasks();
    await userEvent.click(
      await screen.findByRole("button", { name: "Open details: Pay rent" }),
    );
    expect(
      within(details()).getByRole("textbox", { name: "Task title" }),
    ).toHaveValue("Pay rent");
    expect(screen.getByRole("heading", { name: "Tasks" })).toBeInTheDocument();
  });

  it("an item that opens from the item panel shows its row also when the filter is AIfred's", async () => {
    await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("86a", "Checkout totals")],
      tasks: [task("Tax work", { id: "t1" })],
      links: [{ key: "clickup:86a", main: "task:t1" }],
    });
    await openTasks();
    await openItem("Tax work");
    // The panel stays open when you show AIfred's tasks.
    await userEvent.click(
      within(screen.getByRole("group", { name: "Whose tasks" })).getByRole(
        "button",
        { name: "AIfred's" },
      ),
    );
    expect(
      screen.queryByRole("region", { name: "Your tasks" }),
    ).not.toBeInTheDocument();
    const group = await within(details()).findByRole("list", {
      name: "Task tree",
    });
    await userEvent.click(
      within(group).getByRole("button", {
        name: "Open details: Checkout totals",
      }),
    );
    expect(screen.getByRole("button", { name: "Both" })).toHaveAttribute(
      "aria-pressed",
      "true",
    );
    expect(
      within(screen.getByRole("region", { name: "Your tasks" })).getByText(
        "Tax work",
      ),
    ).toBeInTheDocument();
    // The saved filter does not change.
    expect(localStorage.getItem("aifred-task-filter")).toBe("aifred");
  });

  it("home opens the Tasks page", async () => {
    await startApp({ tasks: [task("Pay rent")] });
    await userEvent.click(
      screen.getByRole("button", { name: "See all tasks" }),
    );
    expect(screen.getByRole("heading", { name: "Tasks" })).toBeInTheDocument();
  });

  it("Settings shows the brief job, and it can pause", async () => {
    const fake = await startApp({
      jobs: [
        job("brief-on-task-done", "Update the brief when you finish a task", {
          action: "brief",
          trigger: { kind: "event", event: "task_done", wait_secs: 90 },
        }),
      ],
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(
      await screen.findByText("Update the brief when you finish a task"),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", {
        name: "Delete: Update the brief when you finish a task",
      }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", {
        name: "Pause: Update the brief when you finish a task",
      }),
    );
    await waitFor(() => expect(fake.jobs[0].status).toBe("paused"));
  });

  it("shows AIfred's tasks and the latest runs, and pauses and deletes a task", async () => {
    const now = Math.floor(Date.now() / 1000);
    const fake = await startApp({
      jobs: [
        job("j1", "Timesheet reminder"),
        job("j2", "Watch PR #123", {
          action: "agent",
          message: "Tell me when PR #123 is merged.",
          status: "done",
          can_start: false,
        }),
        job("j3", "Daily check", { action: "agent", status: "off" }),
      ],
      jobRuns: [
        {
          id: 2,
          job: "j2",
          planned: now - 60,
          started: now - 60,
          finished: now - 50,
          state: "ok",
          late: true,
          note: "",
          output: "PR #123 is merged.",
          cost_usd: 0.02,
        },
        {
          id: 1,
          job: "j1",
          planned: now - 99,
          started: null,
          finished: null,
          state: "skipped",
          late: false,
          note: "AIfred was not running at this time.",
          output: "",
          cost_usd: null,
        },
      ],
    });
    await openTasks("AIfred's");
    expect(await screen.findByText("Timesheet reminder")).toBeInTheDocument();
    expect(screen.getByText(/Every Monday at 09:00/)).toBeInTheDocument();
    // A task that ended waits under "Ended". Its run shows with its answer.
    expect(
      screen.queryByRole("button", { name: "Start: Watch PR #123" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText("PR #123 is merged.")).toBeInTheDocument();
    expect(screen.getByText(/late, planned/)).toBeInTheDocument();
    expect(
      screen.queryByText("AIfred was not running at this time."),
      "skipped runs are hidden",
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Ended (2)" }));
    // A task with no more times has no Start: it needs a new time first.
    expect(
      screen.queryByRole("button", { name: "Start: Watch PR #123" }),
    ).not.toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "Start: Daily check" }),
    ).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "Pause: Timesheet reminder" }),
    );
    await waitFor(() => expect(fake.jobs[0].status).toBe("paused"));
    expect(
      await screen.findByRole("button", { name: "Start: Timesheet reminder" }),
    ).toBeInTheDocument();
    await userEvent.click(
      screen.getByRole("button", { name: "Delete: Timesheet reminder" }),
    );
    await waitFor(() =>
      expect(screen.queryByText("Timesheet reminder")).not.toBeInTheDocument(),
    );
    expect(fake.jobs.map((j) => j.id)).toEqual(["j2", "j3"]);
  });

  it("says when AIfred has no tasks", async () => {
    await startApp();
    await openTasks("AIfred's");
    expect(
      await screen.findByText("AIfred has no tasks now."),
    ).toBeInTheDocument();
  });

  it("a load error goes away when the next load works", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({ fail: { jobs: "database is locked" } });
      await openTasks("AIfred's");
      expect(
        await screen.findByText(
          "Cannot load AIfred's tasks: database is locked",
        ),
      ).toBeInTheDocument();
      delete fake.fail.jobs;
      await vi.advanceTimersByTimeAsync(30_000);
      await waitFor(() =>
        expect(
          screen.queryByText(/Cannot load AIfred's tasks/),
        ).not.toBeInTheDocument(),
      );
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("errors the window must show", () => {
  it("a task file that cannot load shows the reason, not an empty list", async () => {
    await startApp({ fail: { tasks: "cannot read the database" } });
    expect(
      await screen.findByText(
        "Cannot load your tasks: cannot read the database",
      ),
    ).toBeInTheDocument();
    expect(screen.queryByText(/Nothing to do/)).not.toBeInTheDocument();
  });

  it("a failed start-up load shows the reason", async () => {
    fakeBackend({ fail: { history: "bad history file" } });
    render(<App />);
    expect(
      await screen.findByText("Cannot load the conversation: bad history file"),
    ).toBeInTheDocument();
  });
});

describe("Doc suggestions", () => {
  const ID = "1AbCdEfGhIjKlMnOpQrStUvWxYz0123";
  const LINK = `https://docs.google.com/document/d/${ID}/edit`;
  const docsOn = {
    client_id: "1-a.apps.googleusercontent.com",
    secret_set: true,
    connected: true,
    email: "me@x.com",
    features: [
      ["Calendar", true],
      ["Gmail", true],
      ["Docs suggestions", true],
    ] as [string, boolean][],
    optional: ["Gmail", "Docs suggestions"],
    off: [],
  };

  it("shows the Doc part when Docs suggestions is ticked, and asks to connect again when the login does not allow it", async () => {
    const ticked = {
      ...docsOn,
      features: [
        ["Calendar", true],
        ["Gmail", true],
        ["Docs suggestions", false],
      ] as [string, boolean][],
    };
    await startApp({
      google: ticked,
      tasks: [task("Write the plan", { id: "t2" })],
    });
    await openItem("Write the plan");
    expect(
      await within(details()).findByRole("textbox", { name: "Doc link" }),
    ).toHaveValue("");
    expect(
      await within(details()).findByText(
        /connect Google again in Settings, and allow "Docs suggestions"/,
      ),
    ).toBeInTheDocument();
  });

  it("shows the changes before and after, saves only the kept ones as suggestions, and logs them with an undo", async () => {
    const fake = await startApp({
      google: docsOn,
      tasks: [task("Send the offer", { id: "t1" })],
      docLinks: { "task:t1": LINK },
      onSuggestDoc: (_k, _l, note) => ({
        edits: [
          {
            find: "Price: 10 dollars.",
            replace: `Price: ${note} dollars.`,
            why: "new price",
          },
          { find: "Old line.", replace: "", why: "not true now" },
        ],
      }),
    });
    await openItem("Send the offer");
    // The link comes from the item.
    expect(
      await within(details()).findByRole("textbox", { name: "Doc link" }),
    ).toHaveValue(LINK);
    await userEvent.type(
      within(details()).getByRole("textbox", { name: "What to change" }),
      "12",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest changes" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Doc changes",
    });
    const one = within(box).getByRole("group", { name: "Change 1" });
    expect(within(one).getByLabelText("Before")).toHaveTextContent(
      "Price: 10 dollars.",
    );
    const after = within(one).getByRole("textbox", { name: "After" });
    expect(after).toHaveValue("Price: 12 dollars.");
    await userEvent.clear(after);
    await userEvent.type(after, "Price: 12 dollars, tax included.");
    // Take out the second change.
    await userEvent.click(
      within(within(box).getByRole("group", { name: "Change 2" })).getByRole(
        "checkbox",
      ),
    );
    // Nothing is in the Doc before the click.
    expect(fake.actions).toEqual([]);
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as suggestions" }),
    );
    expect(
      await screen.findByText(/Saved the changes as suggestions in the Doc/),
    ).toBeInTheDocument();
    expect(fake.savedDocEdits).toEqual([
      {
        find: "Price: 10 dollars.",
        replace: "Price: 12 dollars, tax included.",
        why: "new price",
      },
    ]);
    const log = await within(details()).findByRole("list", {
      name: "AIfred's suggestions on this item",
    });
    expect(
      within(log).getByText('Suggested 1 change in the Doc "Offer"'),
    ).toBeInTheDocument();
    expect(
      within(log).getByRole("link", { name: "Open in the Doc ↗" }),
    ).toHaveAttribute("href", LINK);
    await userEvent.click(within(log).getByRole("button", { name: "Undo" }));
    expect(await within(log).findByText(/^Undone/)).toBeInTheDocument();
    expect(fake.actions[0].undone_at).not.toBeNull();
  });

  it("cannot read the Doc before you turn on Docs suggestions, and hides the section on items with no Doc", async () => {
    const off = {
      ...docsOn,
      features: [
        ["Calendar", true],
        ["Gmail", true],
        ["Docs suggestions", false],
      ] as [string, boolean][],
      off: ["Docs suggestions"],
    };
    const fake = await startApp({
      google: off,
      tasks: [
        task("Send the offer", { id: "t1" }),
        task("Water the plants", { id: "t2" }),
      ],
      docLinks: { "task:t1": LINK },
      onSuggestDoc: () => ({ edits: [{ find: "A.", replace: "B.", why: "" }] }),
    });
    await openItem("Water the plants");
    expect(
      within(details()).queryByRole("button", { name: "✦ Suggest changes" }),
    ).not.toBeInTheDocument();
    await openItem("Send the offer");
    await userEvent.click(
      await within(details()).findByRole("button", {
        name: "✦ Suggest changes",
      }),
    );
    expect(
      await within(details()).findByText(
        /AIfred cannot use Google Docs yet\. In Settings, tick "Use Docs suggestions"/,
      ),
    ).toBeInTheDocument();
    expect(
      within(details()).queryByRole("group", { name: "Doc changes" }),
    ).not.toBeInTheDocument();
    expect(fake.actions).toEqual([]);
  });

  it("keeps a link that you typed before AIfred found the link of the item", async () => {
    let answer = () => {};
    const held = new Promise<void>((r) => (answer = r));
    await startApp({
      google: docsOn,
      tasks: [task("Send the offer", { id: "t1" })],
      docLinks: { "task:t1": LINK },
      beforeDocLink: () => held,
    });
    await openItem("Send the offer");
    const mine = `https://docs.google.com/document/d/${"Z".repeat(30)}/edit`;
    const box = await within(details()).findByRole("textbox", {
      name: "Doc link",
    });
    await userEvent.type(box, mine);
    answer();
    await new Promise((r) => setTimeout(r, 20));
    expect(box).toHaveValue(mine);
  });

  it("keeps the link and the note when you mark the item done and not done", async () => {
    await startApp({
      google: docsOn,
      tasks: [task("Send the offer", { id: "t1" })],
    });
    await openItem("Send the offer");
    const mine = `https://docs.google.com/document/d/${"Z".repeat(30)}/edit`;
    await userEvent.type(
      await within(details()).findByRole("textbox", { name: "Doc link" }),
      mine,
    );
    await userEvent.type(
      within(details()).getByRole("textbox", { name: "What to change" }),
      "make it 12",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "Mark as finished" }),
    );
    await userEvent.click(
      await within(details()).findByRole("button", { name: "✓ Finished" }),
    );
    expect(
      await within(details()).findByRole("textbox", { name: "Doc link" }),
    ).toHaveValue(mine);
    expect(
      within(details()).getByRole("textbox", { name: "What to change" }),
    ).toHaveValue("make it 12");
  });

  it("warns when you can only comment on the Doc", async () => {
    await startApp({
      google: docsOn,
      tasks: [task("Send the offer", { id: "t1" })],
      docLinks: { "task:t1": LINK },
      onSuggestDoc: () => ({
        edits: [{ find: "A.", replace: "B.", why: "" }],
        canEdit: false,
      }),
    });
    await openItem("Send the offer");
    await within(details()).findByDisplayValue(LINK);
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest changes" }),
    );
    expect(
      await within(details()).findByText(/You can only comment on this Doc/),
    ).toBeInTheDocument();
  });

  it("shows an error for a link that is not a Google Doc", async () => {
    await startApp({
      google: docsOn,
      tasks: [task("Send the offer", { id: "t1" })],
    });
    await openItem("Send the offer");
    await userEvent.type(
      await within(details()).findByRole("textbox", { name: "Doc link" }),
      "https://example.com/x",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest changes" }),
    );
    expect(
      await within(details()).findByText(
        /Cannot suggest changes: this is not the link of a Google Doc/,
      ),
    ).toBeInTheDocument();
  });
});

describe("Sheet comments", () => {
  const ID = "1AbCdEfGhIjKlMnOpQrStUvWxYz0123";
  const LINK = `https://docs.google.com/spreadsheets/d/${ID}/edit#gid=77`;
  const sheetsOn = {
    client_id: "1-a.apps.googleusercontent.com",
    secret_set: true,
    connected: true,
    email: "me@x.com",
    features: [
      ["Calendar", true],
      ["Gmail", true],
      ["Sheets comments", true],
    ] as [string, boolean][],
    optional: ["Gmail", "Sheets comments"],
    off: [],
  };

  it("shows each comment with its cell, saves only the kept ones, and logs them with an undo", async () => {
    const fake = await startApp({
      google: sheetsOn,
      tasks: [task("Fix the prices", { id: "t1" })],
      sheetLinks: { "task:t1": LINK },
      onSuggestSheet: (_k, _l, note) => ({
        comments: [
          {
            cell: "B3",
            text: `Change to ${note}: the new price.`,
            value: "$10.00",
          },
          { cell: "C3", text: "Add the date: [date].", value: "" },
        ],
        message:
          "AIfred left out 1 comment: the AI named a cell outside the part of the Sheet that it got.",
      }),
    });
    await openItem("Fix the prices");
    expect(
      await within(details()).findByRole("textbox", { name: "Sheet link" }),
    ).toHaveValue(LINK);
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "What to change in the Sheet",
      }),
      "$12.00",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest comments" }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Sheet comments",
    });
    expect(within(box).getByText(/left out 1 comment/)).toBeInTheDocument();
    expect(
      within(box).getByRole("link", { name: "Prices, Q4 ↗" }),
    ).toHaveAttribute("href", LINK);
    const b3 = within(box).getByRole("group", { name: "Comment on B3" });
    expect(within(b3).getByLabelText("Cell now")).toHaveTextContent(
      "Now: $10.00",
    );
    expect(
      within(
        within(box).getByRole("group", { name: "Comment on C3" }),
      ).getByLabelText("Cell now"),
    ).toHaveTextContent("Now: (empty)");
    const text = within(b3).getByRole("textbox", { name: "Comment" });
    expect(text).toHaveValue("Change to $12.00: the new price.");
    await userEvent.clear(text);
    await userEvent.type(text, "Change to $12.00, tax included.");
    await userEvent.click(
      within(
        within(box).getByRole("group", { name: "Comment on C3" }),
      ).getByRole("checkbox"),
    );
    // Nothing is in the Sheet before the click.
    expect(fake.actions).toEqual([]);
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as comments" }),
    );
    expect(
      await screen.findByText(
        /Commented on 1 cell in the Sheet "Prices"\. No cell changed/,
      ),
    ).toBeInTheDocument();
    expect(fake.savedSheetComments).toEqual([
      { cell: "B3", text: "Change to $12.00, tax included.", value: "$10.00" },
    ]);
    const log = await within(details()).findByRole("list", {
      name: "AIfred's comments on this item",
    });
    expect(
      within(log).getByText('Commented on 1 cell in the Sheet "Prices"'),
    ).toBeInTheDocument();
    expect(
      within(log).getByRole("link", { name: "Open in the Sheet ↗" }),
    ).toHaveAttribute("href", LINK);
    await userEvent.click(within(log).getByRole("button", { name: "Undo" }));
    expect(await within(log).findByText(/^Undone/)).toBeInTheDocument();
    expect(fake.actions[0].undone_at).not.toBeNull();
  });

  it("finds the Doc link and the Sheet link of an item with one call", async () => {
    const DOC =
      "https://docs.google.com/document/d/1AbCdEfGhIjKlMnOpQrStUvWxYz9999/edit";
    const both = {
      ...sheetsOn,
      features: [...sheetsOn.features, ["Docs suggestions", true]] as [
        string,
        boolean,
      ][],
      optional: [...sheetsOn.optional, "Docs suggestions"],
    };
    const fake = await startApp({
      google: both,
      tasks: [task("Fix the prices", { id: "t1" })],
      docLinks: { "task:t1": DOC },
      sheetLinks: { "task:t1": LINK },
    });
    await openItem("Fix the prices");
    expect(
      await within(details()).findByRole("textbox", { name: "Sheet link" }),
    ).toHaveValue(LINK);
    expect(
      await within(details()).findByRole("textbox", { name: "Doc link" }),
    ).toHaveValue(DOC);
    expect(fake.linkCalls).toBe(1);
  });

  it("offers no Undo when Google gave no ids for the saved comments", async () => {
    const fake = await startApp({
      google: sheetsOn,
      tasks: [task("Fix the prices", { id: "t1" })],
      sheetLinks: { "task:t1": LINK },
      sheetNoIds: true,
      onSuggestSheet: () => ({
        comments: [{ cell: "B3", text: "Change to $12.00.", value: "$10.00" }],
      }),
    });
    await openItem("Fix the prices");
    await userEvent.click(
      await within(details()).findByRole("button", {
        name: "✦ Suggest comments",
      }),
    );
    const box = await within(details()).findByRole("group", {
      name: "Sheet comments",
    });
    await userEvent.click(
      within(box).getByRole("button", { name: "Save as comments" }),
    );
    expect(
      await screen.findByText(
        /Google gave no ids, so Undo cannot remove them\. No cell changed/,
      ),
    ).toBeInTheDocument();
    await within(details()).findByRole("list", {
      name: "AIfred's comments on this item",
    });
    expect(
      screen.queryByRole("button", { name: "Undo" }),
    ).not.toBeInTheDocument();
    expect(fake.undoCalls ?? 0).toBe(0);
  });

  it("cannot read the Sheet before you turn on Sheets comments, and hides the part on items with no Sheet", async () => {
    const off = {
      ...sheetsOn,
      features: [
        ["Calendar", true],
        ["Gmail", true],
        ["Sheets comments", false],
      ] as [string, boolean][],
      off: ["Sheets comments"],
    };
    await startApp({
      google: off,
      tasks: [
        task("Fix the prices", { id: "t1" }),
        task("Call Ana", { id: "t2" }),
      ],
      sheetLinks: { "task:t1": LINK },
    });
    await openItem("Fix the prices");
    await userEvent.click(
      await within(details()).findByRole("button", {
        name: "✦ Suggest comments",
      }),
    );
    expect(
      await within(details()).findByText(
        /AIfred cannot use Google Sheets yet\. In Settings, tick "Use Sheets comments"/,
      ),
    ).toBeInTheDocument();
    expect(
      within(details()).queryByRole("group", { name: "Sheet comments" }),
    ).not.toBeInTheDocument();
    await openItem("Call Ana");
    await waitFor(() =>
      expect(
        within(details()).queryByRole("textbox", { name: "Sheet link" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("shows an error for a link that is not a Google Sheet", async () => {
    await startApp({
      google: sheetsOn,
      tasks: [task("Fix the prices", { id: "t1" })],
    });
    await openItem("Fix the prices");
    await userEvent.type(
      await within(details()).findByRole("textbox", { name: "Sheet link" }),
      "https://example.com/x",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "✦ Suggest comments" }),
    );
    expect(
      await within(details()).findByText(
        /Cannot suggest comments: this is not the link of a Google Sheet/,
      ),
    ).toBeInTheDocument();
  });
});

describe("task context", () => {
  it("asks about one item at a time, and you talk with AIfred about it in the card", async () => {
    const sync = task("Fix sync");
    const call = task("Call Bo");
    const fake = await startApp({
      tasks: [sync, call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: [`task:${sync.id}`, `task:${call.id}`],
        },
      },
    });
    // The first answer is not enough: AIfred asks one more question. The
    // second answer is: AIfred saves a short summary (set_item_context).
    fake.onAskSide = async (p) => {
      if (p === "the sync bug")
        return answer("Is it part of the release on Friday?");
      fake.contexts[`task:${sync.id}`] = {
        key: `task:${sync.id}`,
        text: "The ClickUp sync bug. Due Friday.",
        by: "aifred",
        updated: 5,
      };
      return answer("Saved: the sync bug blocks the release on Friday.");
    };
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    expect(within(card).getByText(/1 of 2/)).toBeInTheDocument();
    expect(within(card).getByText("What is this item?")).toBeInTheDocument();
    expect(
      within(card).queryByRole("textbox", {
        name: "Your answer about Call Bo",
      }),
    ).not.toBeInTheDocument();
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Your answer about Fix sync" }),
      "the sync bug{Enter}",
    );
    // The talk stays in the card: no panel opens.
    expect(
      await within(card).findByText("Is it part of the release on Friday?"),
    ).toBeInTheDocument();
    expect(within(card).getByText("the sync bug")).toBeInTheDocument();
    expect(
      screen.queryByRole("dialog", { name: "Task details" }),
    ).not.toBeInTheDocument();
    // The title opens the item: its chat has the same talk.
    await userEvent.click(
      within(card).getByRole("button", { name: "Fix sync" }),
    );
    expect(
      await within(details()).findByText(
        "Is it part of the release on Friday?",
      ),
    ).toBeInTheDocument();
    await userEvent.keyboard("{Escape}");
    await waitFor(() =>
      expect(
        screen.queryByRole("dialog", { name: "Task details" }),
      ).not.toBeInTheDocument(),
    );
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Your answer about Fix sync" }),
      "yes, it blocks it{Enter}",
    );
    // AIfred saved the notes: its answer stays on the screen until you go on.
    expect(
      await within(card).findByText(
        "Saved: the sync bug blocks the release on Friday.",
      ),
    ).toBeInTheDocument();
    expect(await within(card).findByRole("status")).toHaveTextContent(
      'AIfred saved what it knows about "Fix sync".',
    );
    expect(within(card).getByText(/1 more question/)).toBeInTheDocument();
    expect(
      within(card).getByRole("textbox", { name: "Your answer about Fix sync" }),
    ).toBeInTheDocument();
    expect(
      within(card).queryByRole("button", { name: "Skip" }),
    ).not.toBeInTheDocument();
    await userEvent.click(
      within(card).getByRole("button", { name: "Next question" }),
    );
    expect(
      await within(card).findByRole("textbox", {
        name: "Your answer about Call Bo",
      }),
    ).toBeInTheDocument();
    // The count still counts the answered question.
    expect(within(card).getByText(/2 of 2/)).toBeInTheDocument();
    expect(
      within(card).queryByRole("textbox", {
        name: "Your answer about Fix sync",
      }),
    ).not.toBeInTheDocument();
    expect(fake.side[`item:task:${sync.id}`]?.map((t) => t.text)).toEqual([
      "the sync bug",
      "Is it part of the release on Friday?",
      "yes, it blocks it",
      "Saved: the sync bug blocks the release on Friday.",
    ]);

    // Skip: AIfred does not ask again. You answered a question, so AIfred
    // writes the brief and missions again with the new notes.
    await userEvent.click(within(card).getByRole("button", { name: "Skip" }));
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Help AIfred understand" }),
      ).not.toBeInTheDocument(),
    );
    expect(fake.contextSkip).toEqual([`task:${call.id}`]);
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1),
    );
  });

  it("after the answers, AIfred writes the brief and missions again", async () => {
    const sync = task("Fix sync");
    const fake = await startApp({
      tasks: [sync],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: [`task:${sync.id}`],
        },
      },
    });
    fake.onAskSide = async () => {
      fake.contexts[`task:${sync.id}`] = {
        key: `task:${sync.id}`,
        text: "The sync bug.",
        by: "aifred",
        updated: 5,
      };
      return answer("Saved.");
    };
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    expect(
      within(card).getByText(
        /It updates the brief and missions after your answers/,
      ),
    ).toBeInTheDocument();
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Your answer about Fix sync" }),
      "the sync bug{Enter}",
    );
    await userEvent.click(
      await within(card).findByRole("button", {
        name: "Update the brief and missions",
      }),
    );
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1),
    );
    expect(
      await screen.findByRole("region", { name: "Brief" }),
    ).toBeInTheDocument();
  });

  it("a skip alone gives no new details: the brief is not written again, and Undo asks again", async () => {
    const call = task("Call Bo");
    const fake = await startApp({
      tasks: [call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: [`task:${call.id}`],
        },
      },
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    await userEvent.click(within(card).getByRole("button", { name: "Skip" }));
    expect(
      await screen.findByRole("region", { name: "Brief" }),
    ).toBeInTheDocument();
    expect(fake.contextSkip).toEqual([`task:${call.id}`]);
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.contextSkip).toEqual([]));
    expect(
      await screen.findByRole("textbox", { name: "Your answer about Call Bo" }),
    ).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(0);
  });

  it("an answer that says a ClickUp task is finished takes the task away", async () => {
    const sync = task("Fix sync");
    const fake = await startApp({
      clickupToken: "pk_test",
      tasks: [sync],
      clickup: [clickupItem("86a", "echowp")],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: ["clickup:86a", `task:${sync.id}`],
        },
      },
    });
    // Like mark_task_done: ClickUp closes the task, and the saved list loses it.
    fake.onAskSide = async () => {
      fake.clickup = [];
      return answer("Marked echowp done in ClickUp.");
    };
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Your answer about echowp" }),
      "it is already finished{Enter}",
    );
    // AIfred's answer stays, and the card says that the task is done.
    expect(await within(card).findByRole("status")).toHaveTextContent(
      '"echowp" is done.',
    );
    expect(
      within(card).getByText("Marked echowp done in ClickUp."),
    ).toBeInTheDocument();
    expect(
      fake.calls.some((c) => c.cmd === "clickup_items" && !c.args?.force),
    ).toBe(true);
    await userEvent.click(
      within(card).getByRole("button", { name: "Next question" }),
    );
    expect(
      await within(card).findByRole("textbox", {
        name: "Your answer about Fix sync",
      }),
    ).toBeInTheDocument();
  });

  it("a held brief still asks about an item whose list failed to load, with the title from the brief", async () => {
    await startApp({
      github: [githubItem(7, "assigned", "Sync")],
      onGithubItems: () => Promise.reject("GitHub is down"),
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: ["github:acme/shop#7"],
          held: true,
          titles: { "github:acme/shop#7": "Sync the docs" },
        },
      },
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    expect(
      within(card).getByRole("textbox", {
        name: "Your answer about Sync the docs",
      }),
    ).toBeInTheDocument();
  });

  it("a held brief writes again when the card shows none of its questions", async () => {
    // The server still counts the GitHub item as asked, but it is not in
    // the loaded list (closed there). The card shows no question, so the
    // missions must not wait for an answer that you cannot give.
    const fake = await startApp({
      github: [],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: ["github:acme/shop#7"],
          held: true,
        },
      },
    });
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "write_brief")).toBe(true),
    );
    expect(
      screen.queryByRole("region", { name: "Help AIfred understand" }),
    ).not.toBeInTheDocument();
  });

  it("a held brief with no question on the screen is written again only one time on a day", async () => {
    // The server still holds the missions for the question, but the card
    // does not show it. Each new brief holds again: the app must not write
    // the brief again and again, because each write costs AI calls.
    let n = 3;
    const fake = await startApp({
      github: [],
      briefs: {
        [today()]: {
          day: today(),
          written: n,
          text: "One thing.",
          why: {},
          ask: ["github:acme/shop#7"],
          held: true,
        },
      },
      onWriteBrief: (day) => ({
        day,
        written: ++n,
        text: "One thing.",
        why: {},
        ask: ["github:acme/shop#7"],
        held: true,
      }),
    });
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "write_brief")).toBe(true),
    );
    await new Promise((r) => setTimeout(r, 300));
    const writes = fake.calls.filter((c) => c.cmd === "write_brief");
    expect(writes).toHaveLength(1);
    // The write asks the server to stop the hold: else the missions wait
    // for the whole day.
    expect(writes[0].args.release).toBe(true);
  });

  it("a brief that does not hold shows the questions above the brief", async () => {
    const call = task("Call Bo");
    await startApp({
      tasks: [call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: [`task:${call.id}`],
        },
      },
    });
    expect(
      await screen.findByRole("region", { name: "Help AIfred understand" }),
    ).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Brief" })).toBeInTheDocument();
  });

  it("a question that goes with no answer of yours gives no 'used your answer' message", async () => {
    const a = task("Call Bo");
    const b = task("Pay rent");
    const fake = await startApp({
      tasks: [a, b],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: [`task:${a.id}`, `task:${b.id}`],
        },
      },
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    // You write the notes of "Pay rent" elsewhere, then skip "Call Bo".
    fake.contexts[`task:${b.id}`] = {
      key: `task:${b.id}`,
      text: "Rent of the flat.",
      by: "you",
      updated: 5,
    };
    await userEvent.click(within(card).getByRole("button", { name: "Skip" }));
    await waitFor(() => expect(fake.contextSkip).toEqual([`task:${a.id}`]));
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Help AIfred understand" }),
      ).not.toBeInTheDocument(),
    );
    expect(
      screen.queryByText(/AIfred used your answer/),
    ).not.toBeInTheDocument();
  });

  it("one answer can answer the other questions too", async () => {
    const sync = task("Add echowp in WWP");
    const parent = task("echowp");
    const fake = await startApp({
      tasks: [sync, parent],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: [`task:${sync.id}`, `task:${parent.id}`],
          held: true,
        },
      },
    });
    // AIfred saves the notes of both items, and asks one more question.
    fake.onAskSide = async () => {
      for (const t of [sync, parent])
        fake.contexts[`task:${t.id}`] = {
          key: `task:${t.id}`,
          text: "The echowp work.",
          by: "aifred",
          updated: 5,
        };
      return answer("Saved notes for both. Is there a deadline?");
    };
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    await userEvent.type(
      within(card).getByRole("textbox", {
        name: "Your answer about Add echowp in WWP",
      }),
      "part of the echowp work{Enter}",
    );
    expect(
      await screen.findByText(
        'AIfred used your answer for "echowp" too, so it does not ask about it.',
      ),
    ).toBeInTheDocument();
    // AIfred's question stays: you can answer it before you go on.
    expect(
      within(card).getByText("Saved notes for both. Is there a deadline?"),
    ).toBeInTheDocument();
    expect(within(card).getByText(/last question/)).toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(0);
    await userEvent.click(
      within(card).getByRole("button", { name: "Make today's missions" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Help AIfred understand" }),
      ).not.toBeInTheDocument(),
    );
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1),
    );
  });

  it("the card shows AIfred's own first question about each item, and keeps it during the talk", async () => {
    const sync = task("Fix sync");
    const call = task("Call Bo");
    const fake = await startApp({
      tasks: [sync, call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: [`task:${sync.id}`, `task:${call.id}`],
          questions: {
            [`task:${sync.id}`]: [
              "Is the sync bug part of the Friday release?",
              "Who tests it?",
            ],
          },
        },
      },
    });
    // AIfred takes the answered question out of the list, and asks the next one.
    fake.onAskSide = async () => {
      fake.briefs[today()]!.questions = {
        [`task:${sync.id}`]: ["Who tests it?"],
      };
      return answer("Who tests it?");
    };
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    expect(
      within(card).getByText("Is the sync bug part of the Friday release?"),
    ).toBeInTheDocument();
    expect(
      within(card).queryByText("What is this item?"),
    ).not.toBeInTheDocument();
    await userEvent.type(
      within(card).getByRole("textbox", { name: "Your answer about Fix sync" }),
      "yes{Enter}",
    );
    expect(await within(card).findByText("Who tests it?")).toBeInTheDocument();
    // The first question stays at the top of the talk.
    expect(
      within(card).getByText("Is the sync bug part of the Friday release?"),
    ).toBeInTheDocument();
    // An item with no list gets the general question.
    await userEvent.click(within(card).getByRole("button", { name: "Later" }));
    expect(
      await within(card).findByText("What is this item?"),
    ).toBeInTheDocument();
  });

  it("Later shows the next question first", async () => {
    const sync = task("Fix sync");
    const call = task("Call Bo");
    await startApp({
      tasks: [sync, call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: [`task:${sync.id}`, `task:${call.id}`],
        },
      },
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    await userEvent.click(within(card).getByRole("button", { name: "Later" }));
    expect(
      within(card).getByRole("textbox", { name: "Your answer about Call Bo" }),
    ).toBeInTheDocument();
    await userEvent.click(within(card).getByRole("button", { name: "Later" }));
    expect(
      within(card).getByRole("textbox", { name: "Your answer about Fix sync" }),
    ).toBeInTheDocument();
  });

  it("while AIfred asks, the home page shows only the questions; after the last one, the brief makes its missions", async () => {
    const sync = task("Fix sync");
    const post = task("Write the post");
    const old: Mission = {
      id: 5,
      title: "Ship the launch post",
      doer: "you",
      day: "2020-01-01",
      rank: 1,
      status: "open",
      items: [
        {
          key: `task:${post.id}`,
          title: "Write the post",
          open: true,
          by: "aifred",
        },
      ],
      checklist: [],
      progress: {
        steps_done: 0,
        steps: 0,
        items_closed: 0,
        items: 1,
        percent: 0,
      },
    };
    const call = task("Call Bo");
    const mine: Mission = {
      ...old,
      id: 6,
      title: "Call Bo back",
      items: [
        { key: `task:${call.id}`, title: "Call Bo", open: true, by: "you" },
      ],
    };
    const fake = await startApp({
      tasks: [sync, post, call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: [`task:${sync.id}`],
          held: true,
          missions: [old, mine],
        },
      },
      onWriteBrief: (day) => ({
        day,
        written: 4,
        text: "Now with missions.",
        why: {},
      }),
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    expect(
      within(card).getByText(/It makes today's missions after your answers\./),
    ).toBeInTheDocument();
    // The brief, AIfred's missions, the other cards, and the chat box wait for the answers.
    expect(screen.queryByText("Ship the launch post")).not.toBeInTheDocument();
    // A mission with an item that you put there yourself stays.
    expect(screen.getByText("Call Bo back")).toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Brief" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("textbox", { name: "Add a task or ask AIfred" }),
    ).not.toBeInTheDocument();
    expect(
      screen.queryByRole("region", { name: "Waiting for" }),
    ).not.toBeInTheDocument();
    expect(screen.queryByText("One thing.")).not.toBeInTheDocument();
    expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(0);
    await userEvent.click(within(card).getByRole("button", { name: "Skip" }));
    await waitFor(() =>
      expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1),
    );
    expect(await screen.findByText("Now with missions.")).toBeInTheDocument();
    expect(screen.getByRole("region", { name: "Brief" })).toBeInTheDocument();
    expect(homeBox()).toBeInTheDocument();
    await new Promise((r) => setTimeout(r, 50));
    expect(fake.calls.filter((c) => c.cmd === "write_brief")).toHaveLength(1);
  });

  it("a failed answer stays in its box on the home page, so you can send it again", async () => {
    const sync = task("Fix sync");
    const fake = await startApp({
      tasks: [sync],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: [`task:${sync.id}`],
        },
      },
    });
    fake.onAskSide = async () => {
      throw "the AI is busy";
    };
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    const box = within(card).getByRole("textbox", {
      name: "Your answer about Fix sync",
    });
    await userEvent.type(box, "the sync bug{Enter}");
    expect(await within(card).findByText(/the AI is busy/)).toBeInTheDocument();
    await waitFor(() => expect(box).toHaveValue("the sync bug"));
  });

  it("notes that you save in the item panel end the question on the home page", async () => {
    const sync = task("Fix sync");
    await startApp({
      tasks: [sync],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "One thing.",
          why: {},
          ask: [`task:${sync.id}`],
        },
      },
    });
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    // The title in the card opens the item.
    await userEvent.click(
      within(card).getByRole("button", { name: "Fix sync" }),
    );
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    await userEvent.type(box, "The sync bug. Due Friday.");
    await userEvent.click(
      within(details()).getByRole("button", { name: "Save" }),
    );
    await waitFor(() =>
      expect(
        screen.queryByRole("region", { name: "Help AIfred understand" }),
      ).not.toBeInTheDocument(),
    );
  });

  it("the undo of a skip asks again, also when the brief loaded again after the skip", async () => {
    const sync = task("Fix sync");
    const call = task("Call Bo");
    const fake = await startApp({
      tasks: [sync, call],
      briefs: {
        [today()]: {
          day: today(),
          written: 3,
          text: "Two things.",
          why: {},
          ask: [`task:${call.id}`],
        },
      },
    });
    fake.onAskSide = () => answer("Noted.");
    const card = await screen.findByRole("region", {
      name: "Help AIfred understand",
    });
    await userEvent.click(within(card).getByRole("button", { name: "Skip" }));
    await waitFor(() => expect(fake.contextSkip).toEqual([`task:${call.id}`]));
    // A chat answer loads the brief again: the skipped item is not in it.
    const loads = fake.calls.filter((c) => c.cmd === "brief").length;
    await openItem("Fix sync");
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "hi{Enter}",
    );
    await within(details()).findByText("Noted.");
    await waitFor(() =>
      expect(
        fake.calls.filter((c) => c.cmd === "brief").length,
      ).toBeGreaterThan(loads),
    );
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    expect(
      await screen.findByRole("textbox", { name: "Your answer about Call Bo" }),
    ).toBeInTheDocument();
  });

  it("a chat answer does not replace notes that you changed and did not save", async () => {
    const t = task("Fix sync");
    const key = `task:${t.id}`;
    const fake = await startApp({
      tasks: [t],
      contexts: { [key]: { key, text: "A bug.", by: "you", updated: 5 } },
    });
    fake.onAskSide = () => answer("Noted.");
    await openItem("Fix sync");
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    await waitFor(() => expect(box).toHaveValue("A bug."));
    await userEvent.type(box, " Due Friday.");
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "hi{Enter}",
    );
    await within(details()).findByText("Noted.");
    await new Promise((r) => setTimeout(r, 50));
    expect(box).toHaveValue("A bug. Due Friday.");
  });

  it("the undo of AIfred's notes from the chat puts back the notes from before", async () => {
    const t = task("Fix sync");
    const key = `task:${t.id}`;
    const fake = await startApp({
      tasks: [t],
      contexts: { [key]: { key, text: "A bug.", by: "you", updated: 5 } },
    });
    fake.onAskSide = () => {
      fake.contexts[key] = {
        key,
        text: "The sync bug. Due Friday.",
        by: "aifred",
        updated: 6,
      };
      return answer("Saved.");
    };
    await openItem("Fix sync");
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    await waitFor(() => expect(box).toHaveValue("A bug."));
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "it is due Friday{Enter}",
    );
    await waitFor(() => expect(box).toHaveValue("The sync bug. Due Friday."));
    await screen.findByText("AIfred saved what it knows about this item.");
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.contexts[key]?.text).toBe("A bug."));
    await waitFor(() => expect(box).toHaveValue("A bug."));
  });

  it("AIfred's clear of the notes in the chat shows a message with an undo", async () => {
    const t = task("Fix sync");
    const key = `task:${t.id}`;
    const fake = await startApp({
      tasks: [t],
      contexts: { [key]: { key, text: "A bug.", by: "you", updated: 5 } },
    });
    fake.onAskSide = () => {
      delete fake.contexts[key];
      return answer("Cleared.");
    };
    await openItem("Fix sync");
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    await waitFor(() => expect(box).toHaveValue("A bug."));
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "forget it{Enter}",
    );
    await screen.findByText("AIfred cleared what it knows about this item.");
    expect(box).toHaveValue("");
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.contexts[key]?.text).toBe("A bug."));
    await waitFor(() => expect(box).toHaveValue("A bug."));
  });

  it("the undo of AIfred's notes keeps text that you typed and did not save", async () => {
    const t = task("Fix sync");
    const key = `task:${t.id}`;
    const fake = await startApp({
      tasks: [t],
      contexts: { [key]: { key, text: "A bug.", by: "you", updated: 5 } },
    });
    fake.onAskSide = () => {
      fake.contexts[key] = {
        key,
        text: "The sync bug.",
        by: "aifred",
        updated: 6,
      };
      return answer("Saved.");
    };
    await openItem("Fix sync");
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    await waitFor(() => expect(box).toHaveValue("A bug."));
    await userEvent.type(box, " Due Friday.");
    await userEvent.type(
      within(details()).getByRole("textbox", {
        name: "Ask AIfred about this item",
      }),
      "hi{Enter}",
    );
    await screen.findByText("AIfred saved what it knows about this item.");
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.contexts[key]?.text).toBe("A bug."));
    await new Promise((r) => setTimeout(r, 50));
    expect(box).toHaveValue("A bug. Due Friday.");
  });

  it("the notes box of one item does not go to the next item", async () => {
    const a = task("Fix sync");
    const b = task("Call Bo");
    const fake = await startApp({
      tasks: [a, b],
      contexts: {
        [`task:${b.id}`]: {
          key: `task:${b.id}`,
          text: "About Bo.",
          by: "you",
          updated: 5,
        },
      },
    });
    await openItem("Fix sync");
    await userEvent.type(
      await within(details()).findByRole("textbox", {
        name: "What AIfred knows",
      }),
      "Text for sync.",
    );
    // The notes of the next item load slowly: until then, the box must not keep the text.
    const hold = deferred<void>();
    fake.onItemContext = (k) =>
      k === `task:${b.id}` ? hold.promise : undefined;
    await openItem("Call Bo");
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    expect(box).toHaveValue("");
    expect(
      within(details()).getByRole("button", { name: "Save" }),
    ).toBeDisabled();
    hold.resolve();
    await waitFor(() => expect(box).toHaveValue("About Bo."));
    expect(
      within(details()).getByRole("button", { name: "Save" }),
    ).toBeDisabled();
    expect(fake.contexts[`task:${a.id}`]).toBeUndefined();
  });

  it("you can change and clear what AIfred knows, with Undo", async () => {
    const t = task("Fix sync");
    const key = `task:${t.id}`;
    const fake = await startApp({
      tasks: [t],
      contexts: {
        [key]: { key, text: "The sync bug.", by: "aifred", updated: 5 },
      },
    });
    await openItem("Fix sync");
    const box = await within(details()).findByRole("textbox", {
      name: "What AIfred knows",
    });
    await waitFor(() => expect(box).toHaveValue("The sync bug."));
    expect(
      within(details()).getByText(/Written by AIfred/),
    ).toBeInTheDocument();
    await userEvent.clear(box);
    await userEvent.type(box, "The sync bug. Due Friday.");
    await userEvent.click(
      within(details()).getByRole("button", { name: "Save" }),
    );
    await waitFor(() =>
      expect(fake.contexts[key]).toMatchObject({
        text: "The sync bug. Due Friday.",
        by: "you",
      }),
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "Clear" }),
    );
    await waitFor(() => expect(fake.contexts[key]).toBeUndefined());
    expect(box).toHaveValue("");
    await userEvent.click(
      (await screen.findAllByRole("button", { name: "Undo" })).at(-1)!,
    );
    await waitFor(() =>
      expect(fake.contexts[key]?.text).toBe("The sync bug. Due Friday."),
    );
    await waitFor(() => expect(box).toHaveValue("The sync bug. Due Friday."));
  });
});

describe("AIfred status", () => {
  const connected = { clickupToken: "pk_test" };

  it("a status in the panel changes only AIfred, and Undo puts it back", async () => {
    const fake = await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
    });
    await openItem("Fix cart");
    await userEvent.selectOptions(
      within(details()).getByLabelText("Status"),
      "cancelled",
    );
    await waitFor(() =>
      expect(fake.statuses["github:acme/shop#7"]).toEqual({
        status: "cancelled",
        by: "you",
        source: undefined,
      }),
    );
    // Cancelled is closed: the item leaves the open list.
    await waitFor(() =>
      expect(
        screen.queryByRole("button", { name: "Open details: Fix cart" }),
      ).not.toBeInTheDocument(),
    );
    const toast = await screen.findByText("Fix cart: Cancelled");
    await userEvent.click(
      within(toast.closest(".toast") as HTMLElement).getByRole("button", {
        name: "Undo",
      }),
    );
    await waitFor(() =>
      expect(fake.statuses["github:acme/shop#7"]).toBeUndefined(),
    );
    expect(
      await screen.findByRole("button", { name: "Open details: Fix cart" }),
    ).toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "clickup_mark_done")).toBe(false);
  });

  it("a ClickUp task that AIfred counts as Unfinished is not open, and a row shows AIfred's status, not the ClickUp name", async () => {
    await startApp({
      ...connected,
      clickup: [
        clickupItem("r1", "Old rock", { status: "delayed" }),
        clickupItem("86a", "Write notes", { status: "review" }),
      ],
      statuses: {
        "clickup:r1": { status: "unfinished", by: "rule", source: "delayed" },
        "clickup:86a": { status: "progress", by: "default", source: "review" },
      },
    });
    const open = await screen.findByRole("list", { name: "Open" });
    expect(await within(open).findByText("Write notes")).toBeInTheDocument();
    expect(within(open).queryByText("Old rock")).not.toBeInTheDocument();
    const pill = within(open).getByText("In progress");
    expect(pill).toHaveAttribute(
      "title",
      expect.stringContaining("ClickUp status: review"),
    );
    expect(within(open).queryByText("review")).not.toBeInTheDocument();
  });

  it("Done of a ClickUp task in the status menu closes it in ClickUp", async () => {
    const fake = await startApp({
      ...connected,
      clickup: [clickupItem("86a", "Write notes")],
    });
    await openItem("Write notes");
    await userEvent.selectOptions(
      within(details()).getByLabelText("Status"),
      "done",
    );
    await waitFor(() =>
      expect(
        fake.calls.find((c) => c.cmd === "clickup_mark_done")?.args,
      ).toEqual({ id: "86a" }),
    );
    expect(fake.calls.some((c) => c.cmd === "set_item_status")).toBe(false);
  });

  it("Unfinished closes a local task, and the status menu shows it", async () => {
    const fake = await startApp({ tasks: [task("Plan Q4")] });
    await openItem("Plan Q4");
    await userEvent.selectOptions(
      within(details()).getByLabelText("Status"),
      "unfinished",
    );
    await waitFor(() =>
      expect(fake.tasks[0]).toMatchObject({
        done: true,
        closed_as: "unfinished",
      }),
    );
    await waitFor(() =>
      expect(within(details()).getByLabelText("Status")).toHaveValue(
        "unfinished",
      ),
    );
    expect(
      within(details()).getByText(/Accepted as it is/),
    ).toBeInTheDocument();
  });

  it("an Unfinished task links to the task that continues it, with an undo", async () => {
    const fake = await startApp({
      tasks: [
        task("Rock Q3", { id: "t1", done: true, closed_as: "unfinished" }),
        task("Rock Q4", { id: "t2" }),
      ],
    });
    await userEvent.click(
      await screen.findByRole("button", { name: /Show finished/ }),
    );
    await openItem("Rock Q3");
    await userEvent.click(
      await within(details()).findByRole("button", {
        name: "+ Link the task that continues it",
      }),
    );
    await userEvent.type(
      within(details()).getByLabelText("Find the task that continues it"),
      "Q4",
    );
    await userEvent.click(
      within(details()).getByRole("button", { name: "Continued in: Rock Q4" }),
    );
    await waitFor(() =>
      expect(fake.statusNext).toEqual({ "task:t1": "task:t2" }),
    );
    const linked = await within(details()).findByRole("list", {
      name: "Continued in",
    });
    expect(
      within(linked).getByRole("button", { name: "Open details: Rock Q4" }),
    ).toBeInTheDocument();
    const toast = await screen.findByText("Rock Q3: continued in Rock Q4");
    await userEvent.click(
      within(toast.closest(".toast") as HTMLElement).getByRole("button", {
        name: "Undo",
      }),
    );
    await waitFor(() => expect(fake.statusNext).toEqual({}));
    expect(
      await within(details()).findByText(/Accepted as it is/),
    ).toBeInTheDocument();
  });

  it("the task that continues an Unfinished task shows it", async () => {
    await startApp({
      tasks: [
        task("Rock Q3", { id: "t1", done: true, closed_as: "unfinished" }),
        task("Rock Q4", { id: "t2" }),
      ],
      statusNext: { "task:t1": "task:t2" },
    });
    await openItem("Rock Q4");
    const list = await within(details()).findByRole("list", {
      name: "Continues",
    });
    expect(
      within(list).getByRole("button", { name: "Open details: Rock Q3" }),
    ).toBeInTheDocument();
  });

  it("Settings: the ClickUp statuses tab shows the group defaults, and adds an override with an undo", async () => {
    const fake = await startApp({
      ...connected,
      clickup: [
        clickupItem("r1", "Old rock", {
          status: "delayed",
          list_id: "l1",
          space_id: "s1",
        }),
      ],
      cuSpaces: [{ id: "s1", name: "Company Wide", workspace: "Rymera" }],
      cuSpaceStatuses: {
        s1: {
          statuses: [{ name: "Open", kind: "open", color: "#ccc" }],
          lists: [
            {
              id: "l1",
              name: "Rocks (Q4 2026)",
              folder: "",
              statuses: [
                { name: "planned", kind: "open", color: "#ccc" },
                { name: "delayed", kind: "unstarted", color: "#f80" },
              ],
            },
          ],
        },
      },
    });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "ClickUp statuses" }),
    );
    const defaults = await screen.findByRole("table", { name: "Defaults" });
    expect(
      within(defaults).getByRole("row", { name: "Not started To do" }),
    ).toBeInTheDocument();
    expect(
      within(defaults).getByRole("row", { name: "Active In progress" }),
    ).toBeInTheDocument();
    expect(
      within(defaults).getByRole("row", { name: "Closed Done" }),
    ).toBeInTheDocument();
    expect(await screen.findByText(/No overrides/)).toBeInTheDocument();

    await userEvent.click(
      screen.getByRole("button", { name: "+ Add override" }),
    );
    await userEvent.selectOptions(await screen.findByLabelText("Space"), "s1");
    // "Any list in the space" offers the statuses of the space and of each list in it.
    await waitFor(() =>
      expect(
        screen.getByRole("option", { name: "delayed (Not started)" }),
      ).toBeInTheDocument(),
    );
    expect(
      screen.getByRole("option", { name: "Open (Not started)" }),
    ).toBeInTheDocument();
    await userEvent.selectOptions(screen.getByLabelText("List"), "l1");
    expect(
      screen.queryByRole("option", { name: "Open (Not started)" }),
    ).not.toBeInTheDocument();
    await userEvent.selectOptions(
      screen.getByLabelText("ClickUp status"),
      "delayed",
    );
    // The group default comes first. The user changes it.
    expect(screen.getByLabelText("AIfred status")).toHaveValue("todo");
    await userEvent.selectOptions(
      screen.getByLabelText("AIfred status"),
      "unfinished",
    );
    await userEvent.click(screen.getByRole("button", { name: "Add" }));
    await waitFor(() =>
      expect(fake.statusRules).toEqual([
        {
          id: 1,
          name: "delayed",
          scope: "list",
          scope_id: "l1",
          scope_name: "Rocks (Q4 2026)",
          space_id: "s1",
          space_name: "Company Wide",
          kind: "unstarted",
          status: "unfinished",
        },
      ]),
    );
    const rows = screen.getByRole("table", { name: "Overrides" });
    expect(within(rows).getByText("Company Wide")).toBeInTheDocument();
    expect(within(rows).getByText("Rocks (Q4 2026)")).toBeInTheDocument();
    expect(
      within(rows).getByLabelText(
        "AIfred status for delayed in Rocks (Q4 2026)",
      ),
    ).toHaveValue("unfinished");
    expect(
      await screen.findByText(
        /"delayed" in Rocks \(Q4 2026\) is Unfinished now/,
      ),
    ).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Undo" }));
    await waitFor(() => expect(fake.statusRules).toEqual([]));
  });

  it("Settings: a saved override changes its AIfred status in place, and Remove has an undo", async () => {
    const rule = {
      id: 4,
      name: "delayed",
      scope: "space" as const,
      scope_id: "s1",
      scope_name: "Company Wide",
      space_id: "s1",
      space_name: "Company Wide",
      kind: "unstarted",
      status: "unfinished" as const,
    };
    const fake = await startApp({ ...connected, statusRules: [rule] });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(
      await screen.findByRole("tab", { name: "ClickUp statuses" }),
    );
    const box = await screen.findByLabelText(
      "AIfred status for delayed in Any list in the space",
    );
    await userEvent.selectOptions(box, "cancelled");
    await waitFor(() =>
      expect(fake.statusRules).toMatchObject([{ id: 4, status: "cancelled" }]),
    );
    await userEvent.click(
      screen.getByRole("button", {
        name: "Remove the override for delayed in Any list in the space",
      }),
    );
    await waitFor(() => expect(fake.statusRules).toEqual([]));
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.statusRules).toMatchObject([{ id: 4, status: "cancelled" }]),
    );
  });

  it("Settings: tabs group the settings, keep unsaved changes, and move with the arrow keys", async () => {
    await startApp({});
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const general = await screen.findByRole("tab", { name: "General" });
    expect(general).toHaveAttribute("aria-selected", "true");
    await userEvent.selectOptions(screen.getByLabelText("Lunch length"), "90");
    await userEvent.click(screen.getByRole("tab", { name: "About" }));
    expect(
      screen.queryByRole("button", { name: "Save" }),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByRole("tab", { name: "General" }));
    expect(screen.getByLabelText("Lunch length")).toHaveValue("90");
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
    screen.getByRole("tab", { name: "General" }).focus();
    await userEvent.keyboard("{ArrowRight}");
    expect(screen.getByRole("tab", { name: "Connections" })).toHaveAttribute(
      "aria-selected",
      "true",
    );
    expect(screen.getByRole("tab", { name: "Connections" })).toHaveFocus();
  });
});

describe("AIfred status: closed rows", () => {
  it("the tick of a ClickUp task that is Unfinished in AIfred opens it again in AIfred, not in ClickUp", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("r1", "Old rock", { status: "delayed" })],
      statuses: {
        "clickup:r1": { status: "unfinished", by: "rule", source: "delayed" },
      },
    });
    await userEvent.click(
      await screen.findByRole("button", { name: /Show finished/ }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Open again: Old rock" }),
    );
    await waitFor(() =>
      expect(fake.statuses["clickup:r1"]).toMatchObject({
        status: "todo",
        by: "you",
      }),
    );
    expect(fake.calls.some((c) => c.cmd === "clickup_mark_done")).toBe(false);
    expect(fake.calls.some((c) => c.cmd === "open_item_again")).toBe(true);
  });

  it("the tick of a ClickUp task that you closed only takes your pick away, with an undo", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("r1", "Old rock")],
      statuses: { "clickup:r1": { status: "cancelled", by: "you" } },
    });
    await userEvent.click(
      await screen.findByRole("button", { name: /Show finished/ }),
    );
    await userEvent.click(
      await screen.findByRole("button", { name: "Open again: Old rock" }),
    );
    await waitFor(() => expect(fake.statuses["clickup:r1"]).toBeUndefined());
    expect(fake.calls.some((c) => c.cmd === "set_item_status")).toBe(false);
    await userEvent.click(await screen.findByRole("button", { name: "Undo" }));
    await waitFor(() =>
      expect(fake.statuses["clickup:r1"]).toMatchObject({
        status: "cancelled",
        by: "you",
      }),
    );
  });

  it("Done in the status menu of a ClickUp task that is Unfinished in AIfred closes it in ClickUp", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("r1", "Old rock", { status: "delayed" })],
      statuses: {
        "clickup:r1": { status: "unfinished", by: "rule", source: "delayed" },
      },
    });
    await userEvent.click(
      await screen.findByRole("button", { name: /Show finished/ }),
    );
    await openItem("Old rock");
    await userEvent.selectOptions(
      within(details()).getByLabelText("Status"),
      "done",
    );
    await waitFor(() =>
      expect(
        fake.calls.find((c) => c.cmd === "clickup_mark_done")?.args,
      ).toEqual({ id: "r1" }),
    );
    expect(fake.calls.some((c) => c.cmd === "set_item_status")).toBe(false);
  });
});

describe("AIfred status: no ClickUp change for closed items", () => {
  it("the timer Done of a ClickUp task that is Unfinished in AIfred only stops the work", async () => {
    const fake = await startApp({
      clickupToken: "pk_test",
      clickup: [clickupItem("r1", "Old rock", { status: "delayed" })],
      statuses: {
        "clickup:r1": { status: "unfinished", by: "rule", source: "delayed" },
      },
      sessions: [
        {
          key: "clickup:r1",
          started: Math.floor(Date.now() / 1000) - 600,
          ended: null,
        },
      ],
    });
    await waitFor(() =>
      expect(fake.calls.some((c) => c.cmd === "finish_work")).toBe(true),
    );
    expect(fake.calls.some((c) => c.cmd === "clickup_mark_done")).toBe(false);
  });
});
