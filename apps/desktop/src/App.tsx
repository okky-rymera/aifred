import { useCallback, useEffect, useRef, useState } from "react";
import Markdown from "react-markdown";
import remarkGfm from "remark-gfm";
import { openUrl } from "@tauri-apps/plugin-opener";
import * as api from "./api";
import { TodoList } from "./Todo";
import { SettingsPage } from "./Settings";

type Message = {
  role: "user" | "assistant" | "error";
  text: string;
  meta?: string;
};

const BRIEF_PROMPT =
  "What needs my attention right now? Give me a short TL;DR list, most urgent first.";

const SUGGESTIONS = [
  { label: "Brief me", prompt: BRIEF_PROMPT },
  { label: "What should I review?", prompt: "Which pull requests wait for my review? One line each." },
  { label: "Plan my day", prompt: "Look at my to-do list, GitHub items, and ClickUp tasks. Suggest an order for today." },
];

function formatMeta(a: api.Answer): string {
  const parts = [`${(a.stats.total_ms / 1000).toFixed(1)}s`];
  const n = a.stats.tool_calls.length;
  if (n) parts.push(`${n} lookup${n === 1 ? "" : "s"}`);
  if (a.cost_usd != null) parts.push(`~$${a.cost_usd.toFixed(3)} usage`);
  return parts.join(" · ");
}

function ChatInput(props: { busy: boolean; onSend: (text: string) => void; big?: boolean; autoFocus?: boolean }) {
  const [input, setInput] = useState("");
  function send() {
    if (!input.trim() || props.busy) return;
    props.onSend(input);
    setInput("");
  }
  return (
    <div className="flex items-end gap-2">
      <textarea
        value={input}
        autoFocus={props.autoFocus}
        onChange={(e) => setInput(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && !e.shiftKey) {
            e.preventDefault();
            send();
          }
        }}
        rows={props.big ? 2 : 1}
        placeholder={props.big ? "Ask AIfred anything about your work…" : "Ask AIfred…"}
        className={`max-h-40 flex-1 resize-none rounded-xl border border-stone-300 bg-white px-4 outline-none focus:border-amber-500 dark:border-stone-700 dark:bg-stone-900 ${
          props.big ? "py-3 text-[15px] shadow-sm" : "py-2.5 text-sm"
        }`}
      />
      <button
        onClick={send}
        disabled={props.busy || !input.trim()}
        className="h-10 rounded-xl bg-[#252e4a] px-4 text-sm font-medium text-stone-50 hover:bg-[#323d62] disabled:opacity-40"
      >
        Send
      </button>
    </div>
  );
}

export default function App() {
  const [messages, setMessages] = useState<Message[]>([]);
  const [progress, setProgress] = useState<string[]>([]);
  const [busy, setBusy] = useState(false);
  const [chatOpen, setChatOpen] = useState(false);
  const [info, setInfo] = useState<api.Info | null>(null);
  const [memory, setMemory] = useState("");
  const [memorySaved, setMemorySaved] = useState(true);
  const [showMemory, setShowMemory] = useState(false);
  const [tasks, setTasks] = useState<api.Task[]>([]);
  const [tasksError, setTasksError] = useState("");
  const [memoryError, setMemoryError] = useState("");
  const [loadError, setLoadError] = useState("");
  const [github, setGithub] = useState<api.GithubList>({ items: [], fetched_at: 0 });
  const [settings, setSettings] = useState<api.Settings | null>(null);
  const [page, setPage] = useState<"home" | "settings">("home");
  const [githubState, setGithubState] = useState("loading");
  const [clickup, setClickup] = useState<api.ClickupList>({ items: [], fetched_at: 0 });
  const [clickupState, setClickupState] = useState("loading");
  const [clickupToken, setClickupToken] = useState<api.ClickupTokenStatus | null>(null);
  /** Counts ClickUp loads. Only the newest load may change the screen. */
  const clickupLoad = useRef(0);
  const bottom = useRef<HTMLDivElement>(null);
  /** Counts GitHub loads. Only the newest load may change the screen. */
  const githubLoad = useRef(0);

  const loadTasks = useCallback(() => {
    api
      .tasks()
      .then((t) => {
        setTasks(t);
        setTasksError("");
      })
      .catch((e) => setTasksError(String(e)));
  }, []);

  /** force: ask GitHub now. Without force, the app can use its saved list. */
  /** Loads can overlap (reload, then a tick). An older answer must not
   * replace a newer one, so we drop all but the answer to the newest load. */
  const loadGithub = useCallback((force: boolean) => {
    const n = ++githubLoad.current;
    setGithubState("loading");
    api
      .githubItems(force)
      .then((list) => {
        if (n !== githubLoad.current) return;
        setGithub(list);
        setGithubState("ok");
      })
      .catch((e) => {
        if (n !== githubLoad.current) return;
        setGithubState(String(e));
      });
  }, []);

  /** Like loadGithub. Does nothing when ClickUp is not connected. */
  const loadClickup = useCallback((force: boolean, connected: boolean) => {
    const n = ++clickupLoad.current;
    if (!connected) {
      setClickup({ items: [], fetched_at: 0 });
      setClickupState("off");
      return;
    }
    setClickupState("loading");
    api
      .clickupItems(force)
      .then((list) => {
        if (n !== clickupLoad.current) return;
        setClickup(list);
        setClickupState("ok");
      })
      .catch((e) => {
        if (n !== clickupLoad.current) return;
        setClickupState(String(e));
      });
  }, []);

  /** Reads whether ClickUp is connected, then loads its list. */
  const checkClickup = useCallback(
    (force: boolean) => {
      api
        .clickupTokenStatus()
        .then((t) => {
          setClickupToken(t);
          loadClickup(force, t.set);
        })
        .catch((e) => setClickupState(String(e)));
    },
    [loadClickup],
  );
  const clickupOn = clickupToken?.set ?? false;

  useEffect(() => {
    const failed = (what: string) => (e: unknown) => setLoadError(`Cannot load ${what}: ${e}`);
    api
      .history()
      .then((turns) => setMessages(turns.map((t) => ({ role: t.role, text: t.text }))))
      .catch(failed("the conversation"));
    api.info().then(setInfo).catch(failed("app info"));
    api.getMemory().then(setMemory).catch(failed("memory"));
    api
      .getSettings()
      .then(setSettings)
      .catch((e) => {
        failed("settings")(e);
        setSettings({ refresh_minutes: 15 });
      });
    loadTasks();
    loadGithub(false);
    checkClickup(false);
  }, [loadTasks, loadGithub, checkClickup]);

  // Reload the GitHub list on the timer from the settings. 0 means never.
  const refreshMinutes = settings?.refresh_minutes ?? 0;
  useEffect(() => {
    if (refreshMinutes <= 0) return;
    const timer = setInterval(() => {
      loadGithub(true);
      loadClickup(true, clickupOn);
    }, refreshMinutes * 60_000);
    return () => clearInterval(timer);
  }, [refreshMinutes, loadGithub, loadClickup, clickupOn]);

  useEffect(() => {
    bottom.current?.scrollIntoView({ behavior: "smooth" });
  }, [messages, progress, chatOpen]);

  async function send(prompt: string) {
    const text = prompt.trim();
    if (!text || busy) return;
    setChatOpen(true);
    setBusy(true);
    setProgress([]);
    setMessages((m) => [...m, { role: "user", text }]);
    try {
      const answer = await api.ask(text, (e) => setProgress((p) => [...p, api.describe(e)]));
      setMessages((m) => [...m, { role: "assistant", text: answer.text, meta: formatMeta(answer) }]);
    } catch (e) {
      setMessages((m) => [...m, { role: "error", text: String(e) }]);
    } finally {
      setBusy(false);
      setProgress([]);
      // AIfred can add or finish tasks while it answers.
      loadTasks();
    }
  }

  async function startNew() {
    try {
      await api.newConversation();
      setMessages([]);
      setChatOpen(false);
    } catch (e) {
      setMessages((m) => [...m, { role: "error", text: String(e) }]);
    }
  }

  async function saveMemory() {
    try {
      await api.setMemory(memory);
      setMemorySaved(true);
      setMemoryError("");
    } catch (e) {
      setMemoryError(`Not saved: ${e}`);
    }
  }

  // Open links in the system browser, not inside the app window.
  function onClick(e: React.MouseEvent) {
    const a = (e.target as HTMLElement).closest("a");
    if (a?.href.startsWith("http")) {
      e.preventDefault();
      openUrl(a.href);
    }
  }

  return (
    <div className="flex h-full" onClick={onClick}>
      {/* Left sidebar */}
      <aside className="flex w-60 shrink-0 flex-col gap-4 border-r border-stone-200 bg-stone-100/60 p-4 dark:border-stone-800 dark:bg-stone-900/60">
        <button
          onClick={() => {
            setChatOpen(false);
            setPage("home");
          }}
          className="flex items-center gap-2 text-left"
          title="Home"
        >
          <div className="grid size-8 place-items-center rounded-lg bg-[#252e4a]">
            <div className="size-3.5 rounded-full border-[3px] border-amber-400" />
          </div>
          <div>
            <div className="font-semibold leading-tight">AIfred</div>
            <div className="text-xs text-stone-500">{info?.backend ?? "…"}</div>
          </div>
        </button>

        <button
          onClick={startNew}
          disabled={busy}
          className="rounded-lg border border-stone-300 px-3 py-2 text-sm hover:bg-stone-200 disabled:opacity-50 dark:border-stone-700 dark:hover:bg-stone-800"
        >
          New conversation
        </button>

        <div className="flex min-h-0 flex-1 flex-col">
          <button
            onClick={() => setShowMemory((s) => !s)}
            className="flex items-center justify-between text-left text-xs font-semibold uppercase tracking-wide text-stone-500"
          >
            What AIfred knows about you
            <span>{showMemory ? "−" : "+"}</span>
          </button>
          {showMemory && (
            <div className="mt-2 flex min-h-0 flex-1 flex-col gap-2">
              <textarea
                value={memory}
                onChange={(e) => {
                  setMemory(e.target.value);
                  setMemorySaved(false);
                }}
                placeholder="- I work on the StoreAgent plugin."
                className="min-h-40 flex-1 resize-none rounded-lg border border-stone-300 bg-white p-2 font-mono text-xs dark:border-stone-700 dark:bg-stone-950"
              />
              <button
                onClick={saveMemory}
                disabled={memorySaved}
                className="rounded-lg border border-stone-300 px-3 py-1.5 text-xs hover:bg-stone-200 disabled:opacity-40 dark:border-stone-700 dark:hover:bg-stone-800"
              >
                {memorySaved ? "Saved" : "Save memory"}
              </button>
              {memoryError && <div className="text-xs text-red-600">{memoryError}</div>}
            </div>
          )}
        </div>

        <button
          onClick={() => setPage("settings")}
          className="text-left text-xs font-semibold uppercase tracking-wide text-stone-500 hover:text-stone-800 dark:hover:text-stone-200"
        >
          Settings
        </button>

        <div className="space-y-1 text-[11px] text-stone-500">
          {loadError && <div className="text-red-600">{loadError}</div>}
          {info?.api_key_set && (
            <div className="text-red-600">ANTHROPIC_API_KEY is set: Claude uses the API key, not your login.</div>
          )}
          <div>GitHub: {info?.github_owners ? `only ${info.github_owners}` : "all owners"}</div>
          <div className="truncate" title={info?.data_dir}>
            Data: {info?.data_dir}
          </div>
        </div>
      </aside>

      {/* Home */}
      <main className="min-w-0 flex-1 overflow-y-auto">
        <div className="mx-auto max-w-3xl px-6 py-8">
          {page === "settings" && settings && (
            <SettingsPage
              settings={settings}
              onSaved={setSettings}
              onBack={() => setPage("home")}
              clickupToken={clickupToken}
              onClickupTokenChanged={() => checkClickup(true)}
            />
          )}
          {page === "home" && !chatOpen && (
            <div className="mb-10">
              <div className="mb-4 text-2xl font-semibold">Good to see you.</div>
              <ChatInput big autoFocus busy={busy} onSend={send} />
              <div className="mt-3 flex flex-wrap gap-2">
                {SUGGESTIONS.map((s) => (
                  <button
                    key={s.label}
                    onClick={() => send(s.prompt)}
                    disabled={busy}
                    className="rounded-full border border-stone-300 px-3 py-1 text-xs text-stone-600 hover:bg-stone-200 disabled:opacity-50 dark:border-stone-700 dark:text-stone-300 dark:hover:bg-stone-800"
                  >
                    {s.label}
                  </button>
                ))}
                {messages.length > 0 && (
                  <button
                    onClick={() => setChatOpen(true)}
                    className="ml-auto text-xs text-amber-700 hover:underline dark:text-amber-400"
                  >
                    Continue the conversation ({messages.length} messages) →
                  </button>
                )}
              </div>
            </div>
          )}
          {page === "home" && (
            <TodoList
              tasks={tasks}
              github={github.items}
              fetchedAt={github.fetched_at}
              githubWarning={github.warning}
              githubState={githubState}
              tasksError={tasksError}
              clickup={clickup.items}
              clickupState={clickupState}
              clickupFetchedAt={clickup.fetched_at}
              clickupWarning={clickup.warning}
              onClickupDone={(id) => setClickup((l) => ({ ...l, items: l.items.filter((i) => i.id !== id) }))}
              onClickupReload={() => loadClickup(true, clickupOn)}
              onChange={loadTasks}
              onRefresh={() => {
                loadTasks();
                loadGithub(true);
                loadClickup(true, clickupOn);
              }}
            />
          )}
        </div>
      </main>

      {/* Right sidebar: the conversation, after the first message */}
      {chatOpen && (
        <aside aria-label="Conversation" className="flex w-[440px] shrink-0 flex-col border-l border-stone-200 dark:border-stone-800">
          <div className="flex items-center gap-2 border-b border-stone-200 px-4 py-2.5 dark:border-stone-800">
            <span className="text-sm font-semibold">Conversation</span>
            <button
              onClick={startNew}
              disabled={busy}
              className="ml-auto rounded px-2 py-1 text-xs text-stone-500 hover:bg-stone-200 disabled:opacity-40 dark:hover:bg-stone-800"
            >
              New
            </button>
            <button
              onClick={() => setChatOpen(false)}
              title="Close the conversation panel"
              className="rounded px-2 py-1 text-stone-500 hover:bg-stone-200 dark:hover:bg-stone-800"
            >
              ✕
            </button>
          </div>

          <div className="flex-1 space-y-5 overflow-y-auto px-4 py-4">
            {messages.map((m, i) =>
              m.role === "user" ? (
                <div key={i} className="flex justify-end">
                  <div className="max-w-[85%] whitespace-pre-wrap rounded-2xl rounded-br-md bg-[#252e4a] px-4 py-2 text-sm text-stone-50">
                    {m.text}
                  </div>
                </div>
              ) : m.role === "error" ? (
                <div key={i} className="rounded-lg border border-red-300 bg-red-50 px-4 py-2 text-sm text-red-800 dark:border-red-900 dark:bg-red-950 dark:text-red-200">
                  {m.text}
                </div>
              ) : (
                <div key={i}>
                  <div className="md text-sm leading-relaxed">
                    <Markdown remarkPlugins={[remarkGfm]}>{m.text}</Markdown>
                  </div>
                  {m.meta && <div className="mt-2 text-xs text-stone-400">{m.meta}</div>}
                </div>
              ),
            )}
            {busy && (
              <div className="space-y-1 text-sm text-stone-500">
                {progress.map((p, i) => (
                  <div key={i}>✓ {p}</div>
                ))}
                <div className="animate-pulse">Thinking…</div>
              </div>
            )}
            <div ref={bottom} />
          </div>

          <div className="border-t border-stone-200 p-3 dark:border-stone-800">
            <ChatInput autoFocus busy={busy} onSend={send} />
          </div>
        </aside>
      )}
    </div>
  );
}
