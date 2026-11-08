// Suggested work (plan step 7 in docs/plan-assistant.md): the next action
// of an item from the brief, and steps that AIfred suggests for an item.
// For a step that is text in an email, a Google Doc, or a Google Sheet, it
// says that AIfred can draft it. Now it can write replies to emails
// (Drafts.tsx), suggest changes in Docs (Docs.tsx), and comment on the
// cells of Sheets (Sheets.tsx).
import { useEffect, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";
import type { Item } from "./items";

/** Who does a step: AIfred can draft its text later, or only you can do it. */
export const DRAFT_TEXT: Record<api.DraftKind, string> = {
  email: "AIfred can draft the email later",
  doc: "AIfred can draft the Doc text later",
  sheet: "AIfred can comment on the Sheet",
};
const DRAFT_HINT = "AIfred makes a draft for you to check. It sends nothing by itself. Now it can write replies to emails, suggest changes in a Doc, and comment on the cells of a Sheet (use the item panel). New emails come later.";

/** `replyTo`: the item is an email from this person, so AIfred can write a
 * reply to them now. The label names them: the next action can be an email
 * to another person, and "Write a reply" cannot write that one. */
function Who({ draft, replyTo }: { draft?: api.DraftKind; replyTo?: string }) {
  if (draft === "email" && replyTo !== undefined) {
    const to = replyTo || "the sender";
    return (
      <span className="who ai" title={`Click "Write a reply" in the item panel. The reply goes to ${to}. Write an email to another person yourself. AIfred sends nothing by itself.`}>
        AIfred can write a reply to {to}
      </span>
    );
  }
  return draft ? (
    <span className="who ai" title={DRAFT_HINT}>
      {DRAFT_TEXT[draft]}
    </span>
  ) : (
    <span className="who" title="Only you can do this step.">
      You do this
    </span>
  );
}

/** The next action from the brief, and who does it. Nothing when the brief gave none. */
export function NextStep({ item: i }: { item: Item }) {
  if (!i.next || i.done) return null;
  return (
    <div className="next-line">
      <span className="k">Next:</span> <span className="nx">{i.next}</span> <Who draft={i.draft} replyTo={i.src === "gm" ? i.mail?.reply_to || (i.mail?.people[0] ?? "") : undefined} />
    </div>
  );
}

/** The task that holds the steps of `i`: `i` itself when it is an open
 * task, else the first open task directly under `i` that the user placed
 * there (steps.rs, add). Undefined: there is no such task. */
export function stepTaskKey(i: Item, links: api.ItemLink[], byKey: Map<string, Item>): string | undefined {
  const mine = links
    .filter((l) => l.main === i.key && l.by !== "aifred")
    .map((l) => l.key)
    .sort();
  const order = [i.key, ...mine];
  return order.find((k) => k.startsWith("task:") && byKey.get(k)?.done === false);
}

/** Where the picked steps go, said before the click: the task of
 * stepTaskKey(), else a new task. A task that AIfred placed in the tree
 * can be other work, so it does not count. */
function target(i: Item, links: api.ItemLink[], byKey: Map<string, Item>): string {
  const k = stepTaskKey(i, links, byKey);
  if (k === i.key) return "the checklist of this task";
  if (k) return `the checklist of your task "${byKey.get(k)?.title}"`;
  return "a new task of yours, under this item";
}

type State = { kind: "idle" } | { kind: "busy"; key: string } | { kind: "error"; text: string } | { kind: "list"; steps: api.Step[]; keep: boolean[]; saving?: boolean };

/** "Suggest steps": AIfred splits the item into steps. You pick the steps to keep. */
export function SuggestSteps({ item: i }: { item: Item }) {
  const { addSteps, links, byKey } = useApp();
  const [s, setS] = useState<State>({ kind: "idle" });
  // Another item in the panel starts with no suggestion.
  useEffect(() => setS({ kind: "idle" }), [i.key]);
  if (i.done) return null;

  const ask = () => {
    const key = i.key;
    setS({ kind: "busy", key });
    // An answer that comes after you opened another item, or clicked Cancel, is dropped.
    const mine = (old: State) => old.kind === "busy" && old.key === key;
    api
      .suggestSteps(key)
      .then((steps) => setS((old) => (mine(old) ? { kind: "list", steps, keep: steps.map(() => true) } : old)))
      .catch((e) => setS((old) => (mine(old) ? { kind: "error", text: String(e) } : old)));
  };

  if (s.kind === "idle" || s.kind === "error") {
    return (
      <div className="suggest">
        <button className="linkbtn" onClick={ask} title="One AI call. AIfred splits this work into steps. You pick the steps to keep.">
          ✦ Suggest steps
        </button>
        {s.kind === "error" && <span className="d-err">Cannot suggest steps: {s.text}</span>}
      </div>
    );
  }
  if (s.kind === "busy") {
    return (
      <div className="suggest">
        <span className="hintx">AIfred is reading the item to suggest steps…</span>
      </div>
    );
  }
  const picked = s.steps.filter((_, n) => s.keep[n]).map((x) => x.text);
  return (
    <div className="suggest" role="group" aria-label="Suggested steps">
      <span className="hintx">Pick the steps to keep. They go to {target(i, links.links, byKey)}.</span>
      <ul className="steps">
        {s.steps.map((x, n) => (
          <li key={`${n}-${x.text}`}>
            <input
              type="checkbox"
              className="chk"
              aria-label={`Keep step: ${x.text}`}
              checked={s.keep[n]}
              onChange={() => setS({ ...s, keep: s.keep.map((k, j) => (j === n ? !k : k)) })}
            />
            <span className="st">{x.text}</span>
            <Who draft={x.draft} />
          </li>
        ))}
      </ul>
      <div className="suggest-acts">
        <button
          className="btn pri"
          // One save at a time: a second click does not add the steps again.
          disabled={!picked.length || s.saving}
          onClick={async () => {
            setS({ ...s, saving: true });
            const ok = await addSteps(i.key, picked);
            setS((old) => (old.kind !== "list" ? old : ok ? { kind: "idle" } : { ...old, saving: false }));
          }}
        >
          Add {picked.length} step{picked.length === 1 ? "" : "s"}
        </button>
        <button className="linkbtn" onClick={() => setS({ kind: "idle" })}>
          Cancel
        </button>
      </div>
    </div>
  );
}
