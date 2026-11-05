//! The scheduler: it runs AIfred's own jobs (aifred_core::jobs) while the
//! app runs, also when the window is hidden in the tray.
//!
//! Each tick (30 seconds) it asks the database for the jobs to run now. The
//! database marks each run before it starts, so a run never happens twice.
//!
//! Sleep: the tick uses a clock that stops while the computer sleeps. So the
//! first tick after a wake comes at most 30 seconds later, and then
//! claim_due finds the missed times (one late run, the rest skipped).
//!
//! Each tick also checks the follow-up days of the "waiting for" list
//! (aifred_core::waiting). These need no job and no AI call. The check runs
//! in its own task: it can read Gmail, and a slow network must not delay
//! the jobs. One check at a time.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use aifred_core::jobs::{self, Claim, Outcome};
use aifred_core::waiting;
use tauri::{AppHandle, Manager, Runtime};
use tauri_plugin_notification::NotificationExt;

use crate::App;

/// The time between two checks.
pub const TICK: Duration = Duration::from_secs(30);
/// The first check after the start: the window and the lists load first.
const FIRST_TICK: Duration = Duration::from_secs(10);
/// Before a follow-up of an email, AIfred reads Gmail again when the saved
/// emails are older than this, so an answer that came in the morning counts.
/// Not when the user set the refresh of the lists to manual.
const FOLLOW_UP_MAIL_AGE: u64 = 10 * 60;

/// Starts the loop. It runs until the app quits.
pub fn start<R: Runtime>(h: AppHandle<R>) {
    tauri::async_runtime::spawn(async move {
        let dir = h.state::<App>().assistant.dir.clone();
        tokio::time::sleep(FIRST_TICK).await;
        // Only one window runs the jobs. Kept until this process stops.
        let mut lock: Option<jobs::SchedulerLock> = None;
        // True while a follow-up check runs.
        let checking = Arc::new(AtomicBool::new(false));
        loop {
            if lock.is_none() {
                let d = dir.clone();
                match tauri::async_runtime::spawn_blocking(move || jobs::SchedulerLock::take(&d))
                    .await
                {
                    Ok(Ok(Some(l))) => {
                        lock = Some(l);
                        // Runs that were busy when the last scheduler stopped
                        // do not run again. Safe now: no other window runs jobs.
                        let d = dir.clone();
                        match tauri::async_runtime::spawn_blocking(move || jobs::recover(&d)).await
                        {
                            Ok(Ok(0)) => {}
                            Ok(Ok(n)) => {
                                eprintln!("aifred: {n} job runs stopped when AIfred closed")
                            }
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
            if !checking.swap(true, Ordering::AcqRel) {
                let (h, dir, done) = (h.clone(), dir.clone(), Done(checking.clone()));
                tauri::async_runtime::spawn(async move {
                    let _done = done;
                    if let Err(e) = follow_ups(&h, &dir).await {
                        eprintln!("aifred: cannot check the follow-ups: {e:#}");
                    }
                });
            }
            tokio::time::sleep(TICK).await;
        }
    });
}

/// Clears the "a check runs" flag when the check ends, also after a panic.
struct Done(Arc<AtomicBool>);

impl Drop for Done {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Shows a desktop notification. A failure only goes to the log: the run
/// history still has the text.
pub fn notify<R: Runtime>(h: &AppHandle<R>, title: &str, body: &str) {
    if let Err(e) = h.notification().builder().title(title).body(body).show() {
        eprintln!("aifred: cannot show a notification: {e}");
    }
}

/// The items of the "waiting for" list whose follow-up day came now.
async fn due_follow_ups(dir: &std::path::Path) -> anyhow::Result<Vec<waiting::Waiting>> {
    let d = dir.to_path_buf();
    tauri::async_runtime::spawn_blocking(move || waiting::due_now(&d)).await?
}

/// Tells the user about each open item of the "waiting for" list whose
/// follow-up day came. An item from an email is checked in Gmail first:
/// when the person answered, the download closes it, and no notification
/// comes. With a manual refresh, AIfred does not read Gmail by itself.
async fn follow_ups<R: Runtime>(h: &AppHandle<R>, dir: &std::path::Path) -> anyhow::Result<()> {
    let mut due = due_follow_ups(dir).await?;
    if due.is_empty() {
        return Ok(());
    }
    let mut note = None;
    if due.iter().any(|w| w.thread.is_some()) {
        let d = dir.to_path_buf();
        let manual = tauri::async_runtime::spawn_blocking(move || {
            aifred_core::settings::max_age_secs(&d) == u64::MAX
        })
        .await?;
        if manual {
            note = Some(" AIfred did not check Gmail for an answer: the refresh is manual.");
        } else {
            if aifred_core::gmail::refresh_if_old(dir, FOLLOW_UP_MAIL_AGE)
                .await
                .is_some()
            {
                note = Some(" AIfred could not check Gmail for an answer.");
            }
            // The download can close items: read the list again.
            due = due_follow_ups(dir).await?;
        }
    }
    for w in due {
        let (title, mut body) = waiting::reminder(&w);
        if let (Some(_), Some(n)) = (&w.thread, note) {
            body.push_str(n);
        }
        // Mark first, then tell. When the mark fails, the next check (30 s
        // later) tries again. So a failed mark does not repeat the
        // notification, and the other items still get theirs.
        let (d, f) = (dir.to_path_buf(), w.follow_up.clone().unwrap_or_default());
        let marked = tauri::async_runtime::spawn_blocking(move || waiting::mark_told(&d, w.id, &f))
            .await
            .map_err(anyhow::Error::from)
            .and_then(|r| r);
        match marked {
            Ok(true) => notify(h, &title, &body),
            // The item closed, changed, or got its notification meanwhile.
            Ok(false) => {}
            Err(e) => eprintln!("aifred: cannot mark the follow-up of item {}: {e:#}", w.id),
        }
    }
    Ok(())
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
            Outcome::Ok {
                output: c.job.message.clone(),
                cost_usd: None,
            }
        }
        "brief" => {
            // The claim came after the events. A brief that read the tasks
            // after this time (Brief::written) already knows the finished tasks.
            let claimed = aifred_core::files::now();
            // One brief at a time: wait for a brief that the window writes now.
            let _guard = app.brief_busy.lock().await;
            let today = aifred_core::plan::today();
            let (d, day) = (a.dir.clone(), today.clone());
            let saved =
                tauri::async_runtime::spawn_blocking(move || aifred_core::brief::get(&d, &day))
                    .await
                    .map_err(anyhow::Error::from)
                    .and_then(|r| r);
            // Only an update: a day with no brief gets none, so a finished
            // task costs no AI call when the user does not use the brief.
            match saved {
                Ok(None) => Outcome::Quiet { cost_usd: None },
                // The window wrote it while this run waited: no second AI call.
                Ok(Some(b)) if b.written > claimed => Outcome::Quiet { cost_usd: None },
                Err(e) => Outcome::Failed {
                    error: format!("{e:#}"),
                },
                Ok(Some(_)) => match a.write_brief(&today).await {
                    Ok(_) => Outcome::Ok {
                        output: "The brief is new.".into(),
                        cost_usd: None,
                    },
                    Err(e) => Outcome::Failed {
                        error: format!("{e:#}"),
                    },
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
            Err(e) => Outcome::Failed {
                error: format!("{e:#}"),
            },
        },
        other => Outcome::Failed {
            error: format!("unknown action \"{other}\""),
        },
    };
    let failed = matches!(outcome, Outcome::Failed { .. });
    let dir = a.dir.clone();
    let run = c.run;
    match tauri::async_runtime::spawn_blocking(move || jobs::finish(&dir, run, outcome)).await {
        Ok(Ok(Some(job))) if failed && job.status == "off" => notify(
            &h,
            "AIfred stopped a job",
            &format!(
                "\"{}\" failed {} times in a row. Open AIfred's jobs to see the error and start it again.",
                job.title, job.failures
            ),
        ),
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("aifred: cannot save the job run: {e:#}"),
        Err(e) => eprintln!("aifred: cannot save the job run: {e}"),
    }
}
