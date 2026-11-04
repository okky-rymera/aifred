// Checks that each wrapper calls the right Rust command with the right
// argument names. A wrong name fails only at run time in the real app.
import { describe, expect, it } from "vitest";
import * as api from "./api";
import { answer, fakeBackend, githubItem } from "./test/fakeBackend";

describe("command wrappers", () => {
  it("send the names and arguments that the Rust commands expect", async () => {
    const fake = fakeBackend({ github: [githubItem(7, "assigned")] });
    const t = await api.addTask("Call Sam");
    await api.setTaskDone(t.id, true);
    await api.deleteTask(t.id);
    const f = await api.addFact("likes tea");
    await api.editFact(f.id, "likes green tea");
    await api.setFactCore(f.id, false);
    await api.forgetFact(f.id);
    await api.restoreFact(f.id);
    await api.githubItems(true);
    await api.getSettings();
    await api.setSettings({ refresh_minutes: 30 });
    expect(fake.calls).toEqual([
      { cmd: "add_task", args: { task: { title: "Call Sam", notes: "" } } },
      { cmd: "set_task_done", args: { id: t.id, done: true } },
      { cmd: "delete_task", args: { id: t.id } },
      { cmd: "add_fact", args: { text: "likes tea" } },
      { cmd: "edit_fact", args: { id: f.id, text: "likes green tea" } },
      { cmd: "set_fact_core", args: { id: f.id, core: false } },
      { cmd: "forget_fact", args: { id: f.id } },
      { cmd: "restore_fact", args: { id: f.id } },
      { cmd: "github_items", args: { force: true } },
      { cmd: "get_settings", args: {} },
      { cmd: "set_settings", args: { settings: { refresh_minutes: 30 } } },
    ]);
  });

  it("ask passes progress messages to the caller", async () => {
    fakeBackend({
      onAsk: (_p, progress) => {
        progress({ kind: "tool_call", server: "github", tool: "my_issues", input: {} });
        progress({ kind: "notice", message: "Almost done" });
        return answer("Done.");
      },
    });
    const seen: api.ProgressEvent[] = [];
    const a = await api.ask("Brief me", (e) => seen.push(e));
    expect(a.text).toBe("Done.");
    expect(seen.map(api.describe)).toEqual(["Checking issues assigned to you", "Almost done"]);
  });
});

describe("describe()", () => {
  const call = (tool: string, input: Record<string, unknown> = {}) =>
    api.describe({ kind: "tool_call", server: "x", tool, input });

  it("gives plain labels for known tools", () => {
    expect(call("review_requests")).toBe("Checking reviews you were asked to do");
    expect(call("add_task", { title: "Pay rent" })).toBe("Adding task: Pay rent");
    expect(call("item_details", { repo: "acme/shop", number: 7 })).toBe("Reading acme/shop#7");
  });

  it("falls back to server and tool name for unknown tools", () => {
    expect(api.describe({ kind: "tool_call", server: "slack", tool: "search", input: {} })).toBe(
      "Using slack → search",
    );
  });
});
