import { useState } from "react";
import * as api from "./api";

type Filter = "all" | "local" | "github";

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

export function TodoList(props: {
  tasks: api.Task[];
  github: api.GithubItem[];
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
  const [showDone, setShowDone] = useState(false);
  const [error, setError] = useState("");

  const open = tasks.filter((t) => !t.done).sort((a, b) => b.created - a.created);
  const done = tasks.filter((t) => t.done).sort((a, b) => (b.done_at ?? 0) - (a.done_at ?? 0));
  const showLocal = filter !== "github";
  const showGithub = filter !== "local";
  const count = (showLocal ? open.length : 0) + (showGithub ? github.length : 0);
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

  async function add() {
    const text = draft;
    if (!text.trim()) return;
    // Clear the box only when the add worked, so a failure keeps your text.
    // Keep anything you typed while the add was running.
    if (await run(api.addTask(text))) setDraft((d) => (d === text ? "" : d));
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
          <Chip active={filter === "all"} onClick={() => setFilter("all")}>All</Chip>
          <Chip active={filter === "local"} onClick={() => setFilter("local")}>Mine</Chip>
          <Chip active={filter === "github"} onClick={() => setFilter("github")}>GitHub</Chip>
          <Chip disabled>ClickUp</Chip>
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
      {error && <div className="mt-2 text-sm text-red-600">{error}</div>}
      {showLocal && props.tasksError && (
        <div className="mt-2 text-sm text-red-600">Cannot load your tasks: {props.tasksError}</div>
      )}

      <ul className="mt-2 divide-y divide-stone-200 dark:divide-stone-800">
        {showLocal &&
          open.map((t) => (
            <li key={t.id} className="group flex items-center gap-3 py-2.5">
              <input
                type="checkbox"
                checked={false}
                onChange={() => run(api.setTaskDone(t.id, true))}
                className="size-4 accent-amber-500"
              />
              <div className="min-w-0 flex-1">
                <div className="text-sm">{t.title}</div>
                {t.notes && <div className="truncate text-xs text-stone-500">{t.notes}</div>}
              </div>
              {t.added_by === "aifred" && (
                <span className="rounded bg-amber-100 px-1.5 py-0.5 text-[11px] text-amber-800 dark:bg-amber-950 dark:text-amber-300">
                  added by AIfred
                </span>
              )}
              <button
                onClick={() => run(api.deleteTask(t.id))}
                title="Delete task"
                className="invisible text-stone-400 hover:text-red-600 group-hover:visible"
              >
                ✕
              </button>
            </li>
          ))}

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
      </ul>

      {showGithub && githubState === "loading" && (
        <div className="py-3 text-sm text-stone-500 animate-pulse">Loading GitHub items…</div>
      )}
      {showGithub && githubState === "ok" && props.githubWarning && (
        <div className="py-3 text-sm text-amber-700 dark:text-amber-400">{props.githubWarning}</div>
      )}
      {showGithub && githubState !== "loading" && githubState !== "ok" && (
        <div className="py-3 text-sm text-red-600">GitHub: {githubState}</div>
      )}
      {count === 0 && !(showGithub && githubState !== "ok") && !(showLocal && props.tasksError) && (
        <div className="py-6 text-center text-sm text-stone-500">
          {showLocal ? "Nothing to do. Add a task above." : "Nothing from GitHub."}
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
