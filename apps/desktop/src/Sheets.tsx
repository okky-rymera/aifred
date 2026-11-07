// Comments on the cells of a Google Sheet (plan step 8 in
// docs/plan-assistant.md). You give the link of a Sheet and a note of what
// to change. AIfred reads the Sheet and writes comments. You see each
// comment with the value of its cell, and you can change it or take it out.
// Only your click saves them, as comments on the cells. AIfred never
// changes a cell. Each save goes in the action log, with an Undo.
import * as api from "./api";
import { ActionList } from "./Drafts";
import { GOOGLE_APPS } from "./googleApps";
import type { Item } from "./items";
import { useSuggest } from "./useSuggest";

/** One comment of the proposal: `keep` is false when you took it out. */
type Row = api.SheetComment & { keep: boolean };

/** "Suggest comments" on an item, and AIfred's comments for it.
 * `log`: the Sheets actions of the item. `load` loads them again. */
export function SheetComments({ item: i, log, load }: { item: Item; log: api.Action[]; load: () => void }) {
  const feature = GOOGLE_APPS.sheets.feature;
  const { s, setS, link, setLink, note, setNote, show, allowed, suggest, save, toast } = useSuggest<api.SheetProposal, Row>(i, load, {
    draft: "sheet",
    feature,
    findLink: api.sheetLink,
    suggest: api.suggestSheetComments,
    rows: (p) => p.comments.map((c) => ({ ...c, keep: true })),
    save: (p, rows) =>
      api.saveSheetComments(
        p,
        rows.filter((r) => r.keep && r.text.trim()).map(({ cell, text, value }) => ({ cell, text, value })),
      ),
    // The title says when Google gave no id for a comment.
    saved: (a) => `${a.title}. No cell changed: the owner makes the changes.`,
  });

  if (!show && !log.length) return null;
  let body: React.ReactNode = null;
  if (i.done) {
    body = null;
  } else if (s.kind === "idle" || s.kind === "error") {
    body = (
      <div className="suggest">
        <input className="addstep" aria-label="Sheet link" value={link} placeholder="Link of the Google Sheet (docs.google.com/spreadsheets/d/…)" onChange={(e) => setLink(e.target.value)} />
        <input className="addstep" aria-label="What to change in the Sheet" value={note} placeholder="What to change in the Sheet" onChange={(e) => setNote(e.target.value)} />
        <div className="suggest-acts">
          <button className="linkbtn" disabled={!link.trim()} onClick={suggest} title="Reads the tab of the link, and makes one AI call. The AI gets only the Sheet, the title of this item, and your note. It saves nothing.">
            ✦ Suggest comments
          </button>
          {s.kind === "error" && <span className="d-err">Cannot suggest comments: {s.text}</span>}
        </div>
        {!allowed && <span className="hintx">To suggest comments, connect Google again in Settings, and allow "{feature}".</span>}
      </div>
    );
  } else if (s.kind === "busy") {
    body = <span className="hintx">AIfred is reading the Sheet…</span>;
  } else {
    const { p, rows } = s;
    const set = (k: number, change: Partial<Row>) => setS({ ...s, rows: rows.map((r, j) => (j === k ? { ...r, ...change } : r)) });
    const kept = rows.filter((r) => r.keep && r.text.trim()).length;
    body = (
      <div className="suggest" role="group" aria-label="Sheet comments">
        <span className="hintx">
          In:{" "}
          <a className="linkbtn" href={p.url}>
            {p.title || "the Sheet"}
            {p.tab ? `, ${p.tab}` : ""} ↗
          </a>
        </span>
        {p.message && <span className="hintx">{p.message}</span>}
        {!rows.length && <span className="hintx">AIfred found no comment to suggest.</span>}
        {rows.map((r, k) => (
          <div key={k} className="docedit" role="group" aria-label={`Comment on ${r.cell}`}>
            <label className="hintx">
              <input type="checkbox" checked={r.keep} disabled={s.saving} onChange={(e) => set(k, { keep: e.target.checked })} /> Keep the comment on {r.cell}
            </label>
            <div className="cellnow" aria-label="Cell now">
              Now: {r.value || "(empty)"}
            </div>
            {/* Read-only while it saves: the save sends the text of the click. */}
            <textarea className="d-text reply" aria-label="Comment" value={r.text} readOnly={s.saving} onChange={(e) => set(k, { text: e.target.value })} />
          </div>
        ))}
        {s.error && <span className="d-err">Cannot save: {s.error}</span>}
        <div className="suggest-acts">
          <button
            className="btn pri"
            // One save at a time: a second click does not add the comments twice.
            disabled={s.saving || kept === 0}
            onClick={() => save(p, rows)}
            title="Saves the kept comments on their cells. AIfred does not change a cell."
          >
            Save as comments
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
      <div className="d-sec">Sheet</div>
      {body}
      {log.length > 0 && <ActionList list={log} place="item" label="AIfred's comments on this item" onChange={load} report={(text, error) => toast(text, undefined, error)} />}
    </>
  );
}
