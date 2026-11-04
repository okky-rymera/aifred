import { useState } from "react";
import * as api from "./api";

const REFRESH_CHOICES = [
  { minutes: 5, label: "Every 5 minutes" },
  { minutes: 15, label: "Every 15 minutes" },
  { minutes: 30, label: "Every 30 minutes" },
  { minutes: 60, label: "Every hour" },
  { minutes: 0, label: "Only when I click reload" },
];

/** Connects or disconnects ClickUp. It has its own buttons: the token is
 * checked with ClickUp and saved at once, apart from the other settings. */
function ClickupSection(props: { status: api.ClickupTokenStatus | null; onChanged: () => void }) {
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);

  async function run(p: Promise<string | void>, done: (r: string | void) => string) {
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
        AIfred shows the open ClickUp tasks assigned to you. The chat can read them. Ticking a task in the list closes
        it in ClickUp, with an undo.
      </p>
      {status?.from_env ? (
        <p className="text-sm">Connected. The token comes from AIFRED_CLICKUP_TOKEN, so change it there.</p>
      ) : status?.set ? (
        <div className="flex items-center gap-3">
          <span className="text-sm">Connected.</span>
          <button
            onClick={() => run(api.removeClickupToken(), () => "Disconnected. The token is removed from this computer.")}
            disabled={busy}
            className="rounded-lg border border-stone-300 px-3 py-1.5 text-sm hover:bg-stone-200 disabled:opacity-40 dark:border-stone-700 dark:hover:bg-stone-800"
          >
            Disconnect
          </button>
        </div>
      ) : (
        <>
          <p className="text-sm text-stone-500">
            In ClickUp, open Settings &gt; Apps, and copy your API token (it starts with pk_). AIfred keeps it in its
            data folder, readable only by you.
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
              onClick={() => run(api.setClickupToken(token), (name) => `Connected as ${name || "you"}.`)}
              disabled={busy || !token.trim()}
              className="rounded-lg bg-[#252e4a] px-4 py-2 text-sm font-medium text-stone-50 hover:bg-[#323d62] disabled:opacity-40"
            >
              {busy ? "Checking…" : "Connect"}
            </button>
          </div>
        </>
      )}
      {message && (
        <p role="status" className={`text-sm ${message.error ? "text-red-600" : "text-stone-500"}`}>
          {message.text}
        </p>
      )}
    </section>
  );
}

export function SettingsPage(props: {
  settings: api.Settings;
  onSaved: (s: api.Settings) => void;
  onBack: () => void;
  clickupToken?: api.ClickupTokenStatus | null;
  onClickupTokenChanged?: () => void;
}) {
  const [draft, setDraft] = useState(props.settings);
  const [status, setStatus] = useState("");
  const changed = draft.refresh_minutes !== props.settings.refresh_minutes;

  async function save() {
    try {
      await api.setSettings(draft);
      props.onSaved(draft);
      setStatus("Saved");
    } catch (e) {
      setStatus(`Not saved: ${e}`);
    }
  }

  return (
    <div>
      <div className="mb-6 flex items-center gap-3">
        <button onClick={props.onBack} className="text-sm text-stone-500 hover:underline">
          ← Back
        </button>
        <h1 className="text-2xl font-semibold">Settings</h1>
      </div>

      <section className="space-y-2">
        <label htmlFor="github-refresh" className="block text-sm font-semibold">
          Reload GitHub and ClickUp items
        </label>
        <p className="text-sm text-stone-500">
          AIfred keeps the last GitHub and ClickUp lists. Between reloads, the lists come from that copy.
        </p>
        <select
          id="github-refresh"
          value={draft.refresh_minutes}
          onChange={(e) => {
            setDraft({ ...draft, refresh_minutes: Number(e.target.value) });
            setStatus("");
          }}
          className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm dark:border-stone-700 dark:bg-stone-900"
        >
          {REFRESH_CHOICES.map((c) => (
            <option key={c.minutes} value={c.minutes}>
              {c.label}
            </option>
          ))}
        </select>
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
          <span className={`text-sm ${status === "Saved" ? "text-stone-500" : "text-red-600"}`}>{status}</span>
        )}
      </div>

      <ClickupSection status={props.clickupToken ?? null} onChanged={() => props.onClickupTokenChanged?.()} />
    </div>
  );
}
