//! aifred desktop app. The window talks to aifred-core through these commands.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use aifred_core::backend::{Event, claude::ClaudeBackend};
use aifred_core::files::now;
use aifred_core::settings::{self, Settings};
use aifred_core::github;
use aifred_core::github_list::{GithubList, GithubStore};
use aifred_core::history::Turn;
use aifred_core::tasks::Task;
use aifred_core::{Answer, Assistant, default_data_dir};
use serde::Serialize;
use tauri::{Manager, State, ipc::Channel};
use tauri_plugin_window_state::{AppHandleExt, StateFlags};
use tokio::sync::Mutex;

/// The window settings that are saved and restored. "Visible" is left out:
/// a save during close can record "hidden", and the window must always show.
const WINDOW_STATE: StateFlags = StateFlags::all().difference(StateFlags::VISIBLE);

/// The window moved or changed size since the last save.
static WINDOW_CHANGED: AtomicBool = AtomicBool::new(false);

struct App {
    assistant: Arc<Assistant<ClaudeBackend>>,
    /// One question at a time: two at once would race on the saved session.
    busy: Mutex<()>,
}

#[derive(Serialize)]
struct Info {
    data_dir: String,
    backend: String,
    github_owners: Option<String>,
    api_key_set: bool,
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
    app.assistant.ask(&prompt, &mut send).await.map_err(err)
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

#[tauri::command]
async fn get_memory(app: State<'_, App>) -> CmdResult<String> {
    blocking(&app, |a| Ok(a.memory())).await
}

#[tauri::command]
async fn set_memory(app: State<'_, App>, text: String) -> CmdResult<()> {
    blocking(&app, move |a| a.set_memory(&text)).await
}

#[tauri::command]
async fn new_conversation(app: State<'_, App>) -> CmdResult<()> {
    let Ok(_guard) = app.busy.try_lock() else {
        return Err("Wait until the current question is done.".into());
    };
    app.assistant.new_conversation().map_err(err)
}

// The task file has its own lock (tasks.rs), so these do not need the busy lock.
#[tauri::command]
async fn tasks(app: State<'_, App>) -> CmdResult<Vec<Task>> {
    blocking(&app, |a| a.tasks().load()).await
}

#[tauri::command]
async fn add_task(app: State<'_, App>, title: String) -> CmdResult<Task> {
    blocking(&app, move |a| a.tasks().add(&title, "", "you")).await
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
        let saved = blocking(&app, move |_| {
            // A broken settings file must not hide the list: use the defaults.
            let minutes = settings::load_or_default(&d).github_refresh_minutes;
            // 0 means "only when I click reload": the saved list never gets too old.
            let max_age = if minutes == 0 { u64::MAX } else { u64::from(minutes) * 60 };
            GithubStore::new(&d).cached(max_age, now())
        })
        .await?;
        if let Some(list) = saved {
            return Ok(list);
        }
    }
    match github::my_items(&github::owners_from_env()).await {
        Ok(f) => blocking(&app, move |_| GithubStore::new(&dir).save_fetch(f.items, f.complete, now())).await,
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

#[derive(Serialize)]
struct DoneResult {
    /// The item is still open on GitHub.
    still_open: bool,
    /// The user may close it from AIfred (not a review request).
    can_close: bool,
    /// A pull request, not an issue. The window uses it for the close question.
    is_pull_request: bool,
}

/// Marks a GitHub item done in AIfred, then checks GitHub.
#[tauri::command]
async fn github_mark_done(app: State<'_, App>, id: String) -> CmdResult<DoneResult> {
    let dir = app.assistant.dir.clone();
    let item = blocking(&app, move |_| GithubStore::new(&dir).mark_done(&id, now())).await?;
    let open = github::is_open(&item, &github::owners_from_env())
        .await
        .map_err(|e| format!("Marked done in AIfred. Cannot check GitHub: {e:#}"))?;
    Ok(DoneResult { still_open: open, can_close: open && item.can_close(), is_pull_request: item.is_pull_request() })
}

/// Closes a done item on GitHub. The window calls this only after the user
/// clicks the close button.
#[tauri::command]
async fn github_close(app: State<'_, App>, id: String) -> CmdResult<()> {
    let dir = app.assistant.dir.clone();
    let d = dir.clone();
    let i = id.clone();
    let item = blocking(&app, move |_| GithubStore::new(&d).done_item(&i)).await?;
    github::close_item(&item, &github::owners_from_env()).await.map_err(err)?;
    blocking(&app, move |_| GithubStore::new(&dir).forget_open(&id)).await
}

#[tauri::command]
async fn github_undo(app: State<'_, App>, id: String) -> CmdResult<()> {
    let dir = app.assistant.dir.clone();
    blocking(&app, move |_| GithubStore::new(&dir).undo(&id)).await
}

#[tauri::command]
async fn get_settings(app: State<'_, App>) -> CmdResult<Settings> {
    blocking(&app, |a| settings::load(&a.dir)).await
}

#[tauri::command]
async fn set_settings(app: State<'_, App>, settings: Settings) -> CmdResult<()> {
    blocking(&app, move |a| settings::save(&a.dir, &settings)).await
}

#[tauri::command]
fn info(app: State<'_, App>) -> Info {
    Info {
        data_dir: app.assistant.dir.display().to_string(),
        backend: "Claude (your login)".into(),
        github_owners: std::env::var("AIFRED_GITHUB_OWNERS").ok(),
        api_key_set: std::env::var_os("ANTHROPIC_API_KEY").is_some(),
    }
}

/// Adds the app state and all commands. The tests use the same list.
fn with_commands<R: tauri::Runtime>(builder: tauri::Builder<R>, app: App) -> tauri::Builder<R> {
    builder.manage(app).invoke_handler(tauri::generate_handler![
        ask,
        history,
        get_memory,
        set_memory,
        new_conversation,
        tasks,
        add_task,
        set_task_done,
        delete_task,
        github_items,
        github_mark_done,
        github_close,
        github_undo,
        get_settings,
        set_settings,
        info
    ])
}

fn main() -> anyhow::Result<()> {
    let dir = default_data_dir()?;
    let backend = ClaudeBackend { work_dir: dir.join("claude"), model: std::env::var("AIFRED_MODEL").ok() };
    // The GitHub tool server is built into the same folder as this app.
    let tools_dir = std::env::current_exe()?.parent().map(Into::into).unwrap_or_default();
    let assistant = Arc::new(Assistant::new(dir, backend, tools_dir)?);

    // AIFRED_NO_WINDOW_STATE: do not read or write the saved window size.
    // scripts/run-hidden.sh sets it, so a check run cannot change the size
    // of the user's own window.
    let keep_window_state = std::env::var_os("AIFRED_NO_WINDOW_STATE").is_none();

    let mut builder = with_commands(tauri::Builder::default(), App { assistant, busy: Mutex::new(()) })
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
            // The plugin does not save or restore "visible", so this is the
            // only place that shows the window. Ignore errors: some window
            // managers refuse focus, and that must not stop the app.
            if let Some(w) = app.get_webview_window("main") {
                let _ = w.show();
                let _ = w.set_focus();
            }
            Ok(())
        })
        .on_window_event(move |w, e| match e {
            // The plugin writes the file only on a normal quit. A logout or a
            // crash skips that. So also write it when the window loses focus,
            // but only if the window moved or changed size since the last save.
            tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                WINDOW_CHANGED.store(true, Ordering::Relaxed);
            }
            tauri::WindowEvent::Focused(false) if keep_window_state && WINDOW_CHANGED.swap(false, Ordering::Relaxed) => {
                let _ = w.app_handle().save_window_state(WINDOW_STATE);
            }
            _ => {}
        })
        .run(tauri::generate_context!())?;
    Ok(())
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
        let app = with_commands(mock_builder(), App { assistant, busy: Mutex::new(()) })
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

        let t = call(&win, "add_task", json!({ "title": "Call Sam" })).unwrap();
        assert_eq!(t["title"], "Call Sam");
        assert_eq!(t["added_by"], "you", "tasks from the window are yours");
        let id = t["id"].as_str().unwrap().to_string();

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
        let e = call(&win, "add_task", json!({ "title": "  " })).unwrap_err();
        assert!(e.as_str().unwrap().contains("empty"), "got {e}");
        let e = call(&win, "delete_task", json!({ "id": "nope" })).unwrap_err();
        assert!(e.as_str().unwrap().contains("no task"), "got {e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn memory_round_trip_and_empty_history() {
        let (_app, win, dir) = window("memory");
        assert_eq!(call(&win, "get_memory", json!({})).unwrap(), "");
        call(&win, "set_memory", json!({ "text": "- I like short answers." })).unwrap();
        assert_eq!(call(&win, "get_memory", json!({})).unwrap(), "- I like short answers.");
        assert_eq!(call(&win, "history", json!({})).unwrap(), json!([]));
        call(&win, "new_conversation", json!({})).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn settings_round_trip_and_bad_value_is_refused() {
        let (_app, win, dir) = window("settings");
        assert_eq!(call(&win, "get_settings", json!({})).unwrap()["github_refresh_minutes"], 15);
        call(&win, "set_settings", json!({ "settings": { "github_refresh_minutes": 30 } })).unwrap();
        assert_eq!(call(&win, "get_settings", json!({})).unwrap()["github_refresh_minutes"], 30);
        assert!(call(&win, "set_settings", json!({ "settings": { "github_refresh_minutes": 99999 } })).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A saved GitHub list, as if GitHub was asked `ago` seconds earlier.
    fn save_list(dir: &std::path::Path, ago: u64) {
        let item: github::GithubItem = serde_json::from_value(json!({
            "id": "acme/shop#7", "repo": "acme/shop", "number": 7, "title": "Fix cart",
            "url": "https://github.com/acme/shop/pull/7", "reason": "review_requested", "updated": ""
        }))
        .unwrap();
        GithubStore::new(dir).save_fetch(vec![item], true, now() - ago).unwrap();
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
        call(&win, "set_settings", json!({ "settings": { "github_refresh_minutes": 0 } })).unwrap();
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
        call(&win, "set_settings", json!({ "settings": { "github_refresh_minutes": 30 } })).unwrap();
        assert_eq!(call(&win, "get_settings", json!({})).unwrap()["github_refresh_minutes"], 30);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn unknown_github_items_cannot_be_marked_done_or_closed() {
        let (_app, win, dir) = window("gh-unknown");
        save_list(&dir, 60);
        let e = call(&win, "github_mark_done", json!({ "id": "evil/repo#1" })).unwrap_err();
        assert!(e.as_str().unwrap().contains("not in the GitHub list"), "got {e}");
        let e = call(&win, "github_close", json!({ "id": "acme/shop#7" })).unwrap_err();
        assert!(e.as_str().unwrap().contains("not marked done"), "got {e}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn info_shows_the_data_folder() {
        let (_app, win, dir) = window("info");
        let i = call(&win, "info", json!({})).unwrap();
        assert_eq!(i["data_dir"], dir.display().to_string());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
