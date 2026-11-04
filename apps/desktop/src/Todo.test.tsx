// The to-do list on the home page: local tasks plus GitHub items.
import { describe, expect, it, vi } from "vitest";
import { render, screen, waitFor, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { TodoList } from "./Todo";
import { fakeBackend, githubItem, task } from "./test/fakeBackend";
import type { GithubItem, Task } from "./api";

function show(tasks: Task[], github: GithubItem[] = [], githubState = "ok") {
  const onChange = vi.fn();
  const onRefresh = vi.fn();
  render(<TodoList tasks={tasks} github={github} githubState={githubState} onChange={onChange} onRefresh={onRefresh} />);
  return { onChange, onRefresh };
}

describe("TodoList", () => {
  it("shows local tasks and GitHub items together, with a count", () => {
    fakeBackend();
    show(
      [task("Call Sam"), task("Send invoice", { added_by: "aifred" }), task("Old", { done: true })],
      [githubItem(7, "review_requested", "Fix cart"), githubItem(8, "assigned"), githubItem(9, "your_pr")],
    );
    expect(screen.getByText("Call Sam")).toBeInTheDocument();
    expect(screen.getByText("Fix cart")).toBeInTheDocument();
    expect(screen.getByText("5")).toBeInTheDocument(); // 2 open tasks + 3 GitHub items; done is not counted
    expect(screen.getAllByText("added by AIfred")).toHaveLength(1);
    expect(screen.getByText("Review")).toBeInTheDocument();
    expect(screen.getByText("Assigned")).toBeInTheDocument();
    expect(screen.getByText("Your PR")).toBeInTheDocument();
    expect(screen.getByText("acme/shop#7 · 2d ago")).toBeInTheDocument();
    expect(screen.queryByText("Old")).not.toBeInTheDocument(); // done tasks are folded away
  });

  it("shows linked pull requests and issues as small badges that open them", async () => {
    fakeBackend();
    const link = (g: GithubItem, reason?: GithubItem["reason"]) => ({
      id: g.id, repo: g.repo, number: g.number, title: g.title, url: g.url, reason,
      pull_request: g.url.includes("/pull/"),
    });
    const pr = githubItem(143, "your_pr", "Merge suite");
    const other = { ...githubItem(9, "your_pr", "Docs PR"), id: "acme/docs#9", repo: "acme/docs" };
    show([], [
      // Your issue with your PR inside it, plus a PR from another repository.
      { ...githubItem(142, "assigned", "Merge branch"), links: [link(pr, "your_pr"), link(other)] },
      // Your PR for an issue that is not in your list.
      { ...githubItem(150, "your_pr", "Fix cart"), links: [link(githubItem(149, "assigned", "Cart bug"))] },
      // Many links: only 3 badges, then "+2".
      { ...githubItem(160, "assigned", "Big"), links: [1, 2, 3, 4, 5].map((n) => link(githubItem(n, "your_pr"))) },
    ]);
    const prBadge = screen.getByRole("link", { name: "PR #143" });
    expect(prBadge).toHaveAttribute("href", pr.url);
    expect(prBadge).toHaveAttribute("title", "PR acme/shop#143: Merge suite (Your PR)");
    expect(prBadge.className).toContain("emerald");
    expect(screen.getByRole("link", { name: "PR docs#9" })).toHaveAttribute("title", "PR acme/docs#9: Docs PR");
    expect(screen.getByRole("link", { name: "Issue #149" })).toHaveAttribute(
      "href",
      "https://github.com/acme/shop/issues/149",
    );
    expect(screen.getByText("3")).toBeInTheDocument(); // the badges are not rows
    // The links after the first 3 hide behind a button that shows them.
    const more = screen.getByRole("button", { name: "Show 2 more links" });
    expect(more).toHaveTextContent("+2");
    expect(more).toHaveAttribute("title", "PR acme/shop#4: Item 4\nPR acme/shop#5: Item 5");
    expect(screen.queryByRole("link", { name: "PR #5" })).not.toBeInTheDocument();
    await userEvent.click(more);
    expect(screen.getByRole("link", { name: "PR #5" })).toHaveAttribute("href", "https://github.com/acme/shop/pull/5");
    expect(screen.queryByRole("button", { name: /more links/ })).not.toBeInTheDocument();
  });

  it("puts the badges under the title, with no buttons on them", () => {
    fakeBackend();
    const pr = githubItem(143, "your_pr", "Merge suite");
    show([], [{ ...githubItem(142, "assigned", "Merge branch"), links: [{ id: pr.id, repo: pr.repo, number: 143,
      title: pr.title, url: pr.url, pull_request: true, reason: "your_pr" }] }]);
    expect(screen.queryByRole("button", { name: /done/i })).not.toBeInTheDocument();
    const badge = screen.getByRole("link", { name: "PR #143" });
    const title = screen.getByRole("link", { name: "Merge branch" });
    expect(title.parentElement?.contains(badge)).toBe(true); // same column as the title, not beside it
  });

  it("filters by source", async () => {
    fakeBackend();
    show([task("Call Sam")], [githubItem(7, "assigned", "Fix cart")]);
    await userEvent.click(screen.getByRole("button", { name: "Mine" }));
    expect(screen.getByText("Call Sam")).toBeInTheDocument();
    expect(screen.queryByText("Fix cart")).not.toBeInTheDocument();

    await userEvent.click(screen.getByRole("button", { name: "GitHub" }));
    expect(screen.queryByText("Call Sam")).not.toBeInTheDocument();
    expect(screen.getByText("Fix cart")).toBeInTheDocument();
    expect(screen.queryByPlaceholderText(/Add a task/)).not.toBeInTheDocument();

    expect(screen.getByRole("button", { name: "ClickUp" })).toBeEnabled();
  });

  it("adds a task when you press Enter, then asks for a reload", async () => {
    const fake = fakeBackend();
    const { onChange } = show([]);
    const input = screen.getByPlaceholderText(/Add a task/);
    await userEvent.type(input, "Book flights{Enter}");
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(fake.tasks.map((t) => t.title)).toEqual(["Book flights"]);
    expect(input).toHaveValue("");
  });

  it("does not add an empty task", async () => {
    const fake = fakeBackend();
    show([]);
    await userEvent.type(screen.getByPlaceholderText(/Add a task/), "   {Enter}");
    expect(fake.calls.filter((c) => c.cmd === "add_task")).toHaveLength(0);
  });

  it("ticks a task done and deletes a task", async () => {
    const a = task("Call Sam");
    const b = task("Pay rent");
    const fake = fakeBackend({ tasks: [a, b] });
    const { onChange } = show([a, b]);

    const rowA = screen.getByText("Call Sam").closest("li")!;
    await userEvent.click(within(rowA).getByRole("checkbox"));
    await waitFor(() => expect(fake.tasks.find((t) => t.id === a.id)?.done).toBe(true));

    const rowB = screen.getByText("Pay rent").closest("li")!;
    await userEvent.click(within(rowB).getByTitle("Delete task"));
    await waitFor(() => expect(fake.tasks.map((t) => t.id)).toEqual([a.id]));
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("shows done tasks on request and can undo them", async () => {
    const d = task("Filed taxes", { done: true, done_at: 5 });
    const fake = fakeBackend({ tasks: [d] });
    show([d]);
    await userEvent.click(screen.getByRole("button", { name: "Show done (1)" }));
    const row = screen.getByText("Filed taxes").closest("li")!;
    await userEvent.click(within(row).getByRole("checkbox"));
    await waitFor(() => expect(fake.tasks[0].done).toBe(false));
  });

  it("shows the backend error text when a change fails", async () => {
    const ghost = task("Ghost"); // not in the fake backend, so the change fails
    fakeBackend();
    show([ghost]);
    await userEvent.click(within(screen.getByText("Ghost").closest("li")!).getByRole("checkbox"));
    expect(await screen.findByText(/no task with id/)).toBeInTheDocument();
  });

  it("shows loading, error, and empty states", () => {
    fakeBackend();
    const { unmount } = render(
      <TodoList tasks={[]} github={[]} githubState="loading" onChange={() => {}} onRefresh={() => {}} />,
    );
    expect(screen.getByText("Loading GitHub items…")).toBeInTheDocument();
    unmount();

    show([], [], "gh failed: not logged in");
    expect(screen.getByText("GitHub: gh failed: not logged in")).toBeInTheDocument();
    // "Nothing to do" would disagree with the error: we do not know.
    expect(screen.queryByText(/Nothing to do/)).not.toBeInTheDocument();
  });

  it("shows the empty message that fits the filter", async () => {
    fakeBackend();
    show([], []);
    expect(screen.getByText("Nothing to do. Add a task above.")).toBeInTheDocument();
    await userEvent.click(screen.getByRole("button", { name: "GitHub" }));
    // There is no add box on the GitHub filter.
    expect(screen.getByText("Nothing from GitHub.")).toBeInTheDocument();
    expect(screen.queryByText(/Add a task above/)).not.toBeInTheDocument();
  });

  it("shows why tasks cannot load, and not 'Nothing to do'", () => {
    fakeBackend();
    render(
      <TodoList tasks={[]} github={[]} githubState="ok" tasksError="cannot read tasks.json"
        onChange={() => {}} onRefresh={() => {}} />,
    );
    expect(screen.getByText("Cannot load your tasks: cannot read tasks.json")).toBeInTheDocument();
    expect(screen.queryByText(/Nothing to do/)).not.toBeInTheDocument();
  });

  it("keeps the typed text when the add fails", async () => {
    const fake = fakeBackend({ fail: { add_task: "disk full" } });
    const { onChange } = show([]);
    const input = screen.getByPlaceholderText(/Add a task/);
    await userEvent.type(input, "Book flights{Enter}");
    expect(await screen.findByText("disk full")).toBeInTheDocument();
    expect(input).toHaveValue("Book flights");
    expect(onChange).not.toHaveBeenCalled();
    expect(fake.tasks).toHaveLength(0);
  });

  it("the reload button asks for a reload", async () => {
    fakeBackend();
    const { onRefresh } = show([]);
    await userEvent.click(screen.getByTitle("Reload the list"));
    expect(onRefresh).toHaveBeenCalledOnce();
  });
  it("adds a task with an optional description", async () => {
    const fake = fakeBackend();
    show([]);
    expect(screen.queryByRole("textbox", { name: "New task description" })).not.toBeInTheDocument();
    await userEvent.type(screen.getByPlaceholderText("+ Add a task and press Enter"), "Pay rent");
    await userEvent.type(screen.getByRole("textbox", { name: "New task description" }), "Before Friday{Control>}{Enter}{/Control}");
    await waitFor(() => expect(fake.tasks).toHaveLength(1));
    expect(fake.tasks[0]).toMatchObject({ title: "Pay rent", notes: "Before Friday" });
  });

  it("changes the title and the description of a task", async () => {
    const t = task("Call Sam", { notes: "About the invoice" });
    const fake = fakeBackend({ tasks: [t] });
    const { onChange } = show([t]);
    expect(screen.getByText("About the invoice")).toBeInTheDocument();
    await userEvent.click(screen.getByText("Call Sam"));
    const title = screen.getByRole("textbox", { name: "Task title" });
    await userEvent.clear(title);
    await userEvent.type(title, "Call Sam today");
    const notes = screen.getByRole("textbox", { name: "Task description" });
    await userEvent.clear(notes);
    await userEvent.type(notes, "Ask about the{Shift>}{Enter}{/Shift}second invoice");
    await userEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(onChange).toHaveBeenCalled());
    expect(fake.tasks[0]).toMatchObject({ title: "Call Sam today", notes: "Ask about the\nsecond invoice" });
    expect(screen.queryByRole("textbox", { name: "Task title" })).not.toBeInTheDocument();
  });

  it("Escape stops an edit and changes nothing", async () => {
    const t = task("Call Sam");
    const fake = fakeBackend({ tasks: [t] });
    show([t]);
    await userEvent.click(screen.getByText("Call Sam"));
    await userEvent.type(screen.getByRole("textbox", { name: "Task title" }), " later{Escape}");
    expect(screen.queryByRole("textbox", { name: "Task title" })).not.toBeInTheDocument();
    expect(fake.tasks[0].title).toBe("Call Sam");
    expect(fake.calls.some((c) => c.cmd === "edit_task")).toBe(false);
  });

  it("keeps an edit in progress when the list loads again with a changed task", async () => {
    const t = task("Call Sam", { notes: "About the invoice" });
    fakeBackend({ tasks: [t] });
    const props = { github: [], githubState: "ok", onChange: vi.fn(), onRefresh: vi.fn() };
    const { rerender } = render(<TodoList tasks={[t]} {...props} />);
    await userEvent.click(screen.getByText("Call Sam"));
    const notes = screen.getByRole("textbox", { name: "Task description" });
    await userEvent.type(notes, " and the receipt");
    // The list loads again: the task changed in another place.
    const changed = { ...t, title: "Call Sam now" };
    rerender(<TodoList tasks={[changed]} {...props} />);
    expect(screen.getByRole("textbox", { name: "Task description" })).toHaveValue("About the invoice and the receipt");
    // After Cancel, the row shows the new title, and a new edit starts from it.
    await userEvent.click(screen.getByRole("button", { name: "Cancel" }));
    await userEvent.click(screen.getByText("Call Sam now"));
    expect(screen.getByRole("textbox", { name: "Task title" })).toHaveValue("Call Sam now");
  });
});
