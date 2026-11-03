import { useState } from "react";
import * as api from "./api";

const REFRESH_CHOICES = [
  { minutes: 5, label: "Every 5 minutes" },
  { minutes: 15, label: "Every 15 minutes" },
  { minutes: 30, label: "Every 30 minutes" },
  { minutes: 60, label: "Every hour" },
  { minutes: 0, label: "Only when I click reload" },
];

export function SettingsPage(props: { settings: api.Settings; onSaved: (s: api.Settings) => void; onBack: () => void }) {
  const [draft, setDraft] = useState(props.settings);
  const [status, setStatus] = useState("");
  const changed = draft.github_refresh_minutes !== props.settings.github_refresh_minutes;

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
          Reload GitHub items
        </label>
        <p className="text-sm text-stone-500">
          aifred keeps the last GitHub list. Between reloads, the list comes from that copy.
        </p>
        <select
          id="github-refresh"
          value={draft.github_refresh_minutes}
          onChange={(e) => {
            setDraft({ ...draft, github_refresh_minutes: Number(e.target.value) });
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
    </div>
  );
}
