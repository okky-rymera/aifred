// Task context: AIfred's short notes about one item, from
// your answers (context.rs).
// - "Help AIfred understand" on the home page asks about at most 3 items
//   that AIfred cannot rank. You answer in the chat of the item, and
//   AIfred saves a short summary.
// - "What AIfred knows" in the item panel shows the notes, closed at the
//   start. You can change them or clear them, with Undo.
import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import { SRC_NAME } from "./items";

function AskRow({ itemKey, onSkip }: { itemKey: string; onSkip: () => void }) {
  const { byKey, open, askItem, itemChat } = useApp();
  const [text, setText] = useState("");
  const i = byKey.get(itemKey)!;
  const busy = itemChat(itemKey).busy;
  const send = async () => {
    const t = text.trim();
    if (!t) return;
    // Open the panel first: it loads the old messages of the item before
    // the new pair is saved, so the pair does not show two times.
    open(itemKey);
    setText("");
    // False: the answer failed. The text comes back, unless you typed again.
    if (!(await askItem(itemKey, t, []))) setText((x) => x || t);
  };
  return (
    <li className="h-row">
      <div className="h-q">
        <button className="linkbtn ttl" onClick={() => open(itemKey)} title={`Open "${i.title}"`}>
          {i.title}
        </button>
        <span className="hintx">· {SRC_NAME[i.src]}</span>
      </div>
      <div className="h-ans">
        <input
          aria-label={`Your answer about ${i.title}`}
          placeholder="For example: the sync bug from Ana, due Friday"
          value={text}
          disabled={busy}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => e.key === "Enter" && send()}
        />
        <button className="sug" disabled={busy || !text.trim()} onClick={send}>
          {busy ? "AIfred reads…" : "Send"}
        </button>
        <button className="linkbtn" onClick={onSkip} title="AIfred does not ask about this item again">
          Skip
        </button>
      </div>
    </li>
  );
}

/** The items of today's brief that AIfred cannot rank. Hidden when there are
 * none, and when the brief on the screen is not today's (for example just
 * after midnight, before today's brief loads). */
export function HelpCard() {
  const { brief, reloadBrief, byKey, toast, today } = useApp();
  // Skipped here: they go at once, before the brief loads again.
  const [skipped, setSkipped] = useState<string[]>([]);
  const keys =
    brief?.day === today ? (brief.ask ?? []).filter((k) => !skipped.includes(k) && byKey.has(k) && !byKey.get(k)!.done) : [];
  if (!keys.length) return null;
  const day = brief?.day;
  const skip = (key: string) => {
    setSkipped((s) => [...s, key]);
    api.skipContextQuestion(key).then(
      () =>
        toast("AIfred does not ask about this item again.", () => {
          // The brief can have loaded again since the skip, without this
          // item: load it again, so that the question comes back.
          api
            .skipContextQuestion(key, true)
            .then(() => {
              setSkipped((s) => s.filter((k) => k !== key));
              reloadBrief(day);
            })
            .catch((e) => toast(`Cannot undo: ${e}`, undefined, true));
        }),
      (e) => {
        setSkipped((s) => s.filter((k) => k !== key));
        toast(`Cannot skip: ${e}`, undefined, true);
      },
    );
  };
  return (
    <section className="waiting help" aria-label="Help AIfred understand">
      <div className="eyebrow">
        <span>✦ Help AIfred understand · {keys.length}</span>
      </div>
      <div className="hintx">AIfred cannot rank these items well. {brief?.question}</div>
      <ul className="w-list">
        {keys.map((k) => (
          <AskRow key={k} itemKey={k} onSkip={() => skip(k)} />
        ))}
      </ul>
    </section>
  );
}

/** True when two loads of the notes give the same notes. */
const sameNotes = (a: api.ItemContext | null, b: api.ItemContext | null) =>
  a === b || (!!a && !!b && a.text === b.text && a.by === b.by && a.updated === b.updated);

/** "What AIfred knows" in the item panel. `version` changes after each chat
 * answer about the item: AIfred can change the notes in it. -1: an answer is
 * on its way. Give the panel `key={itemKey}`: one panel is for one item. */
export function ItemContextSection({ itemKey, version }: { itemKey: string; version: number }) {
  const { toast, brief, reloadBrief } = useApp();
  const [ctx, setCtx] = useState<api.ItemContext | null | undefined>(undefined);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState("");
  // The notes of the last load. undefined: no load came back yet. A chat
  // answer that changes them shows a message with an undo.
  const last = useRef<api.ItemContext | null | undefined>(undefined);
  // The loads run one after the other, and each one shows its result. So a
  // load does not show old notes after a newer load, and the message for
  // each chat answer shows, also when you send the next message at once.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  const day = brief?.day;

  /** Loads the notes and shows them. It gives the loaded notes.
   * - `notify`: the load comes after a chat answer. A change since the last
   *   load is AIfred's (the background jobs cannot change notes), so a
   *   message with an undo shows.
   * - `reset`: the load comes after your own Save or Clear, so the box shows
   *   the saved text. Else a box that you changed and did not save keeps
   *   your text. */
  const load = (notify: boolean, reset = false): Promise<api.ItemContext | null | undefined> => {
    const run = queue.current.then(() =>
      api.itemContext(itemKey).then(
        (c) => {
          if (!mounted.current) return c;
          const before = last.current;
          if (notify && before !== undefined && !sameNotes(before, c)) {
            toast(
              c ? "AIfred saved what it knows about this item." : "AIfred cleared what it knows about this item.",
              () => undo(before),
            );
          }
          last.current = c;
          setCtx(c);
          setDraft((d) => (reset || before === undefined || d.trim() === (before?.text ?? "") ? (c?.text ?? "") : d));
          setError("");
          return c;
        },
        (e) => {
          if (mounted.current) setError(String(e));
          return undefined;
        },
      ),
    );
    queue.current = run;
    return run;
  };
  // A change of the notes changes what the home page asks about. The undo
  // keeps text that you typed in the box and did not save.
  const undo = (before: api.ItemContext | null) =>
    api
      .restoreItemContext(itemKey, before)
      .then(() => load(false))
      .then(() => reloadBrief(day))
      .catch((e) => toast(`Cannot undo: ${e}`, undefined, true));

  // The first load. An answer that starts at once (from the home page) does
  // not stop it: it gives the notes from before the answer.
  useEffect(() => {
    load(false);
  }, []);
  // A load after each chat answer. The first version is the first load.
  const firstVersion = useRef(true);
  useEffect(() => {
    if (firstVersion.current) {
      firstVersion.current = false;
      return;
    }
    if (version !== -1) load(true);
  }, [version]);

  const save = async (text: string, done: string) => {
    try {
      const before = await api.setItemContext(itemKey, text);
      const c = await load(false, true);
      reloadBrief(day);
      // AIfred keeps at most 600 characters (context.rs). The box shows
      // what it kept.
      const cut = !!text.trim() && !!c && c.text !== text.trim();
      toast(cut ? `${done} The text was too long, so AIfred cut its end.` : done, () => undo(before));
    } catch (e) {
      toast(`Cannot save: ${e}`, undefined, true);
    }
  };
  const changed = draft.trim() !== (ctx?.text ?? "");
  const when = ctx
    ? new Date(ctx.updated * 1000).toLocaleDateString([], {
        day: "numeric",
        month: "short",
      })
    : "";
  return (
    <details className="d-ctx">
      <summary className="d-sec">
        What AIfred knows
        {ctx === null && <span className="c">nothing yet</span>}
      </summary>
      <div className="d-ctx-body">
        {error && <div className="d-err">{error}</div>}
        <div className="hintx">
          Only AIfred reads these notes. It uses them to rank this item.{" "}
          {ctx ? `Written by ${ctx.by === "you" ? "you" : "AIfred"}, ${when}.` : "Tell AIfred about this item in the chat below, or write the notes here."}
        </div>
        <textarea
          className="d-text"
          aria-label="What AIfred knows"
          placeholder="What the item is, how it links to other work, and how urgent it is"
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
        />
        <div className="ctl">
          <button className="sug" disabled={!changed} onClick={() => save(draft, "Saved what AIfred knows.")}>
            Save
          </button>
          {ctx && (
            <button className="linkbtn" onClick={() => save("", "Cleared what AIfred knows.")}>
              Clear
            </button>
          )}
        </div>
      </div>
    </details>
  );
}
