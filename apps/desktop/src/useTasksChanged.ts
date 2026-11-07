import { useEffect } from "react";
import * as api from "./api";

/** Calls `f` each time AIfred adds a task by itself (a repeating task, see
 * the scheduler). The listen call is async, so a stop that comes before it
 * ends must still remove the listener. */
export function useTasksChanged(f: () => void) {
  useEffect(() => {
    let stop: (() => void) | undefined;
    let gone = false;
    api
      .onTasksChanged(f)
      .then((s) => (gone ? s() : (stop = s)))
      .catch(() => {});
    return () => {
      gone = true;
      stop?.();
    };
  }, [f]);
}
