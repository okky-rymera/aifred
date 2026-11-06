// Suggested changes in a Google Doc (plan step 8 in docs/plan-assistant.md).
// You give the link of a Doc and a note of what to change. AIfred reads the
// Doc and writes the changes. You see each change before and after, and you
// can change it or take it out. Only your click saves them, as suggestions
// in the Doc. AIfred never edits the Doc directly. Each save goes in the
// action log, with an Undo.
import { useEffect, useRef, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import { ActionList, saveWithUndo } from "./Drafts";
import { GOOGLE_APPS } from "./googleApps";
import type { Item } from "./items";

/** One change of the proposal: `keep` is false when you took it out. */
type Row = api.DocEdit & { keep: boolean };

type State =
  | { kind: "idle" }
  | { kind: "busy"; key: string }
  | { kind: "error"; text: string }
  | { kind: "edit"; p: api.DocProposal; rows: Row[]; saving?: boolean; error?: string };

/** "Suggest Doc changes" on an item, and AIfred's suggestions for it.
 * `log`: the Docs actions of the item. `load` loads them again. */
export function DocSuggest({ item: i, log, load }: { item: Item; log: api.Action[]; load: () => void }) {
  const { toast, google } = useApp();
  const [s, setS] = useState<State>({ kind: "idle" });
  const [link, setLink] = useState("");
  const [note, setNote] = useState("");
  // True when the item has a Doc link (in its description, or in another item of the same work).
  const [found, setFound] = useState(false);
  const feature = GOOGLE_APPS.docs.feature;
  // The part shows when Docs suggestions is ticked in Settings (also before
  // the login allows it), or when the item has a Doc.
  const show = i.draft === "doc" || found || (!!google && !google.off.includes(feature));
  // False when Docs suggestions is ticked, but the Google login does not
  // allow it yet: the user must connect Google again.
  const allowed = !google || google.features.some(([n, on]) => n === feature && on);
  const shown = useRef(i.key);
  shown.current = i.key;
  // Another item in the panel starts with no changes. Only a new item
  // clears the link and the note: marking it done or not done keeps them.
  useEffect(() => {
    setS({ kind: "idle" });
    setLink("");
    setNote(i.draft === "doc" ? (i.next ?? "") : "");
    setFound(false);
  }, [i.key]); // eslint-disable-line react-hooks/exhaustive-deps
  // The link of the item. A done item shows only the list of its
  // suggestions, so it does not need it.
  useEffect(() => {
    const key = i.key;
    if (i.done) return;
    api
      .docLink(key)
      .then((l) => {
        if (shown.current !== key || !l) return;
        // A link that you typed before this answer came stays.
        setLink((old) => (old.trim() ? old : l));
        setFound(true);
      })
      .catch(() => {});
  }, [i.key, i.done]); // eslint-disable-line react-hooks/exhaustive-deps

  const suggest = () => {
    const key = i.key;
    setS({ kind: "busy", key });
    const mine = (old: State) => old.kind === "busy" && old.key === key;
    api
      .suggestDocChanges(key, link, note)
      .then((p) => setS((old) => (mine(old) ? { kind: "edit", p, rows: p.edits.map((e) => ({ ...e, keep: true })) } : old)))
      .catch((e) => setS((old) => (mine(old) ? { kind: "error", text: String(e) } : old)));
  };

  const save = async (p: api.DocProposal, rows: Row[]) => {
    setS({ kind: "edit", p, rows, saving: true });
    // The result changes the panel only when it still shows this save.
    const mine = (old: State) => old.kind === "edit" && !!old.saving && old.p.key === p.key;
    const edits = rows.filter((r) => r.keep && r.replace !== r.find).map(({ find, replace, why }) => ({ find, replace, why }));
    try {
      await saveWithUndo(() => api.saveDocSuggestions(p, edits), "Saved the changes as suggestions in the Doc. The Doc did not change: the owner accepts or rejects them.", toast, load);
      setS((old) => (mine(old) ? { kind: "idle" } : old));
    } catch (e) {
      setS((old) => (mine(old) ? { ...old, saving: false, error: String(e) } : old));
    }
  };

  if (!show && !log.length) return null;
  let body: React.ReactNode = null;
  if (i.done) {
    body = null;
  } else if (s.kind === "idle" || s.kind === "error") {
    body = (
      <div className="suggest">
        <input className="addstep" aria-label="Doc link" value={link} placeholder="Link of the Google Doc (docs.google.com/document/d/…)" onChange={(e) => setLink(e.target.value)} />
        <input className="addstep" aria-label="What to change" value={note} placeholder="What to change in the Doc" onChange={(e) => setNote(e.target.value)} />
        <div className="suggest-acts">
          <button className="linkbtn" disabled={!link.trim()} onClick={suggest} title="Reads the Doc, and makes one AI call. The AI gets only the Doc, the title of this item, and your note. It saves nothing.">
            ✦ Suggest changes
          </button>
          {s.kind === "error" && <span className="d-err">Cannot suggest changes: {s.text}</span>}
        </div>
        {!allowed && <span className="hintx">To suggest changes, connect Google again in Settings, and allow "{GOOGLE_APPS.docs.feature}".</span>}
      </div>
    );
  } else if (s.kind === "busy") {
    body = <span className="hintx">AIfred is reading the Doc…</span>;
  } else {
    const { p, rows } = s;
    const set = (k: number, change: Partial<Row>) => setS({ ...s, rows: rows.map((r, j) => (j === k ? { ...r, ...change } : r)) });
    const kept = rows.filter((r) => r.keep && r.replace !== r.find).length;
    body = (
      <div className="suggest" role="group" aria-label="Doc changes">
        <span className="hintx">
          In:{" "}
          <a className="linkbtn" href={p.url}>
            {p.title || "the Doc"} ↗
          </a>
        </span>
        {p.message && <span className="hintx">{p.message}</span>}
        {!p.can_edit && (
          <span className="hintx">
            You can only comment on this Doc. Then Google cannot check that nobody changes the Doc during the save. If somebody types in the Doc at that moment, a suggestion can go to the wrong place. Save when nobody else edits the Doc.
          </span>
        )}
        {!rows.length && <span className="hintx">AIfred found no change to suggest.</span>}
        {rows.map((r, k) => (
          <div key={k} className="docedit" role="group" aria-label={`Change ${k + 1}`}>
            <label className="hintx">
              <input type="checkbox" checked={r.keep} disabled={s.saving} onChange={(e) => set(k, { keep: e.target.checked })} /> Keep this change{r.why ? `: ${r.why}` : ""}
            </label>
            <div className="before" aria-label="Before">
              {r.find}
            </div>
            {/* Read-only while it saves: the save sends the text of the click. */}
            <textarea className="d-text reply" aria-label="After" value={r.replace} readOnly={s.saving} onChange={(e) => set(k, { replace: e.target.value })} />
          </div>
        ))}
        {s.error && <span className="d-err">Cannot save: {s.error}</span>}
        <div className="suggest-acts">
          <button
            className="btn pri"
            // One save at a time: a second click does not make the suggestions twice.
            disabled={s.saving || kept === 0}
            onClick={() => save(p, rows)}
            title="Saves the kept changes as suggestions in the Doc. AIfred does not edit the Doc directly."
          >
            Save as suggestions
          </button>
          <button className="linkbtn" disabled={s.saving} onClick={() => setS({ kind: "idle" })}>
            Cancel
          </button>
        </div>
      </div>
    );
  }
  if (!body && !log.length) return null;
  return (
    <>
      <div className="d-sec">Doc</div>
      {body}
      {log.length > 0 && <ActionList list={log} place="item" label="AIfred's suggestions on this item" onChange={load} report={(text, error) => toast(text, undefined, error)} />}
    </>
  );
}
