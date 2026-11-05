// "AIfred's jobs": the reminders and AI runs that AIfred does by itself
// (aifred_core::jobs). You make them in the chat ("remind me every Monday at
// 9"). Here you see them, pause, start, or delete them, and read what each
// run said. The page loads again every 30 seconds, because the jobs run in
// the background.
import { useCallback, useEffect, useState } from "react";
import * as api from "./api";

const RELOAD_MS = 30_000;
const LOAD_ERROR = "Cannot load the jobs";

/** A Unix time as local text, for example "Mon 9 Nov, 09:00". */
function when(t: number): string {
  return new Date(t * 1000).toLocaleString(undefined, { weekday: "short", day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" });
}

const ACTION: Record<api.Job["action"], string> = {
  remind: "Reminder",
  agent: "AIfred checks",
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

export function JobsPage() {
  const [list, setList] = useState<api.Job[] | null>(null);
  const [runs, setRuns] = useState<api.JobRun[]>([]);
  const [showEnded, setShowEnded] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(() => {
    Promise.all([api.jobs(true), api.jobRuns(null, 20, false)])
      .then(([j, r]) => {
        setList(j);
        setRuns(r);
        // Clear only a load error: an action error stays until the next action.
        setError((old) => (old.startsWith(LOAD_ERROR) ? "" : old));
      })
      .catch((e) => setError(`${LOAD_ERROR}: ${e}`));
  }, []);

  useEffect(() => {
    load();
    const timer = setInterval(load, RELOAD_MS);
    return () => clearInterval(timer);
  }, [load]);

  const open = (list ?? []).filter((j) => j.status === "active" || j.status === "paused");
  const ended = (list ?? []).filter((j) => j.status === "done" || j.status === "off");
  const title = (id: string) => list?.find((j) => j.id === id)?.title ?? "A deleted job";

  return (
    <>
      <h2 className="title">AIfred's jobs</h2>
      <p className="note-line" style={{ paddingTop: 0 }}>
        AIfred does these by itself while it runs, also when the window is closed to the tray. Ask in the chat to add one, for
        example "remind me every Monday at 9 to send my timesheet".
      </p>
      {error && <p className="note-line err">{error}</p>}
      {list && open.length === 0 && <p className="note-line">AIfred has no jobs now.</p>}
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
      <h3 className="mt-6 mb-2 text-sm font-semibold">Latest runs</h3>
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
