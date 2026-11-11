import { useEffect } from "react";
import * as api from "./api";

/** Calls `f` each time AIfred adds a task by itself (a repeating task, see
 * the scheduler). */
export function useTasksChanged(f: () => void) {
  useEvent(api.onTasksChanged, f);
}

/** Calls `f` each time AIfred's sort moved new items into subtasks (sort.rs). */
export function useItemsSorted(f: () => void) {
  useEvent(api.onItemsSorted, f);
}

/** Calls `f` when the day changed while the app runs: the chat starts fresh. */
export function useNewDay(f: () => void) {
  useEvent(api.onNewDay, f);
}

/** Listens to one event of AIfred. The listen call is async, so a stop that
 * comes before it ends must still remove the listener. */
function useEvent(on: (f: () => void) => Promise<() => void>, f: () => void) {
  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    on(f)
      .then((s) => (gone ? s() : (stop = s)))
      .catch(() => {});
    return () => {
      gone = true;
      stop?.();
    };
  }, [on, f]);
}
