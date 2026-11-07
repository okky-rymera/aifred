// AIfred's tasks: the reminders and AI runs that AIfred does by itself
// (aifred_core::jobs). You make them in the chat ("remind me every Monday at
// 9"). The Tasks page shows them next to your tasks. There you pause, start,
// or delete them, and read what each run said. The list loads again every 30
// seconds and after a task change, because the runs happen in the background.
import { useCallback, useEffect, useState } from "react";
import * as api from "./api";
import { useTasksChanged } from "./useTasksChanged";

const RELOAD_MS = 30_000;
const LOAD_ERROR = "Cannot load AIfred's tasks";

/** A Unix time as local text, for example "Mon 9 Nov, 09:00". */
function when(t: number): string {
  return new Date(t * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

const ACTION: Record<api.Job["action"], string> = {
  remind: "Reminder",
  agent: "AIfred checks",
  task: "Adds your task",
  brief: "Brief update",
};

const STATUS: Record<api.Job["status"], string> = {
  active: "",
  paused: "Paused",
  done: "Done",
  off: "Off: it failed. See the error in the runs below",
};

const RUN_STATE: Record<api.JobRun["state"], string> = {
  running: "Running",
  ok: "Done",
  quiet: "Nothing to tell",
  skipped: "Skipped",
  failed: "Failed",
};

function JobRow({ job, onChange, onError }: { job: api.Job; onChange: () => void; onError: (e: string) => void }) {
  const [busy, setBusy] = useState(false);
  async function run(p: Promise<unknown>, what: string) {
    setBusy(true);
    try {
      await p;
      onError("");
    } catch (e) {
      onError(`${what}: ${e}`);
    }
    setBusy(false);
    onChange();
  }
  const running = job.status === "active";
  return (
    <li className="rounded-lg border border-stone-200 p-2 text-sm dark:border-stone-800">
      <div className="flex items-baseline gap-2">
        <span className="font-semibold">{job.title}</span>
        <span className="text-[11px] text-stone-500">{ACTION[job.action]}</span>
        {STATUS[job.status] && <span className="text-[11px] text-amber-700 dark:text-amber-400">{STATUS[job.status]}</span>}
      </div>
      <div className="text-xs text-stone-500">
        {job.when}
        {running && job.next_run ? ` · next: ${when(job.next_run)}` : ""}
      </div>
      {job.message && <div className="mt-1 whitespace-pre-wrap text-xs">{job.message}</div>}
      <div className="mt-1 flex gap-3 text-[11px]">
        {running ? (
          <button className="linkbtn" disabled={busy} aria-label={`Pause: ${job.title}`} onClick={() => run(api.setJobStatus(job.id, "paused"), "Not paused")}>
            Pause
          </button>
        ) : (
          // A job with no more times cannot start (jobs.rs): ask AIfred in the chat for a new time.
          job.can_start && <button className="linkbtn" disabled={busy} aria-label={`Start: ${job.title}`} onClick={() => run(api.setJobStatus(job.id, "active"), "Not started")}>
            Start
          </button>
        )}
        {/* The brief job cannot come back after a delete (jobs.rs), so it can only pause. */}
        {job.action !== "brief" && (
          <button className="linkbtn" disabled={busy} aria-label={`Delete: ${job.title}`} onClick={() => run(api.deleteJob(job.id), "Not deleted")}>
            Delete
          </button>
        )}
      </div>
    </li>
  );
}

/** Calls `load` now, every 30 seconds, and when a "task" job adds a task
 * (the "tasks-changed" event). The runs happen in the background, so a list
 * that loads only once goes out of date. A finished task starts the brief
 * job, but sends no event: that run shows at the next 30-second load. */
function useReload(load: () => void) {
  useEffect(() => {
    load();
    const timer = setInterval(load, RELOAD_MS);
    return () => clearInterval(timer);
  }, [load]);
  useTasksChanged(load);
}

/** AIfred's tasks: the reminders and the checks that AIfred runs by itself,
 * with the latest runs. The system job that writes the brief is not here: it
 * is on the Settings page (BriefJob). */
export function AifredTasks() {
  const [list, setList] = useState<api.Job[] | null>(null);
  const [runs, setRuns] = useState<api.JobRun[]>([]);
  const [showEnded, setShowEnded] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(() => {
    // The brief job runs after each finished task. Its runs stay out in the
    // query, so they cannot fill the 20 places (jobs.rs runs).
    Promise.all([api.jobs(true), api.jobRuns(null, 20, false, false)])
      .then(([j, r]) => {
        setList(j);
        setRuns(r);
        // Clear only a load error: an action error stays until the next action.
        setError((old) => (old.startsWith(LOAD_ERROR) ? "" : old));
      })
      .catch((e) => setError(`${LOAD_ERROR}: ${e}`));
  }, []);

  useReload(load);

  const work = (list ?? []).filter((j) => j.action !== "brief");
  const open = work.filter((j) => j.status === "active" || j.status === "paused");
  const ended = work.filter((j) => j.status === "done" || j.status === "off");
  const title = (id: string) => list?.find((j) => j.id === id)?.title ?? "A deleted task";

  return (
    <>
      <p className="note-line" style={{ paddingTop: 0 }}>
        AIfred does these by itself while it runs, also when the window is closed to the tray. Ask in the chat to add one, for
        example "remind me every Monday at 9 to send my timesheet", or "add a task to pay rent on the 1st of each month".
        A repeating task of yours shows here as its rule. Each new copy goes to your list, with its own date.
      </p>
      {error && <p className="note-line err">{error}</p>}
      {list && open.length === 0 && <p className="note-line">AIfred has no tasks now.</p>}
      <ul className="flex flex-col gap-2">
        {open.map((j) => (
          <JobRow key={j.id} job={j} onChange={load} onError={setError} />
        ))}
      </ul>
      {ended.length > 0 && (
        <>
          <button className="linkbtn mt-3 text-xs" onClick={() => setShowEnded((s) => !s)}>
            Ended ({ended.length})
          </button>
          {showEnded && (
            <ul className="mt-2 flex flex-col gap-2">
              {ended.map((j) => (
                <JobRow key={j.id} job={j} onChange={load} onError={setError} />
              ))}
            </ul>
          )}
        </>
      )}
      <h4 className="mt-6 mb-2 text-sm font-semibold">Latest runs</h4>
      {runs.length === 0 && <p className="note-line">No runs yet.</p>}
      <ul className="flex flex-col gap-1">
        {runs.map((r) => (
          <li key={r.id} className="text-xs">
            <span className="text-stone-500">{when(r.started ?? r.planned)}</span> · <span className="font-semibold">{title(r.job)}</span> ·{" "}
            <span className={r.state === "failed" ? "text-red-600" : ""}>{RUN_STATE[r.state]}</span>
            {r.late && <span className="text-stone-500"> (late, planned {when(r.planned)})</span>}
            {r.output && <div className="whitespace-pre-wrap pl-3">{r.output}</div>}
            {r.note && <div className="pl-3 text-stone-500">{r.note}</div>}
          </li>
        ))}
      </ul>
    </>
  );
}

/** The system job that writes the brief again when you finish a task. It is
 * not work, so it is on the Settings page and not in the task list. It can
 * pause, but not go away (jobs.rs). */
export function BriefJob() {
  const [job, setJob] = useState<api.Job | null>(null);
  const [error, setError] = useState("");
  const load = useCallback(() => {
    api
      .briefJob()
      .then((j) => {
        setJob(j);
        setError((old) => (old.startsWith(LOAD_ERROR) ? "" : old));
      })
      .catch((e) => setError(`${LOAD_ERROR}: ${e}`));
  }, []);
  useReload(load);
  return (
    <>
      {error && <p className="text-sm text-red-600">{error}</p>}
      {job && (
        <ul>
          <JobRow job={job} onChange={load} onError={setError} />
        </ul>
      )}
    </>
  );
}
