// "What AIfred knows about you": the list of memory facts in the sidebar.
// AIfred also adds and forgets facts in the chat, so the app loads the
// list again after each answer (App.tsx).
import { useRef, useState } from "react";
import * as api from "./api";

type Props = {
  facts: api.Fact[];
  /** Puts the new or changed fact in the list. */
  onChange: (f: api.Fact) => void;
  onAdd: (f: api.Fact) => void;
  /** Takes a fact that was deleted for good out of the list. */
  onDelete: (id: number) => void;
};

type RowProps = {
  fact: api.Fact;
  onChange: (f: api.Fact) => void;
  onError: (e: string) => void;
  /** Shows only the facts with this tag. */
  onPickTag: (tag: string) => void;
};

function FactRow({ fact, onChange, onError, onPickTag }: RowProps) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(fact.text);
  // The tags as one line of text, "testing, people", while the user edits them.
  const [tagText, setTagText] = useState<string | null>(null);
  // True when a box was closed by Escape or a save. A box that closes can
  // lose focus after that, and the blur must not save it (again): Escape
  // cancels, and the old text in the blur handler is not the saved text.
  const textClosed = useRef(false);
  const tagsClosed = useRef(false);
  // True while a save of the box runs: Enter and a blur (a click away) do
  // not send it again. One Enter and a quick click away save one time.
  const textSaving = useRef(false);
  const tagsSaving = useRef(false);
  // The value of the last failed save. A blur does not send the same value
  // again, so the error does not come back on each click away. Enter can
  // try again. Opening the box clears it.
  const textFailed = useRef<string | null>(null);
  const tagsFailed = useRef<string | null>(null);

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

  function closeText() {
    textClosed.current = true;
    setEditing(false);
  }

  function closeTags() {
    tagsClosed.current = true;
    setTagText(null);
  }

  async function save(blur = false) {
    if (textClosed.current || textSaving.current) return;
    if (blur && textFailed.current === text) return;
    if (text.trim() === fact.text) return closeText();
    textSaving.current = true;
    const saved = await run(api.editFact(fact.id, text));
    textSaving.current = false;
    textFailed.current = saved ? null : text;
    if (saved) closeText();
  }

  async function saveTags(blur = false) {
    if (tagsClosed.current || tagText === null || tagsSaving.current) return;
    if (blur && tagsFailed.current === tagText) return;
    const tags = tagText.split(",").map((t) => t.trim()).filter(Boolean);
    if (tags.join(",") === fact.tags.join(",")) return closeTags();
    tagsSaving.current = true;
    const saved = await run(api.setFactTags(fact.id, tags));
    tagsSaving.current = false;
    tagsFailed.current = saved ? null : tagText;
    if (saved) closeTags();
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
              closeText();
            }
          }}
          onBlur={() => save(true)}
          className="w-full resize-none rounded border border-stone-300 bg-white px-1 py-0.5 dark:border-stone-700 dark:bg-stone-950"
        />
      ) : (
        // The box starts with the saved text. The saved text can differ from
        // what was typed (the backend joins the lines of a list item), and
        // AIfred can change the fact in the chat.
        <button
          onClick={() => {
            setText(fact.text);
            textClosed.current = false;
            textFailed.current = null;
            setEditing(true);
          }}
          title="Edit" className="w-full whitespace-pre-wrap text-left">
          {fact.text}
        </button>
      )}
      {tagText !== null ? (
        <input
          autoFocus
          aria-label={`Tags: ${fact.text}`}
          value={tagText}
          placeholder="testing, people"
          onChange={(e) => setTagText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") saveTags();
            if (e.key === "Escape") closeTags();
          }}
          onBlur={() => saveTags(true)}
          className="mt-1 w-full rounded border border-stone-300 bg-white px-1 py-0.5 text-[11px] dark:border-stone-700 dark:bg-stone-950"
        />
      ) : (
        fact.tags.length > 0 && (
          <div className="mt-1 flex flex-wrap gap-1">
            {fact.tags.map((t) => (
              <button
                key={t}
                onClick={() => onPickTag(t)}
                title={`Show only the facts with #${t}`}
                className="rounded bg-stone-100 px-1 text-[11px] text-stone-600 hover:bg-stone-200 dark:bg-stone-800 dark:text-stone-300 dark:hover:bg-stone-700"
              >
                #{t}
              </button>
            ))}
          </div>
        )
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
          onClick={() => {
            tagsClosed.current = false;
            tagsFailed.current = null;
            setTagText(fact.tags.join(", "));
          }}
          aria-label={`Edit tags: ${fact.text}`}
          title="Topic words, with commas between them. AIfred can read all facts with one tag."
          className="hover:text-stone-800 dark:hover:text-stone-200"
        >
          Tags
        </button>
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

export function MemoryList({ facts, onChange, onAdd, onDelete }: Props) {
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  const [showForgotten, setShowForgotten] = useState(false);
  const [tag, setTag] = useState<string | null>(null);
  // The forgotten fact whose "Delete for good" was clicked once. The
  // second click deletes it: there is no undo.
  const [sure, setSure] = useState<number | null>(null);
  const [note, setNote] = useState("");
  const inUse = facts.filter((f) => f.forgotten === null);
  // Each tag with its number of facts, the most used first (like memory::tags).
  const counts = new Map<string, number>();
  for (const f of inUse) for (const t of f.tags) counts.set(t, (counts.get(t) ?? 0) + 1);
  const tags = [...counts].sort((a, b) => b[1] - a[1] || a[0].localeCompare(b[0]));
  // A tag that no fact has any more shows all facts again, and the filter
  // goes away: a later fact with that tag must not turn it on again.
  if (tag && !counts.has(tag)) setTag(null);
  const active = tag && counts.has(tag) ? tag : null;
  const shown = active ? inUse.filter((f) => f.tags.includes(active)) : inUse;
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

  async function remove(f: api.Fact) {
    if (sure !== f.id) return setSure(f.id);
    setSure(null);
    try {
      const cleaned = await api.deleteFact(f.id);
      onDelete(f.id);
      setError("");
      setNote(cleaned ? "" : "Deleted. Another AIfred program was using the database, so an old copy of the text can stay in the database file. The next delete cleans the file again.");
    } catch (e) {
      setError(`Not deleted: ${e}`);
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
      {note && <div className="text-xs text-stone-500">{note}</div>}
      {tags.length > 0 && (
        <div role="group" aria-label="Filter by tag" className="flex flex-wrap gap-1 text-[11px]">
          <button
            onClick={() => setTag(null)}
            aria-pressed={active === null}
            className={`rounded px-1.5 ${active === null ? "bg-stone-800 text-white dark:bg-stone-200 dark:text-stone-900" : "bg-stone-100 text-stone-600 dark:bg-stone-800 dark:text-stone-300"}`}
          >
            All
          </button>
          {tags.map(([t, n]) => (
            <button
              key={t}
              onClick={() => setTag(active === t ? null : t)}
              aria-pressed={active === t}
              className={`rounded px-1.5 ${active === t ? "bg-stone-800 text-white dark:bg-stone-200 dark:text-stone-900" : "bg-stone-100 text-stone-600 dark:bg-stone-800 dark:text-stone-300"}`}
            >
              #{t} {n}
            </button>
          ))}
        </div>
      )}
      <ul className="flex min-h-0 flex-1 flex-col gap-1.5 overflow-y-auto">
        {inUse.length === 0 && <li className="text-xs text-stone-500">Nothing yet. Tell AIfred about yourself.</li>}
        {shown.map((f) => (
          <FactRow key={f.id} fact={f} onChange={onChange} onError={setError} onPickTag={setTag} />
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
                  <button
                    onClick={() => remove(f)}
                    onBlur={() => setSure((s) => (s === f.id ? null : s))}
                    aria-label={sure === f.id ? `Click again to delete for good: ${f.text}` : `Delete for good: ${f.text}`}
                    title="Removes the fact from AIfred's database. You cannot restore it. Old chat messages that mention it stay."
                    className={sure === f.id ? "font-medium text-red-600" : "hover:text-red-600"}
                  >
                    {sure === f.id ? "Sure? Delete" : "Delete"}
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
