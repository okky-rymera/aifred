//! The AIfred program. With no words after its name it opens the window.
//! The window talks to aifred-core through the commands in this file.
//!
//! The same file also holds the other parts (see aifred_core::program):
//! the terminal commands (cli.rs), the AI tool servers (aifred-tools), and
//! the meaning search helper (aifred-embed).

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod cli;

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aifred_core::backend::{Event, claude::ClaudeBackend};
use aifred_core::files::now;
use aifred_core::settings::{self, Settings};
use aifred_core::clickup::{self, TokenStatus};
use aifred_core::clickup_list::{ClickupList, ClickupStore};
use aifred_core::github;
use aifred_core::github_list::{GithubList, GithubStore};
use aifred_core::history::Turn;
use aifred_core::memory::Fact;
use aifred_core::tasks::{Task, TaskFields};
use aifred_core::{Answer, Assistant, default_data_dir};
use serde::Serialize;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Manager, Runtime, State, Window, ipc::Channel};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tokio::sync::Mutex;

/// The window settings that are saved and restored. "Visible" is left out:
/// a save during close can record "hidden", and the window must always show.
const WINDOW_STATE: StateFlags = StateFlags::all().difference(StateFlags::VISIBLE);

/// The window moved or changed size since the last save.
static WINDOW_CHANGED: AtomicBool = AtomicBool::new(false);

/// A tray icon was made, so closing the window only hides it.
static TRAY_READY: AtomicBool = AtomicBool::new(false);

/// The version of this program, from Cargo.toml.
const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The first update check waits this long after the start, so it does not
/// slow the start. Then the window checks again on each interval.
const UPDATE_FIRST_CHECK: std::time::Duration = std::time::Duration::from_secs(20);
const UPDATE_INTERVAL: std::time::Duration = std::time::Duration::from_secs(6 * 3600);

/// "Restart now" starts the new window with this setting. The new window
/// waits until the old window stops, so they do not use the data folder at
/// the same time.
const RESTARTED_ENV: &str = "AIFRED_RESTARTED";
const RESTART_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// What the window shows about updates.
#[derive(Clone, Serialize)]
struct UpdateStatus {
    /// The version of this program.
    current: String,
    /// "off", "idle", "checking", "none", "downloading", "ready", or "error".
    state: String,
    /// The new version, when one is found.
    latest: Option<String>,
    /// The release page of the new version, with the list of changes.
    page: Option<String>,
    /// Why updates are off, or the error.
    message: Option<String>,
    /// The last check, in Unix seconds.
    checked_at: Option<u64>,
}

impl UpdateStatus {
    fn new(state: &str, message: Option<&str>) -> Self {
        Self {
            current: VERSION.into(),
            state: state.into(),
            latest: None,
            page: None,
            message: message.map(Into::into),
            checked_at: None,
        }
    }
}

/// The program updates itself from the GitHub releases (aifred_core::update).
/// It downloads a new version in the background. The program file changes
/// only when the user clicks "Restart", or when the window closes.
struct Updater {
    /// The program file to replace. None: updates are off.
    program: Option<std::path::PathBuf>,
    status: std::sync::Mutex<UpdateStatus>,
    /// Held while a check or a download runs, and while "Restart now"
    /// installs the update. So only one of them runs at a time.
    run: Mutex<()>,
    /// Only one window updates the program. Kept until this process stops.
    _lock: Option<aifred_core::update::UpdateLock>,
}

impl Updater {
    fn off(why: &str) -> Self {
        Self {
            program: None,
            status: std::sync::Mutex::new(UpdateStatus::new("off", Some(why))),
            run: Mutex::new(()),
            _lock: None,
        }
    }

    fn on(program: std::path::PathBuf, lock: aifred_core::update::UpdateLock) -> Self {
        Self {
            program: Some(program),
            status: std::sync::Mutex::new(UpdateStatus::new("idle", None)),
            run: Mutex::new(()),
            _lock: Some(lock),
        }
    }

    fn status(&self) -> UpdateStatus {
        self.status.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    fn set(&self, f: impl FnOnce(&mut UpdateStatus)) {
        f(&mut self.status.lock().unwrap_or_else(|e| e.into_inner()));
    }

    /// Asks GitHub for a new version. When there is one, downloads and checks
    /// it, and then the state is "ready". When a check already runs, waits
    /// for its end and does not start another.
    async fn check(&self) {
        let Some(program) = &self.program else { return };
        let Ok(_run) = self.run.try_lock() else {
            let _ = self.run.lock().await;
            return;
        };
        self.set(|s| {
            s.state = "checking".into();
            s.message = None;
        });
        let found = aifred_core::update::check(VERSION).await;
        let now = now();
        let release = match found {
            Ok(Some(r)) => r,
            Ok(None) => {
                // A file from before, of a release that was then removed,
                // must not be installed.
                aifred_core::update::discard(program);
                return self.set(|s| {
                    s.state = "none".into();
                    s.latest = None;
                    s.page = None;
                    s.checked_at = Some(now);
                });
            }
            Err(e) => {
                eprintln!("aifred: cannot check for updates: {e:#}");
                return self.set(|s| {
                    s.state = "error".into();
                    s.message = Some(format!("{e:#}"));
                    s.checked_at = Some(now);
                });
            }
        };
        // The same version is already downloaded and checked.
        let have = self.status().latest.as_deref() == Some(release.version.as_str()) && aifred_core::update::is_staged(program);
        if !have {
            self.set(|s| s.state = "downloading".into());
            if let Err(e) = aifred_core::update::download(&release, program).await {
                eprintln!("aifred: cannot download the update: {e:#}");
                return self.set(|s| {
                    s.state = "error".into();
                    s.message = Some(format!("Cannot install version {}: {e:#}", release.version));
                    s.checked_at = Some(now);
                });
            }
        }
        self.set(|s| {
            s.state = "ready".into();
            s.latest = Some(release.version);
            s.page = Some(release.page);
            s.checked_at = Some(now);
        });
    }
}

struct App {
    assistant: Arc<Assistant<ClaudeBackend>>,
    /// One question at a time: two at once would race on the saved session.
    busy: Mutex<()>,
    /// False in the tests: they must not download the meaning search model
    /// or call the AI for a summary.
    background: bool,
    /// What the sidebar says about meaning search.
    meaning: Arc<std::sync::Mutex<String>>,
    update: Arc<Updater>,
}

impl App {
    fn new(assistant: Arc<Assistant<ClaudeBackend>>, background: bool, update: Updater) -> Self {
        let meaning = if background { "starting" } else { "off" };
        let meaning = Arc::new(std::sync::Mutex::new(meaning.into()));
        Self { assistant, busy: Mutex::new(()), background, meaning, update: Arc::new(update) }
    }

    /// Writes a chat summary in the background: of the conversation that
    /// ended (`Some(id)`), or of the current one when it is long (`None`).
    /// It is one more AI call, so it runs only when it is needed (assistant.rs).
    fn summarize(&self, ended: Option<i64>) {
        if !self.background {
            return;
        }
        let a = self.assistant.clone();
        let fill = self.fill_job();
        tauri::async_runtime::spawn(async move {
            let made = match ended {
                Some(id) => a.summarize_ended(id).await,
                None => a.summarize_if_long().await,
            };
            match made {
                // The summary gets its meaning search vector.
                Ok(true) => fill(),
                Ok(false) => {}
                Err(e) => eprintln!("aifred: cannot write the chat summary: {e:#}"),
            }
        });
    }

    /// Writes the missing summaries of old conversations in the background:
    /// conversations from the old history files, and summaries that stopped
    /// when the app closed. At most 5 conversations in one run (assistant.rs).
    fn summarize_old(&self) {
        if !self.background {
            return;
        }
        let a = self.assistant.clone();
        let fill = self.fill_job();
        tauri::async_runtime::spawn(async move {
            match a.summarize_old().await {
                // The summaries get their meaning search vectors.
                Ok(n) if n > 0 => fill(),
                Ok(_) => {}
                Err(e) => eprintln!("aifred: cannot write the old chat summaries: {e:#}"),
            }
        });
    }

    /// Writes the missing summaries of long descriptions in the background
    /// (summaries.rs), for the overview tool of the chat. Each group of 15
    /// items is one AI call with the fast model. It does nothing when no
    /// summary is missing.
    fn summarize_items(&self) {
        if !self.background {
            return;
        }
        let a = self.assistant.clone();
        tauri::async_runtime::spawn(async move {
            if let Err(e) = a.summarize_items().await {
                eprintln!("aifred: cannot write the item summaries: {e:#}");
            }
        });
    }

    /// Makes the missing meaning search vectors on a background thread, so the
    /// window does not wait. Two fills at the same time are safe: the second
    /// waits for the first (embed.rs) and then finds little to do.
    fn fill_vectors(&self) {
        if self.background {
            (self.fill_job())();
        }
    }

    /// The vector fill as a job that can run later, on any thread.
    fn fill_job(&self) -> impl Fn() + Send + 'static {
        let (a, meaning) = (self.assistant.clone(), self.meaning.clone());
        move || {
            let (a, meaning) = (a.clone(), meaning.clone());
            std::thread::spawn(move || {
            let off = "off: the model cannot load. Word search still works.".to_string();
            // When the fill made data, the model works. In the other cases
            // the fill did not ask the helper (nothing was missing, or the
            // database failed first). When the status is already "on", keep
            // it: a check would start the helper and load the model (about
            // 175 MB) for nothing, and keep it in memory 5 more minutes. Only
            // an unknown or "off" status asks the helper with one short text.
            let known_on = meaning.lock().unwrap_or_else(|e| e.into_inner()).as_str() == "on";
            let check = || {
                if known_on {
                    return "on".to_string();
                }
                match aifred_core::embed::check(&a.dir) {
                    Ok(()) => "on".to_string(),
                    Err(e) => {
                        eprintln!("aifred: {e}");
                        off.clone()
                    }
                }
            };
            let status = match a.fill_vectors() {
                Ok(n) if n > 0 => "on".to_string(),
                Ok(_) => check(),
                Err(e) => {
                    eprintln!("aifred: cannot make the meaning search data: {e:#}");
                    if e.downcast_ref::<aifred_core::embed::ModelError>().is_some() {
                        off.clone()
                    } else {
                        // For example a busy database. The next fill (after
                        // the next answer) makes the missing data.
                        check()
                    }
                }
            };
            *meaning.lock().unwrap_or_else(|e| e.into_inner()) = status;
            });
        }
    }
}

#[derive(Serialize)]
struct Info {
    data_dir: String,
    backend: String,
    github_owners: Option<String>,
    api_key_set: bool,
    /// "on", "starting", or "off" with the reason.
    meaning_search: String,
}

type CmdResult<T> = Result<T, String>;

fn err(e: anyhow::Error) -> String {
    format!("{e:#}")
}

#[tauri::command]
async fn ask(app: State<'_, App>, prompt: String, on_event: Channel<Event>) -> CmdResult<Answer> {
    let Ok(_guard) = app.busy.try_lock() else {
        return Err("AIfred is still working on the last question.".into());
    };
    let mut send = |e: Event| {
        let _ = on_event.send(e);
    };
    let answer = app.assistant.ask(&prompt, &mut send).await.map_err(err)?;
    // The new messages, and the facts or tasks that AIfred added.
    app.fill_vectors();
    app.summarize(None);
    app.summarize_items();
    Ok(answer)
}

/// Runs file work on a background thread. Tauri runs commands that are not
/// async on the main thread, so slow disk work there freezes the window.
async fn blocking<T: Send + 'static>(
    app: &State<'_, App>,
    f: impl FnOnce(&Assistant<ClaudeBackend>) -> anyhow::Result<T> + Send + 'static,
) -> CmdResult<T> {
    let a = app.assistant.clone();
    tauri::async_runtime::spawn_blocking(move || f(&a)).await.map_err(|e| e.to_string())?.map_err(err)
}

#[tauri::command]
async fn history(app: State<'_, App>) -> CmdResult<Vec<Turn>> {
    blocking(&app, |a| a.turns()).await
}

/// All memory facts: the ones in use, then the forgotten ones.
#[tauri::command]
async fn memory_facts(app: State<'_, App>) -> CmdResult<Vec<Fact>> {
    blocking(&app, |a| a.memory().list()).await
}

/// A fact that the user adds is a core fact: it goes into every message.
#[tauri::command]
async fn add_fact(app: State<'_, App>, text: String) -> CmdResult<Fact> {
    let f = blocking(&app, move |a| a.memory().add(&text, "you", true)).await?;
    app.fill_vectors();
    Ok(f)
}

#[tauri::command]
async fn edit_fact(app: State<'_, App>, id: i64, text: String) -> CmdResult<Fact> {
    let f = blocking(&app, move |a| a.memory().edit(id, &text)).await?;
    app.fill_vectors();
    Ok(f)
}

#[tauri::command]
async fn set_fact_core(app: State<'_, App>, id: i64, core: bool) -> CmdResult<Fact> {
    blocking(&app, move |a| a.memory().set_core(id, core)).await
}

#[tauri::command]
async fn forget_fact(app: State<'_, App>, id: i64) -> CmdResult<Fact> {
    blocking(&app, move |a| a.memory().forget(id)).await
}

#[tauri::command]
async fn restore_fact(app: State<'_, App>, id: i64) -> CmdResult<Fact> {
    blocking(&app, move |a| a.memory().restore(id)).await
}

#[tauri::command]
async fn new_conversation(app: State<'_, App>) -> CmdResult<()> {
    let Ok(_guard) = app.busy.try_lock() else {
        return Err("Wait until the current question is done.".into());
    };
    let ended = app.assistant.new_conversation().map_err(err)?;
    // The old conversation gets a summary in the background, for search.
    if ended.is_some() {
        app.summarize(ended);
    }
    Ok(())
}

// The database has its own locks (db.rs), so these do not need the busy lock.
#[tauri::command]
async fn tasks(app: State<'_, App>) -> CmdResult<Vec<Task>> {
    blocking(&app, |a| a.tasks().load()).await
}

/// Adds a task. Only the title is necessary.
#[tauri::command]
async fn add_task(app: State<'_, App>, task: TaskFields) -> CmdResult<Task> {
    let t = blocking(&app, move |a| a.tasks().create(task, "you")).await?;
    app.fill_vectors();
    app.summarize_items();
    Ok(t)
}

/// Changes the fields in `changes` of a local task. The other fields stay.
#[tauri::command]
async fn edit_task(app: State<'_, App>, id: String, changes: TaskFields) -> CmdResult<Task> {
    // Meaning search and summaries read only the title and the description.
    // The edit form sends only the changed fields, so this skips both jobs
    // for a change of the tags, the dates, the priority, or the checklist.
    let text = changes.title.is_some() || changes.notes.is_some();
    let t = blocking(&app, move |a| a.tasks().update(&id, changes)).await?;
    if text {
        app.fill_vectors();
        app.summarize_items();
    }
    Ok(t)
}

/// Ticks or un-ticks one step. `text` must be the step's text, so a tick
/// fails when the checklist changed after the list was loaded.
#[tauri::command]
async fn set_task_step(app: State<'_, App>, id: String, index: usize, text: String, done: bool) -> CmdResult<Task> {
    blocking(&app, move |a| a.tasks().set_step(&id, index, &text, done)).await
}

#[tauri::command]
async fn set_task_done(app: State<'_, App>, id: String, done: bool) -> CmdResult<Task> {
    blocking(&app, move |a| a.tasks().set_done(&id, done)).await
}

#[tauri::command]
async fn delete_task(app: State<'_, App>, id: String) -> CmdResult<()> {
    blocking(&app, move |a| a.tasks().remove(&id)).await
}

/// The GitHub list. It uses the saved list while it is younger than the
/// refresh time in the settings. `force` always asks GitHub (the reload button
/// and the refresh timer). If GitHub cannot be asked, it shows the saved list
/// at any age, with a warning.
#[tauri::command]
async fn github_items(app: State<'_, App>, force: bool) -> CmdResult<GithubList> {
    let dir = app.assistant.dir.clone();
    if !force {
        let d = dir.clone();
        let saved = blocking(&app, move |_| GithubStore::new(&d).cached(max_age(&d), now())).await?;
        if let Some(list) = saved {
            return Ok(list);
        }
    }
    match github::my_items(&github::owners_from_env()).await {
        Ok(f) => {
            let list = blocking(&app, move |_| GithubStore::new(&dir).save_fetch(f.items, &f.no_links, now())).await?;
            // Only new and changed items get new vectors and summaries.
            app.fill_vectors();
            app.summarize_items();
            Ok(list)
        }
        Err(e) => {
            let e = err(e);
            let saved = blocking(&app, move |_| GithubStore::new(&dir).cached(u64::MAX, now())).await;
            match saved {
                Ok(Some(mut list)) => {
                    list.warning = Some(format!("Cannot reach GitHub, so this is the saved list. {e}"));
                    Ok(list)
                }
                _ => Err(e),
            }
        }
    }
}

/// The saved list may be used while it is younger than the refresh time in the settings.
fn max_age(dir: &std::path::Path) -> u64 {
    settings::max_age_secs(dir)
}

fn clickup_token(dir: &std::path::Path) -> CmdResult<String> {
    clickup::token(dir).ok_or_else(|| "ClickUp is not connected. Add a token on the Settings page.".to_string())
}

fn clickup_client(dir: &std::path::Path) -> CmdResult<clickup::Client> {
    clickup_token(dir).map(|t| clickup::Client::new(&t))
}

/// The ClickUp list. It works like github_items: the saved list while it is
/// fresh, and the saved list with a warning when ClickUp cannot be asked.
#[tauri::command]
async fn clickup_items(app: State<'_, App>, force: bool) -> CmdResult<ClickupList> {
    let dir = app.assistant.dir.clone();
    let token = clickup_token(&dir)?;
    let client = clickup::Client::new(&token);
    // The saved list is used only for the same token. AIFRED_CLICKUP_TOKEN can
    // change between starts, and then the saved list is of another account.
    let account = clickup::account_key(&token);
    if !force {
        let (d, acc) = (dir.clone(), account.clone());
        if let Some(list) = blocking(&app, move |_| ClickupStore::new(&d).cached(&acc, max_age(&d), now())).await? {
            return Ok(list);
        }
    }
    match client.my_tasks().await {
        Ok(items) => {
            let list = blocking(&app, move |_| ClickupStore::new(&dir).save(&account, items, now())).await?;
            app.fill_vectors();
            app.summarize_items();
            Ok(list)
        }
        Err(e) => {
            let e = err(e);
            match blocking(&app, move |_| ClickupStore::new(&dir).cached(&account, u64::MAX, now())).await {
                Ok(Some(mut list)) => {
                    let cut = list.warning.take().map(|w| format!(" {w}")).unwrap_or_default();
                    list.warning = Some(format!("Cannot reach ClickUp, so this is the saved list. {e}{cut}"));
                    Ok(list)
                }
                _ => Err(e),
            }
        }
    }
}

/// Moves one task to the closed status of its ClickUp list. This is the only
/// change AIfred makes in ClickUp, and only the user starts it.
/// Returns the status the task had before, for the undo.
#[tauri::command]
async fn clickup_mark_done(app: State<'_, App>, id: String) -> CmdResult<String> {
    let dir = app.assistant.dir.clone();
    let before = clickup_client(&dir)?.mark_done(&id).await.map_err(err)?;
    // ClickUp has the change. If the saved list cannot change, the task only
    // shows until the next reload, so this is not an error.
    if let Err(e) = blocking(&app, move |_| ClickupStore::new(&dir).remove(&id)).await {
        eprintln!("aifred: cannot update the saved ClickUp list: {e}");
    }
    Ok(before)
}

/// Puts a task back to the status it had before clickup_mark_done.
#[tauri::command]
async fn clickup_undo_done(app: State<'_, App>, id: String, status: String) -> CmdResult<()> {
    clickup_client(&app.assistant.dir)?.set_status(&id, &status).await.map_err(err)
}

#[tauri::command]
async fn clickup_token_status(app: State<'_, App>) -> CmdResult<TokenStatus> {
    blocking(&app, |a| Ok(clickup::token_status(&a.dir))).await
}

/// Checks the token with ClickUp, then saves it. Returns the ClickUp user name.
#[tauri::command]
async fn set_clickup_token(app: State<'_, App>, token: String) -> CmdResult<String> {
    let token = token.trim().to_string();
    let (_, name) = clickup::Client::new(&token).me().await.map_err(err)?;
    blocking(&app, move |a| {
        clickup::save_token(&a.dir, &token)?;
        // The old list can belong to another ClickUp account.
        ClickupStore::new(&a.dir).clear()
    })
    .await?;
    Ok(name)
}

#[tauri::command]
async fn remove_clickup_token(app: State<'_, App>) -> CmdResult<()> {
    blocking(&app, |a| {
        clickup::remove_token(&a.dir)?;
        ClickupStore::new(&a.dir).clear()
    })
    .await
}

#[tauri::command]
async fn get_settings(app: State<'_, App>) -> CmdResult<Settings> {
    blocking(&app, |a| settings::load(&a.dir)).await
}

#[tauri::command]
async fn set_settings(app: State<'_, App>, settings: Settings) -> CmdResult<()> {
    blocking(&app, move |a| settings::save(&a.dir, &settings)).await
}

/// The model that the chat box shows: the one the user picked last, or None for the default.
#[tauri::command]
async fn chat_model(app: State<'_, App>) -> CmdResult<Option<String>> {
    blocking(&app, |a| settings::chat_model(&a.dir)).await
}

/// Saves the model for the next messages. None goes back to the default.
#[tauri::command]
async fn set_chat_model(app: State<'_, App>, model: Option<String>) -> CmdResult<()> {
    blocking(&app, move |a| settings::set_chat_model(&a.dir, model.as_deref())).await
}

#[tauri::command]
fn info(app: State<'_, App>) -> Info {
    Info {
        data_dir: app.assistant.dir.display().to_string(),
        backend: "Claude (your login)".into(),
        github_owners: std::env::var("AIFRED_GITHUB_OWNERS").ok(),
        api_key_set: std::env::var_os("ANTHROPIC_API_KEY").is_some(),
        meaning_search: app.meaning.lock().unwrap_or_else(|e| e.into_inner()).clone(),
    }
}

#[tauri::command]
fn update_status(app: State<'_, App>) -> UpdateStatus {
    app.update.status()
}

/// Checks for a new version now, and downloads it when there is one.
#[tauri::command]
async fn check_update(app: State<'_, App>) -> CmdResult<UpdateStatus> {
    app.update.check().await;
    Ok(app.update.status())
}

/// Puts the downloaded version in place and starts it. This window closes.
/// The chat, the tasks, and the window size are saved, so the new window
/// opens where this one was.
#[tauri::command]
async fn restart_to_update<R: tauri::Runtime>(handle: tauri::AppHandle<R>, app: State<'_, App>) -> CmdResult<()> {
    let Some(program) = app.update.program.clone() else {
        return Err("Updates are off.".into());
    };
    // Hold the lock to the end: no new question can start.
    let Ok(_guard) = app.busy.try_lock() else {
        return Err("AIfred is still working on the last question. Restart after the answer.".into());
    };
    // No check or download can change the waiting file now.
    let Ok(_run) = app.update.run.try_lock() else {
        return Err("AIfred is checking for a newer version. Try again in a minute.".into());
    };
    if app.update.status().state != "ready" {
        return Err("No downloaded update waits. Check for updates again.".into());
    }
    if !aifred_core::update::apply(&program).map_err(err)? {
        return Err("No downloaded update waits. Check for updates again.".into());
    }
    let _ = handle.save_window_state(WINDOW_STATE);
    // The new window waits until this process stops (RESTARTED_ENV).
    std::process::Command::new(&program)
        .env(RESTARTED_ENV, "1")
        .spawn()
        .map_err(|e| format!("The update is installed, but the new version did not start ({e}). Open AIfred again."))?;
    handle.exit(0);
    Ok(())
}

/// Adds the app state and all commands. The tests use the same list.
fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>, app: App) -> tauri::Builder<R> {
    builder.manage(app).invoke_handler(tauri::generate_handler![
        ask,
        history,
        memory_facts,
        add_fact,
        edit_fact,
        set_fact_core,
        forget_fact,
        restore_fact,
        new_conversation,
        tasks,
        add_task,
        edit_task,
        set_task_step,
        set_task_done,
        delete_task,
        github_items,
        clickup_items,
        clickup_mark_done,
        clickup_undo_done,
        clickup_token_status,
        set_clickup_token,
        remove_clickup_token,
        get_settings,
        set_settings,
        chat_model,
        set_chat_model,
        info,
        update_status,
        check_update,
        restart_to_update
    ])
}

fn main() -> anyhow::Result<()> {
    let program = std::env::current_exe()?;
    aifred_core::program::set_path(program.clone());
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        None => open_window(program),
        // The update check runs the new file with this word (aifred_core::update).
        Some("version" | "--version") => {
            println!("{VERSION}");
            Ok(())
        }
        Some(aifred_core::program::TOOL) => match args.get(1) {
            Some(name) => aifred_tools::run(name),
            None => anyhow::bail!("usage: aifred tool <name>"),
        },
        Some(aifred_core::program::EMBED_HELPER) => match (args.get(1), args.get(2)) {
            (Some(dir), Some(socket)) => {
                let dir = std::path::Path::new(dir);
                aifred_embed::serve(dir, socket.as_ref(), &aifred_embed::models_dir(dir), aifred_embed::IDLE)
            }
            _ => anyhow::bail!("usage: aifred embed-helper <data folder> <socket>"),
        },
        Some(word) if cli::COMMANDS.contains(&word) => cli::run(args),
        Some(_) => {
            eprintln!("usage: aifred [ask \"<message>\" | chat | remember \"<fact>\" | new | index | version]");
            eprintln!("With no words, aifred opens the window.");
            std::process::exit(2);
        }
    }
}

fn open_window(program: std::path::PathBuf) -> anyhow::Result<()> {
    // First, so that a restarted window waits for the old one to stop
    // before it opens the data folder.
    let update = updater(program.clone());
    let dir = default_data_dir()?;
    let backend = ClaudeBackend { work_dir: dir.join("claude"), model: std::env::var("AIFRED_MODEL").ok() };
    GithubStore::new(&dir).remove_old_files();
    let assistant = Arc::new(Assistant::new(dir, backend, program.clone())?);

    // AIFRED_NO_WINDOW_STATE: do not read or write the saved window size.
    // scripts/run-hidden.sh sets it, so a check run cannot change the size
    // of the user's own window.
    let keep_window_state = std::env::var_os("AIFRED_NO_WINDOW_STATE").is_none();

    let app = App::new(assistant, true, update);
    // Vectors for the data from before, or from the command-line program and
    // the tool servers. The first start also downloads the model here.
    app.fill_vectors();
    app.summarize_old();
    let mut builder = with_window_events(with_commands(tauri::Builder::default(), app), keep_window_state)
        .plugin(tauri_plugin_opener::init());
    if keep_window_state {
        // Saves the window size and maximized state when the app closes, and
        // puts them back on the next start. On X11 it also restores the
        // position. On Wayland the app cannot read or set the position, so
        // the desktop picks it. The window starts hidden (tauri.conf.json) so
        // it does not show at the default size first and then jump.
        builder = builder.plugin(tauri_plugin_window_state::Builder::default().with_state_flags(WINDOW_STATE).build());
    }
    builder
        .setup(|app| {
            // On Linux the tray library panics when appindicator is missing.
            // Then there is no tray, and closing the window quits AIfred.
            match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| make_tray(app))) {
                Ok(Ok(())) => TRAY_READY.store(true, Ordering::Relaxed),
                Ok(Err(e)) => eprintln!("aifred: no tray icon ({e:#}); closing the window quits AIfred"),
                Err(_) => eprintln!("aifred: no tray icon (the tray library is missing); closing the window quits AIfred"),
            }
            // The plugin does not save or restore "visible", so this is the
            // only place that shows the window.
            show_main(app.handle());
            // Check for updates after the start, then on each interval.
            let update = app.state::<App>().update.clone();
            if update.program.is_some() {
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(UPDATE_FIRST_CHECK).await;
                    loop {
                        update.check().await;
                        tokio::time::sleep(UPDATE_INTERVAL).await;
                    }
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())?
        .run(|handle, event| {
            // A downloaded update that the user did not restart for: the
            // next start uses it.
            if let tauri::RunEvent::Exit = event
                && let Some(program) = &handle.state::<App>().update.program
            {
                match aifred_core::update::apply(program) {
                    Ok(true) => eprintln!("aifred: the update is installed. The next start uses it."),
                    Ok(false) => {}
                    Err(e) => eprintln!("aifred: cannot install the update: {e:#}"),
                }
            }
        });
    Ok(())
}

/// Shows the main window, also when it is hidden or minimized. Ignore
/// errors: some window managers refuse focus, and that must not stop the app.
fn show_main<R: Runtime>(app: &AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// The tray icon with its menu: "Show AIfred" and "Quit".
fn make_tray(app: &tauri::App) -> anyhow::Result<()> {
    let show = MenuItem::with_id(app, "show", "Show AIfred", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;
    let icon = app.default_window_icon().cloned().ok_or_else(|| anyhow::anyhow!("the app has no icon"))?;
    TrayIconBuilder::with_id("main")
        .icon(icon)
        .tooltip("AIfred")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id().as_ref() {
            "show" => show_main(app),
            // The window-state plugin saves the window on this exit.
            "quit" => app.exit(0),
            _ => {}
        })
        // Linux sends no clicks: there a click opens the menu.
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

/// Closing the main window hides it when there is a tray. Returns true if it
/// hid the window (the caller then stops the close). With no tray the close
/// goes on and the app quits, so the user can always quit.
fn hide_on_close<R: Runtime>(w: &Window<R>, tray: bool, keep_window_state: bool) -> bool {
    if !tray || w.label() != "main" {
        return false;
    }
    // Save first: the size and place of a shown window are sure to be right.
    if keep_window_state {
        let _ = w.app_handle().save_window_state(WINDOW_STATE);
        WINDOW_CHANGED.store(false, Ordering::Relaxed);
    }
    let _ = w.hide();
    true
}

/// Adds the window events. The tests use the same handler.
fn with_window_events<R: Runtime>(builder: tauri::Builder<R>, keep_window_state: bool) -> tauri::Builder<R> {
    builder.on_window_event(move |w, e| match e {
        // The plugin writes the file only on a normal quit. A logout or a
        // crash skips that. So also write it when the window loses focus,
        // but only if the window moved or changed size since the last save.
        tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
            WINDOW_CHANGED.store(true, Ordering::Relaxed);
        }
        tauri::WindowEvent::Focused(false) if keep_window_state && WINDOW_CHANGED.swap(false, Ordering::Relaxed) => {
            let _ = w.app_handle().save_window_state(WINDOW_STATE);
        }
        tauri::WindowEvent::CloseRequested { api, .. }
            if hide_on_close(w, TRAY_READY.load(Ordering::Relaxed), keep_window_state) =>
        {
            api.prevent_close();
        }
        _ => {}
    })
}

/// The updater of this window. Updates are off in a development build, and
/// when another AIfred window already updates the program.
fn updater(program: std::path::PathBuf) -> Updater {
    // A development build must not replace itself with a release.
    if cfg!(debug_assertions) {
        return Updater::off("Updates are off in development builds.");
    }
    let wait = std::env::var_os(RESTARTED_ENV).is_some().then_some(RESTART_WAIT);
    match aifred_core::update::UpdateLock::take(&program, wait) {
        Ok(Some(lock)) => {
            // A download from a window that stopped early was not checked to the end.
            aifred_core::update::discard(&program);
            Updater::on(program, lock)
        }
        Ok(None) => Updater::off("Another AIfred window checks for updates."),
        Err(e) => {
            eprintln!("aifred: {e:#}");
            Updater::off(&format!("{e:#}"))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tauri::test::{INVOKE_KEY, MockRuntime, get_ipc_response, mock_builder, mock_context, noop_assets};
    use tauri::{WebviewWindow, ipc::InvokeBody, webview::InvokeRequest};

    /// A fake app window with its own empty data folder. No Claude, no real window.
    fn window(name: &str) -> (tauri::App<MockRuntime>, WebviewWindow<MockRuntime>, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("aifred-desktop-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let backend = ClaudeBackend { work_dir: dir.join("claude"), model: None };
        let assistant = Arc::new(Assistant::new(dir.clone(), backend, dir.clone()).unwrap());
        let app = with_commands(mock_builder(), App::new(assistant, false, Updater::off("Updates are off in tests.")));
        let app = with_window_events(app, false)
            .build(mock_context(noop_assets()))
            .unwrap();
        let win = tauri::WebviewWindowBuilder::new(&app, "main", Default::default()).build().unwrap();
        (app, win, dir)
    }

    /// Calls a command the same way the window does.
    fn call(win: &WebviewWindow<MockRuntime>, cmd: &str, args: Value) -> Result<Value, Value> {
        get_ipc_response(
            win,
            InvokeRequest {
                cmd: cmd.into(),
                callback: tauri::ipc::CallbackFn(0),
                error: tauri::ipc::CallbackFn(1),
                url: "tauri://localhost".parse().unwrap(),
                body: InvokeBody::Json(args),
                headers: Default::default(),
                invoke_key: INVOKE_KEY.to_string(),
            },
        )
        .map(|b| b.deserialize::<Value>().unwrap())
    }

    #[test]
    fn task_commands_add_finish_and_delete() {
        let (_app, win, dir) = window("tasks");
        assert_eq!(call(&win, "tasks", json!({})).unwrap(), json!([]));

        let t = call(&win, "add_task", json!({ "task": { "title": "Call Sam" } })).unwrap();
        assert_eq!(t["title"], "Call Sam");
        assert_eq!(t["added_by"], "you", "tasks from the window are yours");
        let id = t["id"].as_str().unwrap().to_string();

        let edited = call(&win, "edit_task", json!({ "id": id, "changes": { "title": "Call Sam today", "notes": " About the invoice. " } })).unwrap();
        assert_eq!((edited["title"].clone(), edited["notes"].clone()), (json!("Call Sam today"), json!("About the invoice.")));
        let changes = json!({ "due": "2026-12-01", "priority": "high", "tags": ["work"], "checklist": [{ "text": "Find number", "done": true }] });
        let more = call(&win, "edit_task", json!({ "id": id, "changes": changes })).unwrap();
        assert_eq!((&more["title"], &more["due"], &more["priority"]), (&json!("Call Sam today"), &json!("2026-12-01"), &json!("high")));
        assert_eq!((&more["tags"], &more["checklist"]), (&json!(["work"]), &json!([{ "text": "Find number", "done": true }])));
        let with_notes = call(&win, "add_task", json!({ "task": { "title": "Pay rent", "notes": "Before Friday" } })).unwrap();
        assert_eq!(with_notes["notes"], "Before Friday");
        call(&win, "delete_task", json!({ "id": with_notes["id"] })).unwrap();

        let step = call(&win, "set_task_step", json!({ "id": id, "index": 0, "text": "Find number", "done": false })).unwrap();
        assert_eq!(step["checklist"], json!([{ "text": "Find number", "done": false }]));
        assert!(call(&win, "set_task_step", json!({ "id": id, "index": 0, "text": "Other", "done": true })).is_err(), "the checklist changed");

        let done = call(&win, "set_task_done", json!({ "id": id, "done": true })).unwrap();
        assert_eq!(done["done"], true);
        assert_eq!(call(&win, "tasks", json!({})).unwrap()[0]["done"], true);

        call(&win, "delete_task", json!({ "id": id })).unwrap();
        assert_eq!(call(&win, "tasks", json!({})).unwrap(), json!([]));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn task_command_errors_reach_the_window_as_text() {
        let (_app, win, dir) = window("errors");
        let e = call(&win, "add_task", json!({ "task": { "title": "  " } })).unwrap_err();
        assert!(e.as_str().unwrap().contains("empty"), "got {e}");
        let e = call(&win, "delete_task", json!({ "id": "nope" })).unwrap_err();
        assert!(e.as_str().unwrap().contains("no task"), "got {e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn memory_facts_round_trip_and_empty_history() {
        let (_app, win, dir) = window("memory");
        assert_eq!(call(&win, "memory_facts", json!({})).unwrap(), json!([]));
        let f = call(&win, "add_fact", json!({ "text": "I like short answers." })).unwrap();
        assert_eq!((f["core"].clone(), f["added_by"].clone()), (json!(true), json!("you")));
        let id = f["id"].clone();
        call(&win, "edit_fact", json!({ "id": id, "text": "I like very short answers." })).unwrap();
        call(&win, "set_fact_core", json!({ "id": id, "core": false })).unwrap();
        call(&win, "forget_fact", json!({ "id": id })).unwrap();
        let all = call(&win, "memory_facts", json!({})).unwrap();
        assert_eq!(all[0]["text"], "I like very short answers.");
        assert_eq!(all[0]["core"], false);
        assert!(all[0]["forgotten"].is_number());
        assert!(call(&win, "restore_fact", json!({ "id": id })).unwrap()["forgotten"].is_null());
        assert!(call(&win, "edit_fact", json!({ "id": 999, "text": "x" })).is_err());
        assert_eq!(call(&win, "history", json!({})).unwrap(), json!([]));
        call(&win, "new_conversation", json!({})).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_round_trip_and_bad_value_is_refused() {
        let (_app, win, dir) = window("settings");
        assert_eq!(call(&win, "get_settings", json!({})).unwrap()["refresh_minutes"], 15);
        call(&win, "set_settings", json!({ "settings": { "refresh_minutes": 30 } })).unwrap();
        assert_eq!(call(&win, "get_settings", json!({})).unwrap()["refresh_minutes"], 30);
        assert!(call(&win, "set_settings", json!({ "settings": { "refresh_minutes": 99999 } })).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A saved GitHub list, as if GitHub was asked `ago` seconds earlier.
    fn save_list(dir: &std::path::Path, ago: u64) {
        let item: github::GithubItem = serde_json::from_value(json!({
            "id": "acme/shop#7", "repo": "acme/shop", "number": 7, "title": "Fix cart",
            "url": "https://github.com/acme/shop/pull/7", "reason": "review_requested", "updated": ""
        }))
        .unwrap();
        GithubStore::new(dir).save_fetch(vec![item], &[], now() - ago).unwrap();
    }

    #[test]
    fn github_list_comes_from_the_saved_list_while_it_is_fresh() {
        let (_app, win, dir) = window("gh-cache");
        save_list(&dir, 60); // one minute old; the default refresh time is 15 minutes
        // No `gh` runs here: the saved list is fresh.
        let l = call(&win, "github_items", json!({ "force": false })).unwrap();
        assert_eq!(l["items"][0]["title"], "Fix cart");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn manual_only_setting_never_makes_the_saved_list_old() {
        let (_app, win, dir) = window("gh-manual");
        call(&win, "set_settings", json!({ "settings": { "refresh_minutes": 0 } })).unwrap();
        save_list(&dir, 30 * 86400); // 30 days old
        let l = call(&win, "github_items", json!({ "force": false })).unwrap();
        assert_eq!(l["items"].as_array().unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn broken_settings_file_does_not_hide_the_saved_list_and_can_be_fixed() {
        let (_app, win, dir) = window("gh-bad-settings");
        save_list(&dir, 60);
        std::fs::write(dir.join("settings.json"), "{ not json").unwrap();
        let l = call(&win, "github_items", json!({ "force": false })).unwrap();
        assert_eq!(l["items"][0]["title"], "Fix cart");
        call(&win, "set_settings", json!({ "settings": { "refresh_minutes": 30 } })).unwrap();
        assert_eq!(call(&win, "get_settings", json!({})).unwrap()["refresh_minutes"], 30);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_window_cannot_change_github() {
        // The GitHub sync goes one way: the window can only read the list.
        let (_app, win, dir) = window("gh-read-only");
        save_list(&dir, 60);
        for cmd in ["github_mark_done", "github_close", "github_undo"] {
            assert!(call(&win, cmd, json!({ "id": "acme/shop#7" })).is_err(), "{cmd} must not exist");
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn clickup_list_comes_from_the_saved_list_while_it_is_fresh() {
        let (_app, win, dir) = window("cu-cache");
        let item: clickup::ClickupItem = serde_json::from_value(json!({
            "id": "86abc", "title": "Write notes", "url": "https://app.clickup.com/t/86abc",
            "status": "to do", "list": "Sprint", "workspace": "Rymera", "updated": 1
        }))
        .unwrap();
        if std::env::var_os("AIFRED_CLICKUP_TOKEN").is_none() {
            ClickupStore::new(&dir).save("x", clickup::MyTasks { items: vec![item.clone()], cut: vec![] }, now() - 60).unwrap();
            let e = call(&win, "clickup_items", json!({ "force": false })).unwrap_err();
            assert!(e.as_str().unwrap().contains("Settings page"), "no token: {e}");
            assert_eq!(call(&win, "clickup_token_status", json!({})).unwrap()["set"], false);
        }
        clickup::save_token(&dir, "pk_test").unwrap();
        assert_eq!(call(&win, "clickup_token_status", json!({})).unwrap()["set"], true);
        // The token in use: the file, or AIFRED_CLICKUP_TOKEN when it is set.
        let account = clickup::account_key(&clickup::token(&dir).unwrap());
        ClickupStore::new(&dir).save(&account, clickup::MyTasks { items: vec![item], cut: vec![] }, now() - 60).unwrap();
        // No ClickUp call here: the saved list is fresh.
        let l = call(&win, "clickup_items", json!({ "force": false })).unwrap();
        assert_eq!(l["items"][0]["title"], "Write notes");
        call(&win, "remove_clickup_token", json!({})).unwrap();
        assert!(!dir.join("clickup-token").exists());
        assert!(ClickupStore::new(&dir).cached(&account, u64::MAX, now()).unwrap().is_none(), "the list of the old account is gone");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn the_chat_model_is_remembered() {
        let (_app, win, dir) = window("chat-model");
        assert_eq!(call(&win, "chat_model", json!({})).unwrap(), Value::Null);
        call(&win, "set_chat_model", json!({ "model": "haiku" })).unwrap();
        assert_eq!(call(&win, "chat_model", json!({})).unwrap(), "haiku");
        assert!(call(&win, "set_chat_model", json!({ "model": "nope" })).is_err());
        call(&win, "set_chat_model", json!({ "model": null })).unwrap();
        assert_eq!(call(&win, "chat_model", json!({})).unwrap(), Value::Null);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn updates_are_off_in_tests_and_restart_is_refused() {
        let (_app, win, dir) = window("update");
        let st = call(&win, "update_status", json!({})).unwrap();
        assert_eq!((st["state"].clone(), st["current"].clone()), (json!("off"), json!(VERSION)));
        // No GitHub call: the check does nothing when updates are off.
        assert_eq!(call(&win, "check_update", json!({})).unwrap()["state"], "off");
        assert!(call(&win, "restart_to_update", json!({})).unwrap_err().as_str().unwrap().contains("off"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn info_shows_the_data_folder() {
        let (_app, win, dir) = window("info");
        let i = call(&win, "info", json!({})).unwrap();
        assert_eq!(i["data_dir"], dir.display().to_string());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn close_hides_the_main_window_when_there_is_a_tray() {
        let (_app, win, dir) = window("close-tray");
        assert!(hide_on_close(&win.as_ref().window(), true, false));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn close_quits_when_there_is_no_tray() {
        let (app, win, dir) = window("close-no-tray");
        assert!(!hide_on_close(&win.as_ref().window(), false, false));
        assert!(app.get_webview_window("main").is_some());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_the_main_window_hides() {
        let (app, _win, dir) = window("close-other");
        let other = tauri::WebviewWindowBuilder::new(&app, "other", Default::default()).build().unwrap();
        assert!(!hide_on_close(&other.as_ref().window(), true, false));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
