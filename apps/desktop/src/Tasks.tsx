// The Tasks page: all raw items in one place. "Mine" is your list (local
// tasks, GitHub, ClickUp, and emails). "AIfred's" is the reminders and the
// checks that AIfred runs by itself. "Both" shows the two lists.
import { useState } from "react";
import { useApp } from "./ctx";
import { TaskGroups } from "./Home";
import { AifredTasks } from "./Jobs";

export type TaskFilter = "mine" | "aifred" | "both";

const FILTERS: { key: TaskFilter; label: string }[] = [
  { key: "mine", label: "Mine" },
  { key: "aifred", label: "AIfred's" },
  { key: "both", label: "Both" },
];

const KEY = "aifred-task-filter";

/** The last filter in this window. Storage can fail (private mode), so the start value is "Mine". */
function savedFilter(): TaskFilter {
  try {
    const v = localStorage.getItem(KEY);
    if (v === "mine" || v === "aifred" || v === "both") return v;
  } catch {
    /* no storage: use the start value */
  }
  return "mine";
}

export function TasksPage() {
  const [filter, setFilterState] = useState<TaskFilter>(savedFilter);
  // An item that opens from a toast or the chat must show its row, but
  // "AIfred's" has no rows of your items. Then show both lists. This does
  // not change the saved filter.
  const { sel } = useApp();
  const [seen, setSeen] = useState(sel);
  if (sel !== seen) {
    setSeen(sel);
    if (sel && filter === "aifred") setFilterState("both");
  }
  const setFilter = (f: TaskFilter) => {
    setFilterState(f);
    try {
      localStorage.setItem(KEY, f);
    } catch {
      /* the filter is only for this window */
    }
  };
  return (
    <>
      <h2 className="title">Tasks</h2>
      <div className="seg" role="group" aria-label="Whose tasks">
        {FILTERS.map((f) => (
          <button key={f.key} className={filter === f.key ? "on" : ""} aria-pressed={filter === f.key} onClick={() => setFilter(f.key)}>
            {f.label}
          </button>
        ))}
      </div>
      {filter !== "aifred" && (
        <section aria-label="Your tasks">
          {filter === "both" && <h3 className="group-h">Your tasks</h3>}
          <TaskGroups empty="Nothing to do. Add a task in the box on the home page." />
        </section>
      )}
      {filter !== "mine" && (
        <section aria-label="AIfred's tasks" className={filter === "both" ? "mt-6" : ""}>
          {filter === "both" && <h3 className="group-h">AIfred's tasks</h3>}
          <AifredTasks />
        </section>
      )}
    </>
  );
}
