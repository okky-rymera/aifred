// "What AIfred knows about you": the list of memory facts in the sidebar.
// AIfred also adds and forgets facts in the chat, so the app loads the
// list again after each answer (App.tsx).
import { useState } from "react";
import * as api from "./api";

type Props = {
  facts: api.Fact[];
  /** Puts the new or changed fact in the list. */
  onChange: (f: api.Fact) => void;
  onAdd: (f: api.Fact) => void;
};

function FactRow({ fact, onChange, onError }: { fact: api.Fact; onChange: (f: api.Fact) => void; onError: (e: string) => void }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(fact.text);

  async function run(p: Promise<api.Fact>) {
    try {
      onChange(await p);
      onError("");
      return true;
    } catch (e) {
      onError(`Not saved: ${e}`);
      return false;
    }
  }

  async function save() {
    if (text.trim() === fact.text) return setEditing(false);
    if (await run(api.editFact(fact.id, text))) setEditing(false);
  }

  return (
    <li className="group rounded-lg border border-stone-200 p-2 text-xs dark:border-stone-800">
      {editing ? (
        // A fact with more than one line (for example a heading and its
        // text) gets a box with more than one line. Shift+Enter adds a line.
        <textarea
          autoFocus
          aria-label="Fact text"
          value={text}
          rows={Math.max(1, text.split("\n").length)}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && !e.shiftKey) {
              e.preventDefault();
              save();
            }
            if (e.key === "Escape") {
              setText(fact.text);
              setEditing(false);
            }
          }}
          onBlur={save}
          className="w-full resize-none rounded border border-stone-300 bg-white px-1 py-0.5 dark:border-stone-700 dark:bg-stone-950"
        />
      ) : (
        // The box starts with the saved text. The saved text can differ from
        // what was typed (the backend joins the lines of a list item), and
        // AIfred can change the fact in the chat.
        <button
          onClick={() => {
            setText(fact.text);
            setEditing(true);
          }}
          title="Edit" className="w-full whitespace-pre-wrap text-left">
          {fact.text}
        </button>
      )}
      <div className="mt-1 flex items-center gap-2 text-[11px] text-stone-500">
        <label className="flex items-center gap-1" title="On: the fact goes into every message. Off: AIfred finds it with a search, but only when your memory is large (more than about 8,000 characters). A smaller memory goes into every message in full.">
          <input
            type="checkbox"
            checked={fact.core}
            onChange={(e) => run(api.setFactCore(fact.id, e.target.checked))}
            aria-label={`Always use: ${fact.text}`}
          />
          Always
        </label>
        {fact.added_by === "aifred" && <span>by AIfred</span>}
        <button
          onClick={() => run(api.forgetFact(fact.id))}
          aria-label={`Forget: ${fact.text}`}
          className="ml-auto hover:text-red-600"
        >
          Forget
        </button>
      </div>
    </li>
  );
}

export function MemoryList({ facts, onChange, onAdd }: Props) {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const [showForgotten, setShowForgotten] = useState(false);
  const inUse = facts.filter((f) => f.forgotten === null);
  const forgotten = facts.filter((f) => f.forgotten !== null);

  async function add() {
    if (!draft.trim()) return;
    try {
      onAdd(await api.addFact(draft));
      setDraft("");
      setError("");
    } catch (e) {
      setError(`Not saved: ${e}`);
    }
  }

  async function restore(f: api.Fact) {
    try {
      onChange(await api.restoreFact(f.id));
      setError("");
    } catch (e) {
      setError(`Not saved: ${e}`);
    }
  }

  return (
    <div className="mt-2 flex min-h-0 flex-1 flex-col gap-2">
      <input
        value={draft}
        onChange={(e) => setDraft(e.target.value)}
        onKeyDown={(e) => e.key === "Enter" && add()}
        placeholder="Add a fact and press Enter"
        className="rounded-lg border border-stone-300 bg-white px-2 py-1.5 text-xs dark:border-stone-700 dark:bg-stone-950"
      />
      {error && <div className="text-xs text-red-600">{error}</div>}
      <ul className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto">
        {inUse.length === 0 && <li className="text-xs text-stone-500">Nothing yet. Tell AIfred about yourself.</li>}
        {inUse.map((f) => (
          <FactRow key={f.id} fact={f} onChange={onChange} onError={setError} />
        ))}
      </ul>
      {forgotten.length > 0 && (
        <div className="text-xs">
          <button onClick={() => setShowForgotten((s) => !s)} className="text-stone-500 hover:text-stone-800 dark:hover:text-stone-200">
            Forgotten ({forgotten.length}) {showForgotten ? "−" : "+"}
          </button>
          {showForgotten && (
            <ul className="mt-1 flex max-h-40 flex-col gap-1 overflow-y-auto">
              {forgotten.map((f) => (
                <li key={f.id} className="flex items-start gap-2 text-stone-500">
                  <span className="flex-1 line-through">{f.text}</span>
                  <button onClick={() => restore(f)} aria-label={`Restore: ${f.text}`} className="hover:text-stone-800 dark:hover:text-stone-200">
                    Restore
                  </button>
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
