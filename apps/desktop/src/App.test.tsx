// The home page and the chat panel: the full window, with a fake backend.
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import App from "./App";
import { answer, deferred, fakeBackend, githubItem, task } from "./test/fakeBackend";
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

describe("memory editor", () => {
  it("saves what AIfred knows about you", async () => {
    const fake = await startApp({ memory: "- I work on StoreAgent." });
    await userEvent.click(screen.getByRole("button", { name: /What AIfred knows about you/ }));
    const box = screen.getByDisplayValue("- I work on StoreAgent.");
    await userEvent.type(box, "\n- I like short answers.");
    await userEvent.click(screen.getByRole("button", { name: "Save memory" }));
    await screen.findByRole("button", { name: "Saved" });
    expect(fake.memory).toBe("- I work on StoreAgent.\n- I like short answers.");
  });
});

describe("errors the window must show", () => {
  it("a failed memory save says so and keeps the edit", async () => {
    await startApp({ memory: "", fail: { set_memory: "read-only folder" } });
    await userEvent.click(screen.getByRole("button", { name: /What AIfred knows about you/ }));
    await userEvent.type(screen.getByPlaceholderText(/StoreAgent/), "- Note");
    await userEvent.click(screen.getByRole("button", { name: "Save memory" }));
    expect(await screen.findByText("Not saved: read-only folder")).toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Save memory" })).toBeEnabled();
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
      const fake = await startApp({ github: [githubItem(7, "assigned")], settings: { github_refresh_minutes: 5 } });
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
      const fake = await startApp({ github: [githubItem(7, "assigned")], settings: { github_refresh_minutes: 0 } });
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
    const select = screen.getByLabelText("Reload GitHub items");
    expect(select).toHaveValue("15");
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await userEvent.selectOptions(select, "Only when I click reload");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Saved")).toBeInTheDocument();
    expect(fake.settings.github_refresh_minutes).toBe(0);
    await userEvent.click(screen.getByRole("button", { name: "← Back" }));
    expect(screen.getByText("Good to see you.")).toBeInTheDocument();
  });

  it("shows a failed save", async () => {
    await startApp({ fail: { set_settings: "read-only folder" } });
    await userEvent.click(screen.getByRole("button", { name: "Settings" }));
    await userEvent.selectOptions(screen.getByLabelText("Reload GitHub items"), "Every hour");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(await screen.findByText("Not saved: read-only folder")).toBeInTheDocument();
  });
});
