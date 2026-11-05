//! The scheduler: it runs AIfred's own jobs (aifred_core::jobs) while the
//! app runs, also when the window is hidden in the tray.
//!
//! Each tick (30 seconds) it asks the database for the jobs to run now. The
//! database marks each run before it starts, so a run never happens twice.
//!
//! Sleep: the tick uses a clock that stops while the computer sleeps. So the
//! first tick after a wake comes at most 30 seconds later, and then
//! claim_due finds the missed times (one late run, the rest skipped).

use std::time::Duration;

use aifred_core::jobs::{self, Claim, Outcome};
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

use crate::App;

/// The time between two checks.
pub const TICK: Duration = Duration::from_secs(30);
/// The first check after the start: the window and the lists load first.
const FIRST_TICK: Duration = Duration::from_secs(10);

/// Starts the loop. It runs until the app quits.
pub fn start<R: Runtime>(h: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let dir = h.state::<App>().assistant.dir.clone();
        tokio::time::sleep(FIRST_TICK).await;
        // Only one window runs the jobs. Kept until this process stops.
        let mut lock: Option<jobs::SchedulerLock> = None;
        loop {
            if lock.is_none() {
                let d = dir.clone();
                match tauri::async_runtime::spawn_blocking(move || jobs::SchedulerLock::take(&d)).await {
                    Ok(Ok(Some(l))) => {
                        lock = Some(l);
                        // Runs that were busy when the last scheduler stopped
                        // do not run again. Safe now: no other window runs jobs.
                        let d = dir.clone();
                        match tauri::async_runtime::spawn_blocking(move || jobs::recover(&d)).await {
                            Ok(Ok(0)) => {}
                            Ok(Ok(n)) => eprintln!("aifred: {n} job runs stopped when AIfred closed"),
                            Ok(Err(e)) => eprintln!("aifred: cannot check the job runs: {e:#}"),
                            Err(e) => eprintln!("aifred: cannot check the job runs: {e}"),
                        }
                    }
                    // Another window runs the jobs. Try again at the next tick.
                    Ok(Ok(None)) => {}
                    Ok(Err(e)) => eprintln!("aifred: {e:#}"),
                    Err(e) => eprintln!("aifred: cannot lock the scheduler: {e}"),
                }
                if lock.is_none() {
                    tokio::time::sleep(TICK).await;
                    continue;
                }
            }
            let d = dir.clone();
            let t = aifred_core::files::now() as i64;
            match tauri::async_runtime::spawn_blocking(move || jobs::claim_due(&d, t)).await {
                Ok(Ok(claims)) => {
                    for c in claims {
                        tauri::async_runtime::spawn(run(h.clone(), c));
                    }
                }
                Ok(Err(e)) => eprintln!("aifred: cannot read the jobs: {e:#}"),
                Err(e) => eprintln!("aifred: cannot read the jobs: {e}"),
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Shows a desktop notification. A failure only goes to the log: the run
/// history still has the text.
pub fn notify<R: Runtime>(h: &AppHandle<R>, title: &str, body: &str) {
    if let Err(e) = h.notification().builder().title(title).body(body).show() {
        eprintln!("aifred: cannot show a notification: {e}");
    }
}

/// Runs one claimed job and saves how it ended.
async fn run<R: Runtime>(h: AppHandle<R>, c: Claim) {
    let app = h.state::<App>();
    let a = app.assistant.clone();
    let late = if c.late {
        format!("\n(Planned for {}.)", jobs::show(c.planned, &c.job.tz))
    } else {
        String::new()
    };
    let outcome = match c.job.action.as_str() {
        "remind" => {
            notify(&h, &c.job.title, &format!("{}{late}", c.job.message));
            Outcome::Ok { output: c.job.message.clone(), cost_usd: None }
        }
        "brief" => {
            // The claim came after the events. A brief that read the tasks
            // after this time (Brief::written) already knows the finished tasks.
            let claimed = aifred_core::files::now();
            // One brief at a time: wait for a brief that the window writes now.
            let _guard = app.brief_busy.lock().await;
            let today = aifred_core::plan::today();
            let (d, day) = (a.dir.clone(), today.clone());
            let saved = tauri::async_runtime::spawn_blocking(move || aifred_core::brief::get(&d, &day))
                .await
                .map_err(anyhow::Error::from)
                .and_then(|r| r);
            // Only an update: a day with no brief gets none, so a finished
            // task costs no AI call when the user does not use the brief.
            match saved {
                Ok(None) => Outcome::Quiet { cost_usd: None },
                // The window wrote it while this run waited: no second AI call.
                Ok(Some(b)) if b.written > claimed => Outcome::Quiet { cost_usd: None },
                Err(e) => Outcome::Failed { error: format!("{e:#}") },
                Ok(Some(_)) => match a.write_brief(&today).await {
                    Ok(_) => Outcome::Ok { output: "The brief is new.".into(), cost_usd: None },
                    Err(e) => Outcome::Failed { error: format!("{e:#}") },
                },
            }
        }
        "agent" => match a.run_job(&c.job, c.planned).await {
            Ok((answer, cost)) => {
                let o = jobs::agent_outcome(&answer, cost);
                if let Outcome::Ok { output, .. } | Outcome::Finished { output, .. } = &o {
                    notify(&h, &c.job.title, &format!("{output}{late}"));
                }
                o
            }
            Err(e) => Outcome::Failed { error: format!("{e:#}") },
        },
        other => Outcome::Failed { error: format!("unknown action \"{other}\"") },
    };
    let failed = matches!(outcome, Outcome::Failed { .. });
    let dir = a.dir.clone();
    let run = c.run;
    match tauri::async_runtime::spawn_blocking(move || jobs::finish(&dir, run, outcome)).await {
        Ok(Ok(Some(job))) if failed && job.status == "off" => notify(
            &h,
            "AIfred stopped a job",
            &format!("\"{}\" failed {} times in a row. Open AIfred's jobs to see the error and start it again.", job.title, job.failures),
        ),
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("aifred: cannot save the job run: {e:#}"),
        Err(e) => eprintln!("aifred: cannot save the job run: {e}"),
    }
}
