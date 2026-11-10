import { useRef, useState } from "react";
import * as api from "./api";
import { GoogleSection } from "./Google";
import { ActionLogSection } from "./Drafts";
import { BriefJob } from "./Jobs";
import { ClickupStatusSection } from "./Status";

const REFRESH_CHOICES = [
  { minutes: 5, label: "Every 5 minutes" },
  { minutes: 15, label: "Every 15 minutes" },
  { minutes: 30, label: "Every 30 minutes" },
  { minutes: 60, label: "Every hour" },
  { minutes: 0, label: "Only when I click reload" },
];

/** Connects or disconnects ClickUp. It has its own buttons: the token is
 * checked with ClickUp and saved at once, apart from the other settings. */
function ClickupSection(props: {
  status: api.ClickupTokenStatus | null;
  onChanged: () => void;
}) {
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{
    text: string;
    error: boolean;
  } | null>(null);

  async function run(
    p: Promise<string | void>,
    done: (r: string | void) => string,
  ) {
    setBusy(true);
    setMessage(null);
    try {
      const r = await p;
      setToken("");
      setMessage({ text: done(r), error: false });
      props.onChanged();
    } catch (e) {
      setMessage({ text: String(e), error: true });
    } finally {
      setBusy(false);
    }
  }

  const status = props.status;
  return (
    <section className="mt-8 space-y-2">
      <label htmlFor="clickup-token" className="block text-sm font-semibold">
        ClickUp
      </label>
      <p className="text-sm text-stone-500">
        AIfred shows the open ClickUp tasks assigned to you. The chat can read
        them. Ticking a task in the list closes it in ClickUp, with an undo.
      </p>
      {status?.from_env ? (
        <p className="text-sm">
          Connected. The token comes from AIFRED_CLICKUP_TOKEN, so change it
          there.
        </p>
      ) : status?.set ? (
        <div className="flex items-center gap-3">
          <span className="text-sm">Connected.</span>
          <button
            onClick={() =>
              run(
                api.removeClickupToken(),
                () => "Disconnected. The token is removed from this computer.",
              )
            }
            disabled={busy}
            className="rounded-lg border border-stone-300 px-3 py-1.5 text-sm hover:bg-stone-200 disabled:opacity-40 dark:border-stone-700 dark:hover:bg-stone-800"
          >
            Disconnect
          </button>
        </div>
      ) : (
        <>
          <p className="text-sm text-stone-500">
            In ClickUp, open Settings &gt; Apps, and copy your API token (it
            starts with pk_). AIfred keeps it in its data folder, readable only
            by you.
          </p>
          <div className="flex items-center gap-2">
            <input
              id="clickup-token"
              type="password"
              autoComplete="off"
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="pk_…"
              className="w-72 rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm dark:border-stone-700 dark:bg-stone-900"
            />
            <button
              onClick={() =>
                run(
                  api.setClickupToken(token),
                  (name) => `Connected as ${name || "you"}.`,
                )
              }
              disabled={busy || !token.trim()}
              className="rounded-lg bg-[#252e4a] px-4 py-2 text-sm font-medium text-stone-50 hover:bg-[#323d62] disabled:opacity-40"
            >
              {busy ? "Checking…" : "Connect"}
            </button>
          </div>
        </>
      )}
      {message && (
        <p
          role="status"
          className={`text-sm ${message.error ? "text-red-600" : "text-stone-500"}`}
        >
          {message.text}
        </p>
      )}
    </section>
  );
}

/** Shows the program version, checks for a new one, and restarts into it. */
export function UpdateSection(props: {
  status: api.UpdateStatus | null;
  onChanged: (s: api.UpdateStatus) => void;
  busy?: boolean;
}) {
  const [error, setError] = useState("");
  const s = props.status;
  if (!s) return null;
  const working = s.state === "checking" || s.state === "downloading";

  async function check() {
    setError("");
    props.onChanged({ ...s!, state: "checking" });
    try {
      // When a check already runs, the app waits for its end and sends the result.
      props.onChanged(await api.checkUpdate());
    } catch (e) {
      setError(String(e));
      // Do not show "Checking…" until the next status read.
      api
        .updateStatus()
        .then(props.onChanged)
        .catch(() => props.onChanged(s!));
    }
  }

  const text: Record<api.UpdateStatus["state"], string> = {
    off: s.message ?? "Updates are off.",
    idle: "AIfred checks for a new version after it starts, and then every 6 hours.",
    checking: "Checking for a new version…",
    downloading: "Downloading the new version…",
    none: "You have the newest version.",
    ready: `Version ${s.latest} is downloaded. Restart AIfred to use it. If you close AIfred, the next start uses it.`,
    error: s.message ?? "The update failed.",
  };

  return (
    <section className="mt-8 space-y-2">
      <h2 className="text-sm font-semibold">Updates</h2>
      <p className="text-sm text-stone-500">This is AIfred {s.current}.</p>
      <p
        role="status"
        className={`text-sm ${s.state === "error" ? "text-red-600" : "text-stone-500"}`}
      >
        {text[s.state]}
        {s.state === "ready" && s.page && (
          <>
            {" "}
            <a href={s.page} className="underline">
              What is new
            </a>
          </>
        )}
      </p>
      <div className="flex items-center gap-3">
        {s.state !== "off" && (
          <button
            onClick={check}
            disabled={working}
            className="rounded-lg border border-stone-300 px-3 py-1.5 text-sm hover:bg-stone-200 disabled:opacity-40 dark:border-stone-700 dark:hover:bg-stone-800"
          >
            Check now
          </button>
        )}
        {s.state === "ready" && (
          <RestartButton busy={props.busy} onError={setError} />
        )}
      </div>
      {error && <p className="text-sm text-red-600">{error}</p>}
    </section>
  );
}

/** Installs the downloaded version and starts it. The window closes and opens again. */
export function RestartButton(props: {
  busy?: boolean;
  onError: (e: string) => void;
  small?: boolean;
}) {
  const [restarting, setRestarting] = useState(false);
  return (
    <button
      onClick={async () => {
        setRestarting(true);
        props.onError("");
        try {
          await api.restartToUpdate();
        } catch (e) {
          props.onError(String(e));
          setRestarting(false);
        }
      }}
      disabled={props.busy || restarting}
      title={props.busy ? "Wait for the answer, then restart." : undefined}
      className={`rounded-lg bg-[#252e4a] font-medium text-stone-50 hover:bg-[#323d62] disabled:opacity-40 ${props.small ? "px-2 py-1 text-xs" : "px-4 py-2 text-sm"}`}
    >
      {restarting ? "Restarting…" : "Restart now"}
    </button>
  );
}

const INPUT =
  "rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm dark:border-stone-700 dark:bg-stone-900";

/** The tabs of the Settings page. Each groups settings of one kind. */
const TABS = [
  { id: "general", label: "General" },
  { id: "connections", label: "Connections" },
  { id: "statuses", label: "ClickUp statuses" },
  { id: "activity", label: "Activity" },
  { id: "about", label: "About" },
] as const;
type Tab = (typeof TABS)[number]["id"];

export function SettingsPage(props: {
  settings: api.Settings;
  onSaved: (s: api.Settings) => void;
  /** Facts about this app: data folder, GitHub owners, meaning search. */
  info?: api.Info | null;
  clickupToken?: api.ClickupTokenStatus | null;
  onClickupTokenChanged?: () => void;
  /** The Google connection changed: the calendar must load again. */
  onGoogleChanged?: () => void;
  /** A rule for a ClickUp status changed: the items must load again. */
  onStatusRulesChanged?: () => void;
  update?: api.UpdateStatus | null;
  onUpdateChanged?: (s: api.UpdateStatus) => void;
  busy?: boolean;
}) {
  const [draft, setDraft] = useState(props.settings);
  const [status, setStatus] = useState("");
  const changed = JSON.stringify(draft) !== JSON.stringify(props.settings);
  const set = (change: Partial<api.Settings>) => {
    setDraft({ ...draft, ...change });
    setStatus("");
  };

  async function save() {
    try {
      await api.setSettings(draft);
      props.onSaved(draft);
      setStatus("Saved");
    } catch (e) {
      setStatus(`Not saved: ${e}`);
    }
  }

  const clickup = !!(props.clickupToken?.set || props.clickupToken?.from_env);
  const [tab, setTab] = useState<Tab>("general");
  const tabRefs = useRef<(HTMLButtonElement | null)[]>([]);

  const panels: Record<Tab, React.ReactNode> = {
    general: (
      <div>
        <section className="space-y-2">
          <label
            htmlFor="github-refresh"
            className="block text-sm font-semibold"
          >
            Reload GitHub and ClickUp items
          </label>
          <p className="text-sm text-stone-500">
            AIfred keeps the last GitHub and ClickUp lists. Between reloads, the
            lists come from that copy.
          </p>
          <select
            id="github-refresh"
            value={draft.refresh_minutes}
            onChange={(e) => set({ refresh_minutes: Number(e.target.value) })}
            className={INPUT}
          >
            {REFRESH_CHOICES.map((c) => (
              <option key={c.minutes} value={c.minutes}>
                {c.label}
              </option>
            ))}
          </select>
        </section>

        <section className="mt-6 space-y-2">
          <h2 className="text-sm font-semibold">Your work day</h2>
          <p className="text-sm text-stone-500">
            The day plan puts your tasks between these times, around lunch.
          </p>
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <label className="flex items-center gap-2">
              Starts
              <input
                type="time"
                aria-label="Work day starts"
                value={draft.work_start}
                onChange={(e) => set({ work_start: e.target.value })}
                className={INPUT}
              />
            </label>
            <label className="flex items-center gap-2">
              Ends
              <input
                type="time"
                aria-label="Work day ends"
                value={draft.work_end}
                onChange={(e) => set({ work_end: e.target.value })}
                className={INPUT}
              />
            </label>
          </div>
          <div className="flex flex-wrap items-center gap-3 text-sm">
            <label className="flex items-center gap-2">
              Lunch at
              <input
                type="time"
                aria-label="Lunch starts"
                value={draft.lunch_start}
                onChange={(e) => set({ lunch_start: e.target.value })}
                className={INPUT}
              />
            </label>
            <label className="flex items-center gap-2">
              for
              <select
                aria-label="Lunch length"
                value={draft.lunch_minutes}
                onChange={(e) => set({ lunch_minutes: Number(e.target.value) })}
                className={INPUT}
              >
                {[0, 15, 30, 45, 60, 90].map((m) => (
                  <option key={m} value={m}>
                    {m === 0 ? "No lunch break" : `${m} minutes`}
                  </option>
                ))}
              </select>
            </label>
          </div>
        </section>

        <section className="mt-6 space-y-2">
          <h2 className="text-sm font-semibold">Brief</h2>
          <label className="flex items-center gap-2 text-sm">
            <input
              type="checkbox"
              checked={draft.auto_brief}
              onChange={(e) => set({ auto_brief: e.target.checked })}
              className="size-4 accent-amber-500"
            />
            Write the brief by itself once a day, when AIfred opens
          </label>
          <p className="text-sm text-stone-500">
            The brief is one AI call on your Claude login. You can always write
            it again from the home screen.
          </p>
          <BriefJob />
        </section>

        <div className="mt-6 flex items-center gap-3">
          <button
            onClick={save}
            disabled={!changed}
            className="rounded-lg bg-[#252e4a] px-4 py-2 text-sm font-medium text-stone-50 hover:bg-[#323d62] disabled:opacity-40"
          >
            Save
          </button>
          {status && (
            <span
              className={`text-sm ${status === "Saved" ? "text-stone-500" : "text-red-600"}`}
            >
              {status}
            </span>
          )}
        </div>
      </div>
    ),
    connections: (
      <div>
        <ClickupSection
          status={props.clickupToken ?? null}
          onChanged={() => props.onClickupTokenChanged?.()}
        />
        <GoogleSection onChanged={() => props.onGoogleChanged?.()} />
      </div>
    ),
    statuses: clickup ? (
      <ClickupStatusSection onChanged={props.onStatusRulesChanged} />
    ) : (
      <p className="text-sm text-stone-500">
        Connect ClickUp on the Connections tab first. Then you can change what
        each ClickUp status means in AIfred.
      </p>
    ),
    activity: <ActionLogSection />,
    about: (
      <div>
        <UpdateSection
          status={props.update ?? null}
          onChanged={(s) => props.onUpdateChanged?.(s)}
          busy={props.busy}
        />
        {props.info && (
          <section className="mt-8 space-y-1 text-sm text-stone-500">
            <h2 className="text-sm font-semibold text-stone-900 dark:text-stone-100">
              About this app
            </h2>
            <div>AI: {props.info.backend}</div>
            <div>
              GitHub:{" "}
              {props.info.github_owners
                ? `only ${props.info.github_owners}`
                : "all owners"}
            </div>
            <div title="Finds facts, chats, and tasks by meaning, not only by words.">
              Meaning search: {props.info.meaning_search}
            </div>
            <div className="break-all">Data: {props.info.data_dir}</div>
          </section>
        )}
      </div>
    ),
  };

  return (
    <div>
      <h1 className="title mb-4 text-2xl font-semibold">Settings</h1>
      <div
        role="tablist"
        aria-label="Settings groups"
        className="mb-6 flex flex-wrap gap-1 border-b border-stone-200 dark:border-stone-800"
      >
        {TABS.map((t, n) => (
          <button
            key={t.id}
            ref={(el) => {
              tabRefs.current[n] = el;
            }}
            role="tab"
            id={`settings-tab-${t.id}`}
            aria-selected={tab === t.id}
            aria-controls={`settings-panel-${t.id}`}
            tabIndex={tab === t.id ? 0 : -1}
            onClick={() => setTab(t.id)}
            onKeyDown={(e) => {
              const step =
                e.key === "ArrowRight" ? 1 : e.key === "ArrowLeft" ? -1 : 0;
              const to =
                e.key === "Home"
                  ? 0
                  : e.key === "End"
                    ? TABS.length - 1
                    : step
                      ? (n + step + TABS.length) % TABS.length
                      : -1;
              if (to < 0) return;
              e.preventDefault();
              setTab(TABS[to].id);
              tabRefs.current[to]?.focus();
            }}
            className={`-mb-px border-b-2 px-3 py-2 text-sm ${
              tab === t.id
                ? "border-[#252e4a] font-medium text-stone-900 dark:border-stone-200 dark:text-stone-100"
                : "border-transparent text-stone-500 hover:text-stone-800 dark:hover:text-stone-200"
            }`}
          >
            {t.label}
          </button>
        ))}
      </div>
      <div
        role="tabpanel"
        id={`settings-panel-${tab}`}
        aria-labelledby={`settings-tab-${tab}`}
      >
        {panels[tab]}
      </div>
    </div>
  );
}
