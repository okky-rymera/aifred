// Task context: AIfred's short notes about one item, from your answers or
// from AIfred's own look-up of the item text (context.rs).
// - Before the brief, AIfred looks up the items that it does not know
//   (context.rs). "Help AIfred understand" on the home page asks about the
//   items that it still has questions about, one item at a time, with
//   AIfred's own first question about the item. You talk about the item in
//   the card: it is the chat of the item. AIfred can ask more questions,
//   and it saves a short summary when it knows enough. One answer can also
//   change other items (context.rs). The talk stays on the screen until you
//   press "Next question". After the last one, AIfred writes the brief and
//   missions again with the new notes (App.tsx).
// - The first brief of a day with questions holds its missions (brief.rs).
//   After the last answer or skip, the app writes the brief again (App.tsx).
// - "What AIfred knows" in the item panel shows the notes, closed at the
//   start. You can change them or clear them, with Undo.
import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import { Composer, Messages } from "./Chat";
import { openQuestions, useApp } from "./ctx";
import { SRC_NAME } from "./items";

/** The talk about one item: AIfred's question, the chat of the item, and a
 * box for your answer. `title`: the title of the item, also when the item
 * went from the lists (AIfred marked it done). */
function AskTalk({ itemKey, title, question, onSend, onDone }: { itemKey: string; title: string; question: string; onSend: () => void; onDone: () => void }) {
  const { byKey, open, askItem, itemChat, loadItemChat, models, setModel } = useApp();
  const i = byKey.get(itemKey);
  const chat = itemChat(itemKey);
  useEffect(() => loadItemChat(itemKey), [itemKey, loadItemChat]);
  const send = async (text: string, files: api.FileRef[]) => {
    // The talk stays on the screen with AIfred's answer.
    onSend();
    const ok = await askItem(itemKey, text, files);
    onDone();
    return ok;
  };
  return (
    <div className="h-talk">
      <div className="h-q">
        {i ? (
          <button className="linkbtn ttl" onClick={() => open(itemKey)} title={`Open "${title}"`}>
            {title}
          </button>
        ) : (
          <span className="ttl">{title}</span>
        )}
        {i && <span className="hintx">· {SRC_NAME[i.src]}</span>}
      </div>
      <div className="d-asks" aria-label={`Talk about ${title}`}>
        <Messages
          list={[{ role: "intro", text: question }, ...chat.msgs]}
          busy={chat.busy}
          progress={chat.progress}
          onItem={open}
          hasItem={(k) => byKey.has(k)}
        />
      </div>
      <Composer
        id={`help-${itemKey}`}
        label={`Your answer about ${title}`}
        placeholder="For example: the sync bug from Ana, due Friday"
        sendLabel="Send"
        busy={chat.busy}
        model={models.item}
        onModel={(m) => setModel("item", m)}
        onSend={send}
      />
    </div>
  );
}

/** The items that today's brief asks about now (openQuestions in ctx.ts). */
export function useQuestions(): string[] {
  const { brief, byKey, today, skipped, githubState, clickupState } = useApp();
  return openQuestions(brief, byKey, today, skipped, githubState, clickupState);
}

/** True when the question card shows: AIfred has questions, or the last
 * talk stays on the screen. */
export function useHasQuestions(): boolean {
  const { talk } = useApp();
  return useQuestions().length > 0 || talk !== null;
}

/** True while the home page shows only the question card: the brief holds
 * its missions until the answers (brief.rs). A later brief of the day does
 * not hold, so the card shows above the missions. */
export function useAsking(): boolean {
  const { brief } = useApp();
  return useHasQuestions() && !!brief?.held;
}

/** The items of today's brief that AIfred cannot rank, one at a time.
 * After your answer, the talk stays on the screen with AIfred's reply,
 * also when AIfred knows enough now. "Next question" goes on.
 * Hidden when there are none, and when the brief on the screen is not
 * today's (for example just after midnight, before today's brief loads). */
export function HelpCard() {
  const { brief, reloadBrief, byKey, toast, itemChat, reloadClickup, talk, setTalk, noteAnswer, skipped, setSkipped } = useApp();
  // "Later" puts an item at the end of the line, on this screen only.
  const [later, setLater] = useState<string[]>([]);
  const all = useQuestions();
  // The title from the loaded lists, else from the brief.
  const titleOf = (k: string) => byKey.get(k)?.title ?? brief?.titles?.[k] ?? k;
  const keys = [...all.filter((k) => !later.includes(k)), ...later.filter((k) => all.includes(k))];
  // The item on the screen: the talk, else the first question.
  // AIfred's next question about an item: the first of its list.
  const firstQuestion = (k: string) => brief?.questions?.[k]?.[0] ?? brief?.question ?? "";
  const cur = talk ?? (keys.length ? { key: keys[0], title: titleOf(keys[0]), question: firstQuestion(keys[0]) } : null);
  // Questions that went because AIfred used your answer about another item.
  // Only just after an answer on this card, and in the same brief: an item
  // can also go because it closed elsewhere, or because a new brief came.
  const shown = useRef<{ ask: string[]; written?: number }>({ ask: [] });
  const answeredAt = useRef(0);
  const seen = useRef<{ day?: string; keys: Set<string> }>({ keys: new Set() });
  // It runs only when the asked items change. The other values that it
  // reads come from that same render, so they are current.
  useEffect(() => {
    const before = shown.current;
    const recent = Date.now() - answeredAt.current < 60_000;
    if (recent && before.written === brief?.written) {
      const gone = before.ask.filter((k) => !brief?.ask?.includes(k) && !skipped.includes(k) && byKey.has(k) && k !== cur?.key);
      for (const k of gone) {
        const t = byKey.get(k)!.title;
        // A talk in the item panel, or an answer about another item.
        toast(itemChat(k).msgs.length ? `AIfred knows now what "${t}" is.` : `AIfred used your answer for "${t}" too, so it does not ask about it.`);
      }
    }
    shown.current = { ask: brief?.ask ?? [], written: brief?.written };
  }, [brief?.ask]);
  // Each item that the card asked about today, for the count.
  if (seen.current.day !== brief?.day) seen.current = { day: brief?.day, keys: new Set() };
  for (const k of all) seen.current.keys.add(k);
  if (!cur) return null;
  const day = brief?.day;
  const key = cur.key;
  const busy = itemChat(key).busy;
  // AIfred knows enough about the item now, or the item is done.
  const answered = !all.includes(key);
  const item = byKey.get(key);
  const next = () => {
    setTalk(null);
    reloadBrief(day);
  };
  const skip = (key: string) => {
    setSkipped((s) => [...s, key]);
    setTalk(null);
    api.skipContextQuestion(key).then(
      () => {
        // The brief loads again: after the last question, it makes the missions.
        reloadBrief(day);
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
        });
      },
      (e) => {
        setSkipped((s) => s.filter((k) => k !== key));
        toast(`Cannot skip: ${e}`, undefined, true);
      },
    );
  };
  const left = all.filter((k) => k !== key).length;
  // The place of this question among all questions of the day: the answered
  // and skipped ones leave `all`, but they still count.
  const total = seen.current.keys.size;
  const count = answered
    ? left === 0
      ? "last question"
      : left === 1
        ? "1 more question"
        : `${left} more questions`
    : total <= 1
      ? "1 question"
      : `${total - all.length + 1} of ${total}`;
  return (
    <section className="waiting help" aria-label="Help AIfred understand">
      <div className="eyebrow">
        <span>✦ Help AIfred understand · {count}</span>
      </div>
      {!answered && (
        <div className="hintx">
          AIfred looked this item up and still has questions. {brief?.held ? "It makes today's missions after your answers." : "It updates the brief and missions after your answers."}
        </div>
      )}
      <AskTalk
        key={key}
        itemKey={key}
        title={cur.title}
        question={cur.question}
        onSend={() => {
          // The talk keeps the question that started it: AIfred asks the
          // next ones in its answers.
          setTalk({ key, title: cur.title, question: cur.question });
          noteAnswer();
        }}
        onDone={() => {
          answeredAt.current = Date.now();
          reloadBrief(day);
          // The answer can mark a ClickUp task done. The reload reads the
          // saved list (no download while it is new), and the home page
          // does not wait for it (homeLoading).
          reloadClickup();
        }}
      />
      {answered && !busy && (
        <div className="h-done" role="status">
          {!item || item.done ? `"${cur.title}" is done.` : `AIfred saved what it knows about "${cur.title}".`} You can tell it more, or go on.
        </div>
      )}
      <div className="ctl">
        {answered ? (
          <button className="sug" onClick={next} disabled={busy}>
            {left > 0 ? "Next question" : brief?.held ? "Make today's missions" : "Update the brief and missions"}
          </button>
        ) : (
          <>
            {keys.length > 1 && (
              <button
                className="linkbtn"
                onClick={() => {
                  setTalk(null);
                  setLater((l) => [...l.filter((k) => k !== key), key]);
                }}
                title="Ask about the next item first"
              >
                Later
              </button>
            )}
            <button className="linkbtn" onClick={() => skip(key)} title="AIfred does not ask about this item again">
              Skip
            </button>
          </>
        )}
      </div>
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
          {ctx ? `Written by ${ctx.by === "you" ? "you" : ctx.by === "lookup" ? "AIfred, from the item text" : "AIfred"}, ${when}.` : "Tell AIfred about this item in the chat below, or write the notes here."}
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
