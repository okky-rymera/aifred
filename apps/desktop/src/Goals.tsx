// The user's goals on the home screen: 3 to 5 short lines. The brief and the
// day plan rank the work by them. After 30 days with no change, the card
// asks the user to check them.
import { useRef, useState } from "react";
import * as api from "./api";
import { useApp } from "./ctx";

/** The goal that an item serves, or "No goal" when the brief ranked it but it serves none. */
export function GoalTag({ goal, ranked }: { goal?: string; ranked: boolean }) {
  const { goals } = useApp();
  if (goal) return <span className="goal-tag" title="The goal that this item serves">◎ {goal}</span>;
  // With no goals, "No goal" says nothing useful.
  if (ranked && goals?.goals.length) return <span className="goal-tag none">No goal</span>;
  return null;
}

function GoalEditor() {
  const { goals, changeGoals } = useApp();
  const [text, setText] = useState("");
  // True while a new goal is saved: a second Enter (or a held key) must not add it again.
  const adding = useRef(false);
  const list = goals?.goals ?? [];
  const full = list.length >= api.MAX_GOALS;
  const save = (g: api.Goal, box: HTMLInputElement) => {
    const v = box.value.trim();
    // An empty box does not remove the goal (the ✕ button does), so it shows the goal again.
    if (!v) box.value = g.text;
    else if (v !== g.text) changeGoals(() => api.editGoal(g.id, v));
  };
  return (
    <div className="goal-edit">
      <ol className="goal-list">
        {list.map((g) => (
          <li key={`${g.id}:${g.text}`}>
            <input
              aria-label={`Goal: ${g.text}`}
              defaultValue={g.text}
              maxLength={200}
              onBlur={(e) => save(g, e.currentTarget)}
              onKeyDown={(e) => {
                if (e.key === "Enter") e.currentTarget.blur();
                if (e.key === "Escape") {
                  e.currentTarget.value = g.text;
                  e.currentTarget.blur();
                }
              }}
            />
            <button className="x" aria-label={`Remove goal: ${g.text}`} onClick={() => changeGoals(() => api.removeGoal(g.id))}>
              ✕
            </button>
          </li>
        ))}
      </ol>
      {full ? (
        <div className="goal-hint">You have {api.MAX_GOALS} goals, the most. Remove one to add another.</div>
      ) : (
        <input
          className="goal-new"
          aria-label="New goal"
          placeholder="New goal, for example: Ship version 2.0 by 30 Nov"
          maxLength={200}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={async (e) => {
            if (e.key !== "Enter" || e.repeat || adding.current || !text.trim()) return;
            adding.current = true;
            try {
              if (await changeGoals(() => api.addGoal(text))) setText("");
            } finally {
              adding.current = false;
            }
          }}
        />
      )}
    </div>
  );
}

export function GoalsCard() {
  const { goals, changeGoals } = useApp();
  const [editing, setEditing] = useState(false);
  if (!goals) return null;
  const list = goals.goals;
  const days = Math.floor((Date.now() / 1000 - goals.checked) / 86_400);
  return (
    <section className="goals" aria-label="Goals">
      <div className="eyebrow">
        <span>◎ Goals</span>
        <span className="sp" />
        <button className="linkbtn" onClick={() => setEditing((e) => !e)}>
          {editing ? "Done" : list.length ? "Edit" : "Add goals"}
        </button>
      </div>
      {goals.stale && !editing && (
        <div className="goal-check">
          <span>You set these goals {days} days ago. Are they still right?</span>
          <button className="btn" onClick={() => changeGoals(api.confirmGoals)}>
            Still right
          </button>
          <button className="btn" onClick={() => setEditing(true)}>
            Change them
          </button>
        </div>
      )}
      {editing ? (
        <GoalEditor />
      ) : list.length ? (
        <ol className="goal-list">
          {list.map((g) => (
            <li key={g.id}>{g.text}</li>
          ))}
        </ol>
      ) : (
        <div className="goal-empty">Add 3 to 5 goals. AIfred then puts the work that moves them first.</div>
      )}
    </section>
  );
}
