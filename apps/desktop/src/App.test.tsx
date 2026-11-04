// The home page and the chat panel: the full window, with a fake backend.
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { answer, clickupItem, deferred, fact, fakeBackend, githubItem, task } from "./test/fakeBackend";
import type { Answer } from "./api";

const panel = () => screen.queryByRole("complementary", { name: "Conversation" });

async function startApp(init: Parameters<typeof fakeBackend>[0] = {}) {
  const fake = fakeBackend(init);
  render(<App />);
  await screen.findByText("Claude (your login)"); // first load is done
  return fake;
}

describe("home page", () => {
  it("shows the chat box and the to-do list, and no chat panel", async () => {
    await startApp({ tasks: [task("Call Sam")], github: [githubItem(7, "review_requested", "Fix cart")] });
    expect(screen.getByText("Good to see you.")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Ask AIfred anything about your work…")).toBeInTheDocument();
    expect(await screen.findByText("Fix cart")).toBeInTheDocument();
    expect(screen.getByText("Call Sam")).toBeInTheDocument();
    expect(panel()).not.toBeInTheDocument();
  });

  it("offers to continue an old conversation", async () => {
    await startApp({
      history: [
        { ts: 1, role: "user", text: "Hi", backend: "claude" },
        { ts: 2, role: "assistant", text: "Hello!", backend: "claude" },
      ],
    });
    await userEvent.click(screen.getByRole("button", { name: /Continue the conversation \(2 messages\)/ }));
    expect(within(panel()!).getByText("Hello!")).toBeInTheDocument();
  });
});

describe("the model picker", () => {
  it("shows the model you picked last and uses it for the next message", async () => {
    const fake = await startApp({ chatModel: "sonnet" });
    const picker = await screen.findByRole("combobox", { name: "Model" });
    await waitFor(() => expect(picker).toHaveValue("sonnet"));
    await userEvent.selectOptions(picker, "haiku");
    expect(fake.chatModel).toBe("haiku");
    await userEvent.type(screen.getByPlaceholderText("Ask AIfred anything about your work…"), "hi{Enter}");
    expect(await within(panel()!).findByText(/^1\.5s · Haiku/)).toBeInTheDocument();
    // The chat panel has its own box with the same pick.
    expect(within(panel()!).getByRole("combobox", { name: "Model" })).toHaveValue("haiku");
    await userEvent.selectOptions(within(panel()!).getByRole("combobox", { name: "Model" }), "");
    expect(fake.chatModel).toBeNull();
  });

  it("goes back to the old pick when the save fails", async () => {
    await startApp({ fail: { set_chat_model: "the database is locked" } });
    const picker = await screen.findByRole("combobox", { name: "Model" });
    await userEvent.selectOptions(picker, "opus");
    await waitFor(() => expect(picker).toHaveValue(""));
  });
});

describe("sending a message", () => {
  it("moves the chat into the right panel and keeps the to-do list", async () => {
    const reply = deferred<Answer>();
    await startApp({ tasks: [task("Call Sam")], onAsk: () => reply.promise });

    await userEvent.type(screen.getByPlaceholderText("Ask AIfred anything about your work…"), "What is new?{Enter}");

    const side = panel()!;
    expect(side).toBeInTheDocument();
    expect(within(side).getByText("What is new?")).toBeInTheDocument();
    expect(within(side).getByText("Thinking…")).toBeInTheDocument();
    expect(screen.queryByText("Good to see you.")).not.toBeInTheDocument(); // big chat box is gone
    expect(screen.getByText("Call Sam")).toBeInTheDocument(); // to-do list stays

    reply.resolve(answer("**Two** new reviews.", { cost_usd: 0.0054 }));
    expect(await within(side).findByText("Two")).toBeInTheDocument(); // markdown is shown as bold
    expect(within(side).getByText("1.5s · ~$0.005 usage")).toBeInTheDocument();
    expect(within(side).queryByText("Thinking…")).not.toBeInTheDocument();
  });

  it("shows progress while AIfred works", async () => {
    const reply = deferred<Answer>();
    await startApp({
      onAsk: (_p, progress) => {
        progress({ kind: "tool_call", server: "github", tool: "review_requests", input: {} });
        return reply.promise;
      },
    });
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Reviews?{Enter}");
    expect(await within(panel()!).findByText("✓ Checking reviews you were asked to do")).toBeInTheDocument();
    reply.resolve(answer("None."));
    expect(await within(panel()!).findByText("None.")).toBeInTheDocument();
  });

  it("a suggestion button sends its question", async () => {
    const fake = await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Brief me" }));
    await within(panel()!).findByText(/You said: What needs my attention/);
    expect(fake.calls.find((c) => c.cmd === "ask")?.args.prompt).toMatch(/^What needs my attention/);
  });

  it("Shift+Enter makes a new line and does not send", async () => {
    const fake = await startApp();
    const box = screen.getByPlaceholderText(/Ask AIfred anything/);
    await userEvent.type(box, "line one{Shift>}{Enter}{/Shift}line two");
    expect(box).toHaveValue("line one\nline two");
    expect(fake.calls.some((c) => c.cmd === "ask")).toBe(false);
  });

  it("reloads the to-do list after the answer, so tasks AIfred added show", async () => {
    const fake = await startApp({
      onAsk: () => {
        fake.tasks.push(task("Send the April invoice to Sam", { added_by: "aifred" }));
        return answer("Added it.");
      },
    });
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Remind me to send the invoice{Enter}");
    expect(await screen.findByText("Send the April invoice to Sam")).toBeInTheDocument();
    expect(screen.getByText("added by AIfred")).toBeInTheDocument();
  });

  it("shows an error in the panel when the question fails", async () => {
    await startApp({ onAsk: () => Promise.reject("claude is not logged in") });
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Hi{Enter}");
    expect(await within(panel()!).findByText("claude is not logged in")).toBeInTheDocument();
  });

  it("does not send while AIfred is still working", async () => {
    const reply = deferred<Answer>();
    const fake = await startApp({ onAsk: () => reply.promise });
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "First{Enter}");
    const box = within(panel()!).getByPlaceholderText("Ask AIfred…");
    await userEvent.type(box, "Second{Enter}");
    expect(within(panel()!).getByRole("button", { name: "Send" })).toBeDisabled();
    expect(fake.calls.filter((c) => c.cmd === "ask")).toHaveLength(1);
    reply.resolve(answer("ok"));
    await within(panel()!).findByText("ok");
  });
});

describe("closing the chat panel", () => {
  it("✕ goes back to the home page and keeps the conversation", async () => {
    await startApp();
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Hi{Enter}");
    await within(panel()!).findByText("You said: Hi");

    await userEvent.click(screen.getByTitle("Close the conversation panel"));
    expect(panel()).not.toBeInTheDocument();
    expect(screen.getByText("Good to see you.")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: /Continue the conversation \(2 messages\)/ })).toBeInTheDocument();
  });

  it("New starts a new conversation and closes the panel", async () => {
    const fake = await startApp();
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Hi{Enter}");
    await within(panel()!).findByText("You said: Hi");

    await userEvent.click(within(panel()!).getByRole("button", { name: "New" }));
    await waitFor(() => expect(panel()).not.toBeInTheDocument());
    expect(fake.calls.some((c) => c.cmd === "new_conversation")).toBe(true);
    expect(screen.queryByText(/Continue the conversation/)).not.toBeInTheDocument();
  });

  it("the logo goes back home", async () => {
    await startApp();
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Hi{Enter}");
    await within(panel()!).findByText("You said: Hi");
    await userEvent.click(screen.getByTitle("Home"));
    expect(panel()).not.toBeInTheDocument();
  });
});

describe("memory list", () => {
  const openMemory = () => userEvent.click(screen.getByRole("button", { name: /What AIfred knows about you/ }));

  it("adds a fact that goes into every message", async () => {
    const fake = await startApp({ memory: [fact("I work on StoreAgent.")] });
    await openMemory();
    expect(screen.getByText("I work on StoreAgent.")).toBeInTheDocument();
    await userEvent.type(screen.getByPlaceholderText("Add a fact and press Enter"), "I like short answers.{Enter}");
    expect(await screen.findByText("I like short answers.")).toBeInTheDocument();
    expect(fake.memory.map((f) => [f.text, f.core])).toEqual([
      ["I work on StoreAgent.", true],
      ["I like short answers.", true],
    ]);
    expect(screen.getByPlaceholderText("Add a fact and press Enter")).toHaveValue("");
  });

  it("edits a fact and turns off 'Always'", async () => {
    const fake = await startApp({ memory: [fact("Tea, no sugar")] });
    await openMemory();
    await userEvent.click(screen.getByText("Tea, no sugar"));
    const box = screen.getByLabelText("Fact text");
    await userEvent.clear(box);
    await userEvent.type(box, "Green tea, no sugar{Enter}");
    expect(await screen.findByText("Green tea, no sugar")).toBeInTheDocument();
    await userEvent.click(screen.getByLabelText("Always use: Green tea, no sugar"));
    await waitFor(() => expect(fake.memory[0].core).toBe(false));
    expect(fake.memory[0].text).toBe("Green tea, no sugar");
  });

  it("starts the edit box with the saved text, so an unchanged fact is not saved again", async () => {
    const fake = await startApp({ memory: [fact("Tea")] });
    await openMemory();
    await userEvent.click(screen.getByText("Tea"));
    const box = screen.getByLabelText("Fact text");
    await userEvent.clear(box);
    // The backend joins the two lines into "likes tea".
    await userEvent.type(box, "likes{Shift>}{Enter}{/Shift} tea{Enter}");
    expect(await screen.findByText("likes tea")).toBeInTheDocument();
    await userEvent.click(screen.getByText("likes tea"));
    expect(screen.getByLabelText("Fact text")).toHaveValue("likes tea");
    await userEvent.tab();
    expect(fake.calls.filter((c) => c.cmd === "edit_fact")).toHaveLength(1);
  });

  it("forgets a fact and restores it", async () => {
    const fake = await startApp({ memory: [fact("Old job at Acme", { added_by: "aifred" })] });
    await openMemory();
    expect(screen.getByText("by AIfred")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Forget: Old job at Acme" }));
    await userEvent.click(await screen.findByRole("button", { name: /Forgotten \(1\)/ }));
    expect(fake.memory[0].forgotten).not.toBeNull();
    await userEvent.click(screen.getByRole("button", { name: "Restore: Old job at Acme" }));
    expect(await screen.findByRole("button", { name: "Forget: Old job at Acme" })).toBeInTheDocument();
    expect(fake.memory[0].forgotten).toBeNull();
  });

  it("shows the facts that the app added to a question", async () => {
    await startApp({ onAsk: () => answer("It comes on the 1st.", { memory: ["Sam sends the hosting invoice"] }) });
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "When is the invoice?{Enter}");
    const side = panel()!;
    await userEvent.click(await within(side).findByText("Used 1 fact from memory"));
    expect(within(side).getByText("Sam sends the hosting invoice")).toBeVisible();
  });

  it("shows a fact that AIfred added in the chat", async () => {
    const fake = await startApp({
      onAsk: () => {
        fake.memory.push(fact("Sam sends the invoices", { core: false, added_by: "aifred" }));
        return answer("Noted.");
      },
    });
    await openMemory();
    await userEvent.type(screen.getByPlaceholderText(/Ask AIfred anything/), "Sam sends the invoices{Enter}");
    expect(await screen.findByText("Sam sends the invoices", { selector: "button" })).toBeInTheDocument();
  });
});

describe("errors the window must show", () => {
  it("a failed memory save says so and keeps the text", async () => {
    await startApp({ fail: { add_fact: "database is locked" } });
    await userEvent.click(screen.getByRole("button", { name: /What AIfred knows about you/ }));
    await userEvent.type(screen.getByPlaceholderText("Add a fact and press Enter"), "Note{Enter}");
    expect(await screen.findByText("Not saved: database is locked")).toBeInTheDocument();
    expect(screen.getByPlaceholderText("Add a fact and press Enter")).toHaveValue("Note");
  });

  it("a task file that cannot load shows the reason, not an empty list", async () => {
    await startApp({ fail: { tasks: "cannot read /x/tasks.json" } });
    expect(await screen.findByText("Cannot load your tasks: cannot read /x/tasks.json")).toBeInTheDocument();
    expect(screen.queryByText(/Nothing to do/)).not.toBeInTheDocument();
  });

  it("a failed start-up load shows the reason in the sidebar", async () => {
    fakeBackend({ fail: { history: "bad history file" } });
    render(<App />);
    expect(await screen.findByText("Cannot load the conversation: bad history file")).toBeInTheDocument();
  });
});

describe("GitHub list: saved copy and timer", () => {
  it("asks GitHub once at start", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")] });
    await screen.findByText("Fix cart");
    expect(fake.githubFetches).toBe(1);
  });

  it("the reload button asks GitHub again", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")] });
    await screen.findByText("Fix cart");
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() => expect(fake.githubFetches).toBe(2));
  });

  it("reloads from GitHub on the timer from the settings", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({ github: [githubItem(7, "assigned")], settings: { refresh_minutes: 5 } });
      await waitFor(() => expect(fake.githubFetches).toBe(1));
      await vi.advanceTimersByTimeAsync(5 * 60_000);
      await waitFor(() => expect(fake.githubFetches).toBe(2));
    } finally {
      vi.useRealTimers();
    }
  });

  it("does not reload on a timer when the setting is 'only when I click reload'", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({ github: [githubItem(7, "assigned")], settings: { refresh_minutes: 0 } });
      await waitFor(() => expect(fake.githubFetches).toBe(1));
      await vi.advanceTimersByTimeAsync(24 * 60 * 60_000);
      expect(fake.githubFetches).toBe(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("GitHub items are read-only", () => {
  it("have no checkbox and never call a command that changes GitHub", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")] });
    const row = (await screen.findByText("Fix cart")).closest("li")!;
    expect(within(row).queryByRole("checkbox")).not.toBeInTheDocument();
    expect(fake.calls.map((c) => c.cmd).filter((c) => c.startsWith("github_") && c !== "github_items")).toEqual([]);
  });

  it("an item closed on GitHub leaves the list on the next reload", async () => {
    const fake = await startApp({ github: [githubItem(7, "assigned", "Fix cart")] });
    await screen.findByText("Fix cart");
    fake.github = [];
    await userEvent.click(screen.getByTitle("Reload the list"));
    await waitFor(() => expect(screen.queryByText("Fix cart")).not.toBeInTheDocument());
  });

  it("shows the saved list with a warning when GitHub cannot be reached", async () => {
    await startApp({
      github: [githubItem(7, "assigned", "Fix cart")],
      githubWarning: "Cannot reach GitHub, so this is the saved list. gh failed: offline",
    });
    expect(await screen.findByText("Fix cart")).toBeInTheDocument();
    expect(screen.getByText(/Cannot reach GitHub/)).toBeInTheDocument();
  });
});

describe("settings page", () => {
  it("saves the GitHub reload time", async () => {
    const fake = await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    const select = screen.getByLabelText("Reload GitHub and ClickUp items");
    expect(select).toHaveValue("15");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.selectOptions(select, "Only when I click reload");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(fake.settings.refresh_minutes).toBe(0);
    await userEvent.click(screen.getByRole("button", { name: "← Back" }));
    expect(screen.getByText("Good to see you.")).toBeInTheDocument();
  });

  it("checks for a new version and restarts into it", async () => {
    const fake = await startApp({ updateTo: "0.2.0" });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(screen.getByText("This is AIfred 0.1.1.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(await screen.findByText(/Version 0.2.0 is downloaded/)).toBeInTheDocument();
    // The sidebar shows it too.
    expect(screen.getByText("AIfred 0.2.0 is ready.")).toBeInTheDocument();
    await userEvent.click(screen.getAllByRole("button", { name: "Restart now" })[0]);
    await waitFor(() => expect(fake.restarts).toBe(1));
  });

  it("says when there is no new version", async () => {
    await startApp();
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(screen.getByRole("button", { name: "Check now" }));
    expect(await screen.findByText("You have the newest version.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Restart now" })).not.toBeInTheDocument();
  });

  it("shows why updates are off", async () => {
    await startApp({ update: { current: "0.1.1", state: "off", latest: null, page: null, message: "Updates are off in development builds.", checked_at: null } });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    expect(await screen.findByText("Updates are off in development builds.")).toBeInTheDocument();
    expect(screen.queryByRole("button", { name: "Check now" })).not.toBeInTheDocument();
  });

  it("shows a failed save", async () => {
    await startApp({ fail: { set_settings: "read-only folder" } });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.selectOptions(screen.getByLabelText("Reload GitHub and ClickUp items"), "Every hour");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Not saved: read-only folder")).toBeInTheDocument();
  });
});

describe("ClickUp", () => {
  const connected = { clickupToken: "pk_test" };

  it("shows nothing from ClickUp and makes no ClickUp call when it is not connected", async () => {
    const fake = await startApp({ clickup: [clickupItem("86a", "Write notes")] });
    await userEvent.click(screen.getByRole("button", { name: "ClickUp" }));
    expect(screen.getByText(/ClickUp is not connected/)).toBeInTheDocument();
    expect(screen.queryByText("Write notes")).not.toBeInTheDocument();
    expect(fake.calls.some((c) => c.cmd === "clickup_items")).toBe(false);
  });

  it("shows your ClickUp tasks next to the others, with status and list", async () => {
    await startApp({ ...connected, tasks: [task("Call Sam")], clickup: [clickupItem("86a", "Write notes", { custom_id: "DEV-42" })] });
    const row = (await screen.findByText("Write notes")).closest("li")!;
    expect(within(row).getByText("in progress")).toBeInTheDocument();
    expect(within(row).getByText(/DEV-42 · Sprint 12/)).toBeInTheDocument();
    expect(screen.getByText("Call Sam")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "ClickUp" }));
    expect(screen.queryByText("Call Sam")).not.toBeInTheDocument();
    expect(screen.getByText("Write notes")).toBeInTheDocument();
  });

  it("the tick closes the task in ClickUp, and Undo puts the old status back", async () => {
    const fake = await startApp({ ...connected, clickup: [clickupItem("86a", "Write notes", { status: "review" })] });
    await userEvent.click(await screen.findByRole("checkbox", { name: "Close in ClickUp: Write notes" }));
    expect(await screen.findByText("Closed in ClickUp: Write notes")).toBeInTheDocument();
    expect(screen.queryByRole("link", { name: "Write notes" })).not.toBeInTheDocument();
    expect(fake.clickup).toEqual([]);

    await userEvent.click(screen.getByRole("button", { name: "Undo: Write notes" }));
    expect(await screen.findByRole("link", { name: "Write notes" })).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "clickup_undo_done")!.args).toEqual({ id: "86a", status: "review" });
    expect(screen.queryByText(/Closed in ClickUp/)).not.toBeInTheDocument();
  });

  it("a second tick keeps the undo of the first task", async () => {
    const fake = await startApp({
      ...connected,
      clickup: [clickupItem("86a", "Write notes", { status: "review" }), clickupItem("86b", "Fix login", { status: "to do" })],
    });
    await userEvent.click(await screen.findByRole("checkbox", { name: "Close in ClickUp: Write notes" }));
    await screen.findByText("Closed in ClickUp: Write notes");
    await userEvent.click(screen.getByRole("checkbox", { name: "Close in ClickUp: Fix login" }));
    await screen.findByText("Closed in ClickUp: Fix login");
    expect(screen.getByText("Closed in ClickUp: Write notes")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Undo: Write notes" }));
    expect(await screen.findByRole("link", { name: "Write notes" })).toBeInTheDocument();
    expect(fake.calls.find((c) => c.cmd === "clickup_undo_done")!.args).toEqual({ id: "86a", status: "review" });
    expect(screen.queryByText("Closed in ClickUp: Write notes")).not.toBeInTheDocument();
    expect(screen.getByText("Closed in ClickUp: Fix login")).toBeInTheDocument();
  });

  it("a failed close keeps the task and says why", async () => {
    await startApp({ ...connected, clickup: [clickupItem("86a", "Write notes")], fail: { clickup_mark_done: "ClickUp said 403: no access" } });
    await userEvent.click(await screen.findByRole("checkbox", { name: "Close in ClickUp: Write notes" }));
    expect(await screen.findByText("ClickUp: ClickUp said 403: no access")).toBeInTheDocument();
    expect(screen.getByRole("link", { name: "Write notes" })).toBeInTheDocument();
  });

  it("the reload button and the timer ask ClickUp again", async () => {
    vi.useFakeTimers({ shouldAdvanceTime: true });
    try {
      const fake = await startApp({ ...connected, clickup: [clickupItem("86a")], settings: { refresh_minutes: 5 } });
      await waitFor(() => expect(fake.clickupFetches).toBe(1));
      await userEvent.click(screen.getByTitle("Reload the list"));
      await waitFor(() => expect(fake.clickupFetches).toBe(2));
      await vi.advanceTimersByTimeAsync(5 * 60_000);
      await waitFor(() => expect(fake.clickupFetches).toBe(3));
    } finally {
      vi.useRealTimers();
    }
  });

  it("shows the saved list with a warning, or the error, when ClickUp cannot be reached", async () => {
    await startApp({ ...connected, clickup: [clickupItem("86a", "Write notes")], clickupWarning: "Cannot reach ClickUp, so this is the saved list." });
    expect(await screen.findByText("Write notes")).toBeInTheDocument();
    expect(screen.getByText(/Cannot reach ClickUp/)).toBeInTheDocument();
  });

  it("connects with a token on the Settings page, then loads the tasks", async () => {
    const fake = await startApp({ clickup: [clickupItem("86a", "Write notes")] });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.type(screen.getByLabelText("ClickUp"), "bad");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText(/ClickUp refused the token/)).toBeInTheDocument();

    await userEvent.clear(screen.getByLabelText("ClickUp"));
    await userEvent.type(screen.getByLabelText("ClickUp"), "pk_123");
    await userEvent.click(screen.getByRole("button", { name: "Connect" }));
    expect(await screen.findByText("Connected as okky.")).toBeInTheDocument();
    expect(fake.clickupToken).toBe("pk_123");

    await userEvent.click(screen.getByRole("button", { name: "← Back" }));
    expect(await screen.findByText("Write notes")).toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.click(await screen.findByRole("button", { name: "Disconnect" }));
    expect(await screen.findByText(/Disconnected/)).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "← Back" }));
    await waitFor(() => expect(screen.queryByText("Write notes")).not.toBeInTheDocument());
  });
});
