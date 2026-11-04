import { useState } from "react";
import * as api from "./api";

type Filter = "all" | "local" | "github" | "clickup";

const PRIORITY: Record<NonNullable<api.ClickupItem["priority"]>, string> = {
  urgent: "text-red-600",
  high: "text-amber-600",
  normal: "text-stone-500",
  low: "text-stone-400",
};

/** "due today", "due in 3d", "3d overdue", for a time in Unix milliseconds. */
function dueText(ms: number): { text: string; late: boolean } {
  const day = (t: number) => Math.floor((t - new Date(t).getTimezoneOffset() * 60_000) / 86400_000);
  const days = day(ms) - day(Date.now());
  if (days === 0) return { text: "due today", late: false };
  if (days < 0) return { text: `${-days}d overdue`, late: true };
  return { text: `due in ${days}d`, late: false };
}

const REASON: Record<api.GithubItem["reason"], { label: string; cls: string }> = {
  review_requested: { label: "Review", cls: "bg-violet-100 text-violet-800 dark:bg-violet-950 dark:text-violet-300" },
  assigned: { label: "Assigned", cls: "bg-sky-100 text-sky-800 dark:bg-sky-950 dark:text-sky-300" },
  your_pr: { label: "Your PR", cls: "bg-emerald-100 text-emerald-800 dark:bg-emerald-950 dark:text-emerald-300" },
};

/** The most link badges on one row. The rest hide behind a "+N" button. */
const MAX_LINKS = 3;

/** Small badges that open the linked pull requests (on an issue) or issues (on a pull request). */
function LinkBadges({ item }: { item: api.GithubItem }) {
  const [all, setAll] = useState(false);
  const links = item.links ?? [];
  if (links.length === 0) return null;
  const kind = (l: api.GithubLink) => (l.pull_request ? "PR" : "Issue");
  const name = (l: api.GithubLink) => `${l.repo === item.repo ? "" : l.repo.split("/")[1]}#${l.number}`;
  const hint = (l: api.GithubLink) =>
    `${kind(l)} ${l.repo}#${l.number}: ${l.title}${l.reason ? ` (${REASON[l.reason].label})` : ""}`;
  const extra = links.slice(MAX_LINKS);
  return (
    <>
      {(all ? links : links.slice(0, MAX_LINKS)).map((l) => (
        <a
          key={l.id}
          href={l.url}
          title={hint(l)}
          className={`rounded px-1.5 py-0.5 text-[11px] hover:underline ${
            l.reason ? REASON[l.reason].cls : "bg-stone-100 text-stone-600 dark:bg-stone-800 dark:text-stone-300"
          }`}
        >
          {kind(l)} {name(l)}
        </a>
      ))}
      {extra.length > 0 && !all && (
        <button
          onClick={() => setAll(true)}
          title={extra.map(hint).join("\n")}
          aria-label={`Show ${extra.length} more links`}
          className="rounded px-1 text-[11px] text-stone-500 hover:bg-stone-100 hover:underline dark:hover:bg-stone-800"
        >
          +{extra.length}
        </button>
      )}
    </>
  );
}

function ago(iso: string): string {
  return agoSecs(new Date(iso).getTime() / 1000);
}

/** "5m ago" for a time in Unix seconds. */
function agoSecs(t: number): string {
  const s = Date.now() / 1000 - t;
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))}m ago`;
  if (s < 86400) return `${Math.round(s / 3600)}h ago`;
  return `${Math.round(s / 86400)}d ago`;
}

function Chip({ active, onClick, children, disabled }: {
  active?: boolean;
  onClick?: () => void;
  children: React.ReactNode;
  disabled?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      disabled={disabled}
      title={disabled ? "Coming later" : undefined}
      className={`rounded-full px-3 py-1 text-xs font-medium disabled:opacity-40 ${
        active
          ? "bg-[#252e4a] text-stone-50"
          : "border border-stone-300 text-stone-600 hover:bg-stone-200 dark:border-stone-700 dark:text-stone-300 dark:hover:bg-stone-800"
      }`}
    >
      {children}
    </button>
  );
}

/** One open local task. Click the title to change the title or the description. */
function TaskRow(props: { task: api.Task; run: (p: Promise<unknown>) => Promise<boolean> }) {
  const t = props.task;
  const [editing, setEditing] = useState(false);
  const [title, setTitle] = useState(t.title);
  const [notes, setNotes] = useState(t.notes);

  function start() {
    setTitle(t.title);
    setNotes(t.notes);
    setEditing(true);
  }

  async function save() {
    if (!title.trim()) return;
    if (await props.run(api.editTask(t.id, title, notes))) setEditing(false);
  }

  return (
    <li className="group flex items-start gap-3 py-2.5">
      <input
        type="checkbox"
        aria-label={`Mark "${t.title}" done`}
        checked={false}
        onChange={() => props.run(api.setTaskDone(t.id, true))}
        className="mt-0.5 size-4 accent-amber-500"
      />
      {editing ? (
        <div className="min-w-0 flex-1 space-y-1.5">
          <input
            aria-label="Task title"
            value={title}
            autoFocus
            onChange={(e) => setTitle(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter") save();
              if (e.key === "Escape") setEditing(false);
            }}
            className="w-full rounded border border-stone-300 bg-transparent px-2 py-1 text-sm outline-none focus:border-amber-500 dark:border-stone-700"
          />
          <textarea
            aria-label="Task description"
            value={notes}
            rows={Math.max(2, notes.split("\n").length)}
            placeholder="Description (optional)"
            onChange={(e) => setNotes(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.ctrlKey || e.metaKey)) save();
              if (e.key === "Escape") setEditing(false);
            }}
            className="w-full resize-none rounded border border-stone-300 bg-transparent px-2 py-1 text-xs outline-none focus:border-amber-500 dark:border-stone-700"
          />
          <div className="flex gap-2 text-xs">
            <button onClick={save} disabled={!title.trim()} className="rounded bg-[#252e4a] px-2 py-1 text-stone-50 disabled:opacity-40">
              Save
            </button>
            <button onClick={() => setEditing(false)} className="rounded px-2 py-1 text-stone-500 hover:bg-stone-200 dark:hover:bg-stone-800">
              Cancel
            </button>
          </div>
        </div>
      ) : (
        <button onClick={start} title="Change the title or the description" className="min-w-0 flex-1 text-left">
          <div className="text-sm">{t.title}</div>
          {t.notes && <div className="line-clamp-2 whitespace-pre-line text-xs text-stone-500">{t.notes}</div>}
        </button>
      )}
      {t.added_by === "aifred" && (
        <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[11px] text-amber-800 dark:bg-amber-950 dark:text-amber-300">
          added by AIfred
        </span>
      )}
      <button
        onClick={() => props.run(api.deleteTask(t.id))}
        title="Delete task"
        className="invisible text-stone-400 hover:text-red-600 group-hover:visible"
      >
        ✕
      </button>
    </li>
  );
}

/** A task that was just marked done in ClickUp, so the user can undo it.
 * The list keeps each one: a second tick must not lose the undo of the first. */
type Closed = { id: string; title: string; before: string };

export function TodoList(props: {
  tasks: api.Task[];
  github: api.GithubItem[];
  /** The ClickUp tasks. Leave out when ClickUp is not connected. */
  clickup?: api.ClickupItem[];
  /** "off" when ClickUp is not connected. */
  clickupState?: "off" | "loading" | "ok" | string;
  clickupFetchedAt?: number;
  clickupWarning?: string;
  /** A ClickUp task is done in ClickUp. Take it out of the list. */
  onClickupDone?: (id: string) => void;
  /** Load the ClickUp list from ClickUp again (after an undo). */
  onClickupReload?: () => void;
  /** When the GitHub list came from GitHub, in Unix seconds. */
  fetchedAt?: number;
  /** Set when GitHub could not be asked and the list is the older saved one. */
  githubWarning?: string;
  githubState: "loading" | "ok" | string;
  /** Set when the local tasks cannot be loaded. */
  tasksError?: string;
  onChange: () => void;
  onRefresh: () => void;
}) {
  const { tasks, github, githubState } = props;
  const [filter, setFilter] = useState<Filter>("all");
  const [draft, setDraft] = useState("");
  const [draftNotes, setDraftNotes] = useState("");
  const [showDone, setShowDone] = useState(false);
  const [error, setError] = useState("");
  /** ClickUp tasks that wait for ClickUp to answer, so they cannot be clicked twice. */
  const [closing, setClosing] = useState<string[]>([]);
  const [closed, setClosed] = useState<Closed[]>([]);
  const clickup = props.clickup ?? [];
  const clickupState = props.clickupState ?? "off";

  const open = tasks.filter((t) => !t.done).sort((a, b) => b.created - a.created);
  const done = tasks.filter((t) => t.done).sort((a, b) => (b.done_at ?? 0) - (a.done_at ?? 0));
  const showLocal = filter === "all" || filter === "local";
  const showGithub = filter === "all" || filter === "github";
  const showClickup = filter === "all" || filter === "clickup";
  const count =
    (showLocal ? open.length : 0) + (showGithub ? github.length : 0) + (showClickup ? clickup.length : 0);
  const doneCount = showLocal ? done.length : 0;

  /** Runs a change. Returns true if it worked. */
  async function run(p: Promise<unknown>): Promise<boolean> {
    try {
      setError("");
      await p;
      props.onChange();
      return true;
    } catch (e) {
      setError(String(e));
      return false;
    }
  }

  /** Closes the task in ClickUp. This changes the real task, so the row
   * offers an undo after it. */
  async function markClickupDone(c: api.ClickupItem) {
    setClosing((l) => [...l, c.id]);
    try {
      setError("");
      const before = await api.clickupMarkDone(c.id);
      setClosed((l) => [...l.filter((x) => x.id !== c.id), { id: c.id, title: c.title, before }]);
      props.onClickupDone?.(c.id);
    } catch (e) {
      setError(`ClickUp: ${e}`);
    } finally {
      setClosing((l) => l.filter((x) => x !== c.id));
    }
  }

  async function undoClickupDone(c: Closed) {
    try {
      setError("");
      await api.clickupUndoDone(c.id, c.before);
      setClosed((l) => l.filter((x) => x.id !== c.id));
      props.onClickupReload?.();
    } catch (e) {
      setError(`ClickUp undo failed: ${e}`);
    }
  }

  async function add() {
    const text = draft;
    const notes = draftNotes;
    if (!text.trim()) return;
    // Clear the boxes only when the add worked, so a failure keeps your text.
    // Keep anything you typed while the add was running.
    if (await run(api.addTask(text, notes))) {
      setDraft((d) => (d === text ? "" : d));
      setDraftNotes((d) => (d === notes ? "" : d));
    }
  }

  return (
    <section className="flex min-h-0 flex-col">
      <div className="flex items-center gap-2">
        <h2 className="text-sm font-semibold uppercase tracking-wide text-stone-500">To do</h2>
        <span className="text-sm text-stone-400">{count}</span>
        <div className="ml-auto flex items-center gap-1.5">
          {showGithub && githubState === "ok" && !!props.fetchedAt && (
            <span className="mr-1 text-xs text-stone-400" title="When the GitHub list last came from GitHub">
              GitHub {agoSecs(props.fetchedAt)}
            </span>
          )}
          {showClickup && clickupState === "ok" && !!props.clickupFetchedAt && (
            <span className="mr-1 text-xs text-stone-400" title="When the ClickUp list last came from ClickUp">
              ClickUp {agoSecs(props.clickupFetchedAt)}
            </span>
          )}
          <Chip active={filter === "all"} onClick={() => setFilter("all")}>All</Chip>
          <Chip active={filter === "local"} onClick={() => setFilter("local")}>Mine</Chip>
          <Chip active={filter === "github"} onClick={() => setFilter("github")}>GitHub</Chip>
          <Chip active={filter === "clickup"} onClick={() => setFilter("clickup")}>ClickUp</Chip>
          <button
            onClick={props.onRefresh}
            title="Reload the list"
            className="ml-1 rounded-full px-2 text-stone-500 hover:bg-stone-200 dark:hover:bg-stone-800"
          >
            ↻
          </button>
        </div>
      </div>

      {showLocal && (
        <input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && add()}
          placeholder="+ Add a task and press Enter"
          className="mt-3 rounded-lg border border-dashed border-stone-300 bg-transparent px-3 py-2 text-sm outline-none focus:border-amber-500 focus:border-solid dark:border-stone-700"
        />
      )}
      {/* The description box shows when you type a title. */}
      {showLocal && (draft.trim() || draftNotes) && (
        <textarea
          aria-label="New task description"
          value={draftNotes}
          rows={Math.max(2, draftNotes.split("\n").length)}
          onChange={(e) => setDraftNotes(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && (e.ctrlKey || e.metaKey) && add()}
          placeholder="Description (optional). Ctrl+Enter adds the task."
          className="mt-1.5 resize-none rounded-lg border border-dashed border-stone-300 bg-transparent px-3 py-2 text-xs outline-none focus:border-amber-500 focus:border-solid dark:border-stone-700"
        />
      )}
      {error && <div className="mt-2 text-sm text-red-600">{error}</div>}
      {closed.map((c) => (
        <div key={c.id} role="status" className="mt-2 flex items-center gap-2 text-sm text-stone-600 dark:text-stone-300">
          <span className="min-w-0 truncate">Closed in ClickUp: {c.title}</span>
          <button
            onClick={() => undoClickupDone(c)}
            aria-label={`Undo: ${c.title}`}
            className="text-amber-700 hover:underline dark:text-amber-400"
          >
            Undo
          </button>
          <button
            onClick={() => setClosed((l) => l.filter((x) => x.id !== c.id))}
            title="Hide this message"
            className="text-stone-400 hover:text-stone-600"
          >
            ✕
          </button>
        </div>
      ))}
      {showLocal && props.tasksError && (
        <div className="mt-2 text-sm text-red-600">Cannot load your tasks: {props.tasksError}</div>
      )}

      <ul className="mt-2 divide-y divide-stone-200 dark:divide-stone-800">
        {showLocal &&
          open.map((t) => <TaskRow key={t.id} task={t} run={run} />)}

        {/* GitHub items have no checkbox: AIfred only reads GitHub. An item
            leaves the list when it is closed on GitHub. */}
        {showGithub &&
          github.map((g) => (
            <li key={g.id} className="flex items-center gap-3 py-2.5">
              <span className="size-4 shrink-0" aria-hidden />
              <div className="min-w-0 flex-1">
                <a href={g.url} className="text-sm hover:underline">
                  {g.title}
                </a>
                <div className="truncate text-xs text-stone-500">
                  {g.repo}#{g.number} · {ago(g.updated)}
                </div>
                {/* Under the title, so many badges do not cut a long title short. */}
                {!!g.links?.length && (
                  <div className="mt-1 flex flex-wrap items-center gap-1">
                    <LinkBadges item={g} />
                  </div>
                )}
              </div>
              <span className={`rounded px-1.5 py-0.5 text-[11px] ${REASON[g.reason].cls}`}>
                {REASON[g.reason].label}
              </span>
            </li>
          ))}

        {/* The tick closes the task in ClickUp. This is the only change
            AIfred makes in ClickUp. */}
        {showClickup &&
          clickup.map((c) => {
            const due = c.due ? dueText(c.due) : null;
            return (
              <li key={`cu-${c.id}`} className="flex items-center gap-3 py-2.5">
                <input
                  type="checkbox"
                  checked={false}
                  disabled={closing.includes(c.id)}
                  onChange={() => markClickupDone(c)}
                  title="Close this task in ClickUp"
                  aria-label={`Close in ClickUp: ${c.title}`}
                  className="size-4 accent-amber-500 disabled:opacity-40"
                />
                <div className="min-w-0 flex-1">
                  <a href={c.url} className="text-sm hover:underline">
                    {c.title}
                  </a>
                  <div className="truncate text-xs text-stone-500">
                    {c.custom_id ? `${c.custom_id} · ` : ""}
                    {c.list} · {agoSecs(c.updated / 1000)}
                    {due && <span className={due.late ? "text-red-600" : ""}> · {due.text}</span>}
                    {c.priority && c.priority !== "normal" && (
                      <span className={PRIORITY[c.priority]}> · {c.priority}</span>
                    )}
                  </div>
                </div>
                <span
                  className="flex items-center gap-1 rounded bg-stone-100 px-1.5 py-0.5 text-[11px] text-stone-700 dark:bg-stone-800 dark:text-stone-300"
                  title={`ClickUp status in ${c.workspace}`}
                >
                  <span className="size-2 rounded-full" style={{ background: c.status_color || "#a8a29e" }} aria-hidden />
                  {c.status}
                </span>
              </li>
            );
          })}
      </ul>

      {showClickup && clickupState === "loading" && (
        <div className="py-3 text-sm text-stone-500 animate-pulse">Loading ClickUp tasks…</div>
      )}
      {showClickup && clickupState === "ok" && props.clickupWarning && (
        <div className="py-3 text-sm text-amber-700 dark:text-amber-400">{props.clickupWarning}</div>
      )}
      {showClickup && !["off", "loading", "ok"].includes(clickupState) && (
        <div className="py-3 text-sm text-red-600">ClickUp: {clickupState}</div>
      )}
      {filter === "clickup" && clickupState === "off" && (
        <div className="py-6 text-center text-sm text-stone-500">
          ClickUp is not connected. Add your ClickUp token on the Settings page.
        </div>
      )}

      {showGithub && githubState === "loading" && (
        <div className="py-3 text-sm text-stone-500 animate-pulse">Loading GitHub items…</div>
      )}
      {showGithub && githubState === "ok" && props.githubWarning && (
        <div className="py-3 text-sm text-amber-700 dark:text-amber-400">{props.githubWarning}</div>
      )}
      {showGithub && githubState !== "loading" && githubState !== "ok" && (
        <div className="py-3 text-sm text-red-600">GitHub: {githubState}</div>
      )}
      {count === 0 &&
        !(showGithub && githubState !== "ok") &&
        !(showClickup && clickupState !== "ok" && clickupState !== "off") &&
        !(filter === "clickup" && clickupState === "off") &&
        !(showLocal && props.tasksError) && (
          <div className="py-6 text-center text-sm text-stone-500">
            {showLocal ? "Nothing to do. Add a task above." : filter === "github" ? "Nothing from GitHub." : "Nothing from ClickUp."}
          </div>
        )}

      {doneCount > 0 && (
        <div className="mt-3">
          <button onClick={() => setShowDone((s) => !s)} className="text-xs text-stone-500 hover:underline">
            {showDone ? "Hide" : "Show"} done ({doneCount})
          </button>
          {showDone && (
            <ul className="mt-1">
              {showLocal && done.map((t) => (
                <li key={t.id} className="group flex items-center gap-3 py-1.5 text-stone-400">
                  <input
                    type="checkbox"
                    checked
                    onChange={() => run(api.setTaskDone(t.id, false))}
                    className="size-4 accent-amber-500"
                  />
                  <span className="flex-1 text-sm line-through">{t.title}</span>
                  <button
                    onClick={() => run(api.deleteTask(t.id))}
                    title="Delete task"
                    className="invisible hover:text-red-600 group-hover:visible"
                  >
                    ✕
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </section>
  );
}
