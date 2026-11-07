// The state of a "Suggest" part in the item panel: the Doc part (Docs.tsx)
// and the Sheet part (Sheets.tsx). It keeps the link and the note, finds the
// link of the item, runs the AI call, and saves with an Undo. Each part
// shows its own rows.
import { useEffect, useRef, useState } from "react";
import type * as api from "./api";
import { useApp } from "./ctx";
import { saveWithUndo } from "./Drafts";
import type { Item } from "./items";

export type SuggestState<P, R> =
  | { kind: "idle" }
  | { kind: "busy"; key: string }
  | { kind: "error"; text: string }
  | { kind: "edit"; p: P; rows: R[]; saving?: boolean; error?: string };

type Options<P, R> = {
  /** The kind of draft that opens this part on an item. */
  draft: api.DraftKind;
  /** The Google feature in Settings, for example "Docs suggestions". */
  feature: string;
  /** The link of the item, or null. */
  findLink: (key: string) => Promise<string | null>;
  /** The AI call. It saves nothing. */
  suggest: (key: string, link: string, note: string) => Promise<P>;
  /** The rows that the user checks, from the AI's proposal. */
  rows: (p: P) => R[];
  /** Saves the kept rows, and gives the new action of the log. */
  save: (p: P, rows: R[]) => Promise<api.Action>;
  /** The toast after the save. */
  saved: string | ((a: api.Action) => string);
};

/** `load` loads the actions of the item again. */
export function useSuggest<P extends { key: string }, R>(i: Item, load: () => void, o: Options<P, R>) {
  const { toast, google } = useApp();
  const [s, setS] = useState<SuggestState<P, R>>({ kind: "idle" });
  const [link, setLink] = useState("");
  const [note, setNote] = useState("");
  // True when the item has a link (in its description, or in another item of the same work).
  const [found, setFound] = useState(false);
  // The part shows when the feature is ticked in Settings (also before the
  // login allows it), or when the item has a link.
  const show = i.draft === o.draft || found || (!!google && !google.off.includes(o.feature));
  // False when the feature is ticked, but the Google login does not allow
  // it yet: the user must connect Google again.
  const allowed = !google || google.features.some(([n, on]) => n === o.feature && on);
  const shown = useRef(i.key);
  shown.current = i.key;
  // Another item in the panel starts with no rows. Only a new item clears
  // the link and the note: marking it done or not done keeps them.
  useEffect(() => {
    setS({ kind: "idle" });
    setLink("");
    setNote(i.draft === o.draft ? (i.next ?? "") : "");
    setFound(false);
  }, [i.key]); // eslint-disable-line react-hooks/exhaustive-deps
  // The link of the item. A done item shows only the list of its actions,
  // so it does not need it.
  useEffect(() => {
    const key = i.key;
    if (i.done) return;
    o.findLink(key)
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
    const mine = (old: SuggestState<P, R>) => old.kind === "busy" && old.key === key;
    o.suggest(key, link, note)
      .then((p) => setS((old) => (mine(old) ? { kind: "edit", p, rows: o.rows(p) } : old)))
      .catch((e) => setS((old) => (mine(old) ? { kind: "error", text: String(e) } : old)));
  };

  const save = async (p: P, rows: R[]) => {
    setS({ kind: "edit", p, rows, saving: true });
    // The result changes the panel only when it still shows this save.
    const mine = (old: SuggestState<P, R>) => old.kind === "edit" && !!old.saving && old.p.key === p.key;
    try {
      await saveWithUndo(() => o.save(p, rows), o.saved, toast, load);
      setS((old) => (mine(old) ? { kind: "idle" } : old));
    } catch (e) {
      setS((old) => (mine(old) ? { ...old, saving: false, error: String(e) } : old));
    }
  };

  return { s, setS, link, setLink, note, setNote, show, allowed, suggest, save, toast };
}
