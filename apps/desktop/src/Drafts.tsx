// Replies to emails (plan step 8 in docs/plan-assistant.md). AIfred writes
// a reply to an email that waits for your answer. You read it and change
// it. Only your click saves it, as a draft in the Gmail thread. AIfred
// never sends. Each saved draft goes in the action log, with an Undo.
import { useCallback, useEffect, useRef, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import type { Item } from "./items";

const when = (secs: number) => new Date(secs * 1000).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });

/** The Undo calls that run now, by action id. */
const running = new Map<number, Promise<api.Action>>();

/** Calls Undo of an action. A second click while it runs (in the list or
 * in the toast) gets the same result: it does not start a second Undo. */
function undo(id: number): Promise<api.Action> {
  let p = running.get(id);
  if (!p) {
    p = api.undoAction(id).finally(() => running.delete(id));
    running.set(id, p);
  }
  return p;
}

/** The state of an action, in words. */
function stateText(a: api.Action) {
  if (a.undone_at === null) return "";
  return a.note ? `Undo: ${a.note}` : `Undone ${when(a.undone_at)}`;
}

/** What an Undo did. The note says when AIfred deleted nothing (the draft was sent or gone). */
export const undoneText = (a: api.Action) => a.note || "Deleted the draft in Gmail.";

/** The look of the list: in the item panel, or on the Settings page. */
const LOOK = {
  item: { list: "actions", row: "", title: "st", hint: "hintx", link: "linkbtn", button: "linkbtn", open: "Open in Gmail ↗" },
  settings: { list: "space-y-1 text-sm", row: "flex flex-wrap items-baseline gap-x-2", title: "", hint: "text-stone-500", link: "underline", button: "underline", open: "Open in Gmail" },
};

/** AIfred's actions, with Undo. `report` shows what an Undo did. */
function ActionList(props: { list: api.Action[]; place: keyof typeof LOOK; label?: string; onChange: () => void; report: (text: string, error: boolean) => void }) {
  const c = LOOK[props.place];
  // The actions whose Undo runs now: their button is off until it ends.
  const [busy, setBusy] = useState<number[]>([]);
  const click = (id: number) => {
    setBusy((b) => [...b, id]);
    undo(id)
      .then((u) => props.report(undoneText(u), false))
      .catch((e) => props.report(`Cannot undo: ${e}`, true))
      .finally(() => {
        setBusy((b) => b.filter((x) => x !== id));
        props.onChange();
      });
  };
  return (
    <ul className={c.list} aria-label={props.label}>
      {props.list.map((a) => (
        <li key={a.id} className={c.row || undefined}>
          <span className={c.title || undefined}>{a.title}</span>
          <span className={c.hint}>{when(a.at)}</span>
          {a.url && (
            <a className={c.link} href={a.url}>
              {c.open}
            </a>
          )}
          {a.undone_at === null ? (
            <button className={c.button} disabled={busy.includes(a.id)} title="Deletes the draft in Gmail. AIfred does not delete a draft that you changed there." onClick={() => click(a.id)}>
              {busy.includes(a.id) ? "Undoing…" : "Undo"}
            </button>
          ) : (
            <span className={c.hint}>{stateText(a)}</span>
          )}
        </li>
      ))}
    </ul>
  );
}

type State =
  | { kind: "idle" }
  | { kind: "busy"; key: string }
  | { kind: "error"; text: string }
  | { kind: "edit"; draft: api.ReplyDraft; saving?: boolean; error?: string };

/** "Write a reply" on an email item, and AIfred's drafts for it. */
export function ReplyDraft({ item: i }: { item: Item }) {
  const { toast } = useApp();
  const [s, setS] = useState<State>({ kind: "idle" });
  const [note, setNote] = useState(i.next ?? "");
  const [log, setLog] = useState<api.Action[]>([]);
  const email = i.src === "gm";
  const shown = useRef(i.key);
  shown.current = i.key;
  const load = useCallback(() => {
    if (!email) return;
    const key = i.key;
    // A list that comes after you opened another item is dropped.
    api
      .actions(key)
      .then((l) => shown.current === key && setLog(l))
      .catch(() => shown.current === key && setLog([]));
  }, [i.key, email]);
  // Another item in the panel starts with no reply.
  useEffect(() => {
    setS({ kind: "idle" });
    setNote(i.next ?? "");
    setLog([]);
    load();
  }, [i.key, load]); // eslint-disable-line react-hooks/exhaustive-deps
  if (!email) return null;

  const write = () => {
    const key = i.key;
    setS({ kind: "busy", key });
    const mine = (old: State) => old.kind === "busy" && old.key === key;
    api
      .writeReply(key, note)
      .then((draft) => setS((old) => (mine(old) ? { kind: "edit", draft } : old)))
      .catch((e) => setS((old) => (mine(old) ? { kind: "error", text: String(e) } : old)));
  };

  const save = async (draft: api.ReplyDraft) => {
    setS({ kind: "edit", draft, saving: true });
    // The result changes the panel only when it still shows this save:
    // you can open another item and write there in the meantime.
    const mine = (old: State) => old.kind === "edit" && !!old.saving && old.draft.key === draft.key;
    try {
      const a = await api.saveReplyDraft(draft);
      setS((old) => (mine(old) ? { kind: "idle" } : old));
      toast("Saved the reply as a draft in Gmail. It is not sent.", () => {
        undo(a.id)
          .then((u) => toast(undoneText(u)))
          .catch((e) => toast(`Cannot undo: ${e}`, undefined, true))
          .finally(load);
      });
    } catch (e) {
      setS((old) => (mine(old) ? { ...old, saving: false, error: String(e) } : old));
    }
    load();
  };

  let body: React.ReactNode = null;
  if (i.done) {
    body = null;
  } else if (s.kind === "idle" || s.kind === "error") {
    body = (
      <div className="suggest">
        <input className="addstep" aria-label="What to say" value={note} placeholder="What to say (you can leave it empty)" onChange={(e) => setNote(e.target.value)} />
        <div className="suggest-acts">
          <button className="linkbtn" onClick={write} title="One AI call. AIfred reads only this email and your note. It saves nothing.">
            ✦ Write a reply
          </button>
          {s.kind === "error" && <span className="d-err">Cannot write a reply: {s.text}</span>}
        </div>
      </div>
    );
  } else if (s.kind === "busy") {
    body = <span className="hintx">AIfred is writing a reply…</span>;
  } else {
    const d = s.draft;
    body = (
      <div className="suggest" role="group" aria-label="Reply draft">
        <span className="hintx">
          To: {d.to_name === d.to ? d.to : `${d.to_name} <${d.to}>`} · {d.subject}
        </span>
        {/* Read-only while it saves: the save sends the text of the click, so a change typed now would be lost. */}
        <textarea className="d-text reply" aria-label="Reply text" value={d.body} readOnly={s.saving} onChange={(e) => setS({ ...s, draft: { ...d, body: e.target.value } })} />
        {!d.can_save && <span className="hintx">To save it in Gmail, tick "Use Gmail drafts" in Settings, then connect Google again. You can copy the text now.</span>}
        {s.error && <span className="d-err">Cannot save: {s.error}</span>}
        <div className="suggest-acts">
          <button
            className="btn pri"
            // One save at a time: a second click does not make a second draft.
            disabled={!d.can_save || s.saving || !d.body.trim()}
            onClick={() => save(d)}
            title="Saves this text as a draft in the Gmail thread. AIfred does not send it."
          >
            Save as Gmail draft
          </button>
          <button className="linkbtn" onClick={() => navigator.clipboard?.writeText(d.body)}>
            Copy text
          </button>
          <button className="linkbtn" onClick={() => setS({ kind: "idle" })}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  if (!body && !log.length) return null;
  return (
    <>
      <div className="d-sec">Reply</div>
      {body}
      {log.length > 0 && <ActionList list={log} place="item" label="AIfred's actions on this item" onChange={load} report={(text, error) => toast(text, undefined, error)} />}
    </>
  );
}

/** The log of AIfred's actions in other apps, on the Settings page. */
export function ActionLogSection() {
  const [list, setList] = useState<api.Action[] | null>(null);
  // What an Undo did. The error of the list is apart: a list that loads
  // again removes it, and it does not remove the result of an Undo.
  const [message, setMessage] = useState<{ text: string; error: boolean } | null>(null);
  const [loadError, setLoadError] = useState("");
  const load = useCallback(() => {
    api
      .actions()
      .then((l) => {
        setList(l);
        setLoadError("");
      })
      .catch((e) => setLoadError(`Cannot show the actions: ${e}`));
  }, []);
  useEffect(load, [load]);
  return (
    <section className="mt-8 space-y-2" aria-label="AIfred's actions">
      <h2 className="text-sm font-semibold">AIfred's actions</h2>
      <p className="text-sm text-stone-500">
        Each change that AIfred made in another app, newest first. Now AIfred makes only Gmail drafts, and only when you click "Save as Gmail draft". Undo deletes the draft, when you did not change it in Gmail.
      </p>
      {list && !list.length && <p className="text-sm text-stone-500">No actions yet.</p>}
      {list && list.length > 0 && <ActionList list={list} place="settings" onChange={load} report={(text, error) => setMessage({ text, error })} />}
      {loadError && <p className="text-sm text-red-600">{loadError}</p>}
      {message && (
        <p role="status" className={`text-sm ${message.error ? "text-red-600" : "text-stone-500"}`}>
          {message.text}
        </p>
      )}
    </section>
  );
}
