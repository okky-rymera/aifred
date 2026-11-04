//! Direct ClickUp calls, without the AI, so the list loads fast and costs nothing.
//!
//! AIfred reads the open tasks that are assigned to the user. The only change
//! it makes is the status of one task: "mark done" and its undo. The user
//! starts both from the window. The chat tools only read.
//!
//! The login is a personal API token (it starts with "pk_"). It is kept in
//! the file clickup-token in the data folder, readable only by the user.
//! AIFRED_CLICKUP_TOKEN overrides the file.

use std::path::{Path, PathBuf};

use anyhow::{Context, bail};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

const API: &str = "https://api.clickup.com/api/v2";
/// ClickUp sends at most 100 tasks on one page.
const PAGE_SIZE: usize = 100;
/// The most pages we read from one workspace: 1000 tasks.
const MAX_PAGES: u32 = 10;

// ----- Token -----

fn token_path(dir: &Path) -> PathBuf {
    dir.join("clickup-token")
}

/// Where the token comes from. The window shows it, never the token itself.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct TokenStatus {
    pub set: bool,
    /// True when AIFRED_CLICKUP_TOKEN gives the token. The window then
    /// cannot change or remove it.
    pub from_env: bool,
}

/// The token from AIFRED_CLICKUP_TOKEN, or else from the file.
pub fn token(dir: &Path) -> Option<String> {
    if let Some(t) = std::env::var("AIFRED_CLICKUP_TOKEN").ok().filter(|t| !t.trim().is_empty()) {
        return Some(t.trim().to_string());
    }
    let t = std::fs::read_to_string(token_path(dir)).ok()?;
    let t = t.trim();
    (!t.is_empty()).then(|| t.to_string())
}

/// A short name for the ClickUp account of a token, so the saved list is
/// not shown for another account. It is a hash: the token cannot be read
/// back from it. A Rust update can change the hash. Then the list loads again once.
pub fn account_key(token: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.trim().hash(&mut h);
    format!("{:016x}", h.finish())
}

pub fn token_status(dir: &Path) -> TokenStatus {
    let from_env = std::env::var("AIFRED_CLICKUP_TOKEN").is_ok_and(|t| !t.trim().is_empty());
    TokenStatus { set: token(dir).is_some(), from_env }
}

/// A personal token is one line of letters, digits, and "_".
fn check_token(t: &str) -> anyhow::Result<()> {
    if t.is_empty() || !t.chars().all(|c| c.is_ascii_alphanumeric() || c == '_') {
        bail!("this does not look like a ClickUp token. Copy it from ClickUp: Settings > Apps > API Token");
    }
    Ok(())
}

/// Saves the token. Only the user can read the file.
pub fn save_token(dir: &Path, token: &str) -> anyhow::Result<()> {
    let token = token.trim();
    check_token(token)?;
    let path = token_path(dir);
    let tmp = path.with_extension("tmp");
    // A crash can leave an old temp file with other permissions. The mode
    // below applies only to a new file, so remove the old one first.
    match std::fs::remove_file(&tmp) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
            return Err(e).with_context(|| format!("cannot remove {}", tmp.display()));
        }
        _ => {}
    }
    {
        use std::io::Write;
        let mut opts = std::fs::OpenOptions::new();
        // create_new: fail if another file appears at this path in the meantime.
        opts.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let mut f = opts.open(&tmp).with_context(|| format!("cannot write {}", tmp.display()))?;
        f.write_all(token.as_bytes())?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, &path).with_context(|| format!("cannot write {}", path.display()))
}

pub fn remove_token(dir: &Path) -> anyhow::Result<()> {
    match std::fs::remove_file(token_path(dir)) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
        _ => Ok(()),
    }
}

// ----- Items -----

/// One open ClickUp task that is assigned to the user.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ClickupItem {
    pub id: String,
    /// The team's own task id, like "DEV-42", when the workspace uses them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub custom_id: Option<String>,
    pub title: String,
    pub url: String,
    /// The status name, like "in progress".
    pub status: String,
    /// The status color, like "#d3d3d3".
    #[serde(default)]
    pub status_color: String,
    /// The list that holds the task.
    pub list: String,
    /// The workspace name.
    pub workspace: String,
    /// "urgent", "high", "normal", or "low".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub priority: Option<String>,
    /// Unix milliseconds.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub due: Option<u64>,
    /// Unix milliseconds.
    pub updated: u64,
    /// The description as plain text. It goes into the details column of the
    /// database (db.rs), not into the saved item, so the window and the list
    /// tools do not get it.
    #[serde(skip)]
    pub description: Option<String>,
}

#[derive(Deserialize)]
struct RawTasks {
    tasks: Vec<RawTask>,
    #[serde(default)]
    last_page: Option<bool>,
}

#[derive(Deserialize)]
struct RawTask {
    id: String,
    #[serde(default)]
    custom_id: Option<String>,
    name: String,
    url: String,
    status: RawStatus,
    #[serde(default)]
    list: Option<RawNamed>,
    #[serde(default)]
    priority: Option<RawPriority>,
    #[serde(default)]
    due_date: Option<String>,
    #[serde(default)]
    date_updated: Option<String>,
    #[serde(default)]
    text_content: Option<String>,
    #[serde(default)]
    description: Option<String>,
}

#[derive(Deserialize)]
struct RawStatus {
    status: String,
    #[serde(default)]
    color: String,
    #[serde(default, rename = "type")]
    kind: String,
}

#[derive(Deserialize)]
struct RawNamed {
    id: String,
    #[serde(default)]
    name: String,
}

#[derive(Deserialize)]
struct RawPriority {
    priority: String,
}

/// ClickUp sends times as text: Unix milliseconds.
fn millis(s: Option<&str>) -> Option<u64> {
    s.and_then(|s| s.parse().ok())
}

/// ClickUp has two kinds of finished status: "done" (for example "shipped")
/// and "closed". include_closed=false leaves out only the closed tasks, so the
/// done tasks are left out here. The check also guards against closed tasks.
fn is_open(kind: &str) -> bool {
    kind != "done" && kind != "closed"
}

/// Turns one page of tasks into items. Also returns true if this was the last page.
fn parse_tasks(json: &[u8], workspace: &str) -> anyhow::Result<(Vec<ClickupItem>, bool)> {
    let raw: RawTasks = serde_json::from_slice(json).context("ClickUp sent a task list we cannot read")?;
    let last = raw.last_page.unwrap_or(raw.tasks.len() < PAGE_SIZE);
    let items = raw
        .tasks
        .into_iter()
        .filter(|t| is_open(&t.status.kind))
        .map(|t| ClickupItem {
            id: t.id,
            custom_id: t.custom_id.filter(|c| !c.is_empty()),
            title: t.name,
            url: t.url,
            status: t.status.status,
            status_color: t.status.color,
            list: t.list.map(|l| l.name).unwrap_or_default(),
            workspace: workspace.to_string(),
            priority: t.priority.map(|p| p.priority),
            due: millis(t.due_date.as_deref()),
            updated: millis(t.date_updated.as_deref()).unwrap_or_default(),
            description: Some(description_text(t.text_content.as_deref(), t.description.as_deref())),
        })
        .collect();
    Ok((items, last))
}

/// The description that AIfred saves: the plain text, or else the
/// description field. The list download and the task_details tool both use
/// it, so the saved text (and its summary) does not change between them.
pub fn description_text(text_content: Option<&str>, description: Option<&str>) -> String {
    text_content.filter(|t| !t.trim().is_empty()).or(description).unwrap_or_default().trim().to_string()
}

#[derive(Deserialize)]
struct RawStatuses {
    statuses: Vec<RawStatus>,
}

/// The status that "mark done" moves a task to: the "closed" status of its
/// list. ClickUp's own "mark complete" button does the same. A list without
/// one falls back to a "done" status.
fn done_status(list_json: &[u8]) -> anyhow::Result<String> {
    let raw: RawStatuses = serde_json::from_slice(list_json).context("ClickUp sent a list we cannot read")?;
    for kind in ["closed", "done"] {
        if let Some(s) = raw.statuses.iter().find(|s| s.kind == kind) {
            return Ok(s.status.clone());
        }
    }
    bail!("the task's list has no closed status")
}

/// A ClickUp task id is letters, digits, "-", and "_". The check keeps an
/// id from changing the address of the call.
pub fn check_task_id(id: &str) -> anyhow::Result<()> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        bail!("bad ClickUp task id: {id:?}");
    }
    Ok(())
}

/// ClickUp's own task ids are letters and digits only. A team's custom id
/// has a prefix and a "-", like "DEV-42".
fn is_custom_id(id: &str) -> bool {
    id.contains('-')
}

/// The result of Client::my_tasks.
#[derive(Debug, Clone, Default)]
pub struct MyTasks {
    pub items: Vec<ClickupItem>,
    /// The workspaces that have more tasks than AIfred reads (MAX_PAGES
    /// pages). Their lists are not complete.
    pub cut: Vec<String>,
}

/// The message for workspaces whose lists are not complete. None when all are complete.
pub fn cut_warning(cut: &[String]) -> Option<String> {
    if cut.is_empty() {
        return None;
    }
    let max = PAGE_SIZE * MAX_PAGES as usize;
    Some(format!(
        "ClickUp has more than {max} open tasks for you in {}. AIfred shows only the first {max} from each workspace.",
        cut.join(", ")
    ))
}

// ----- Calls -----

#[derive(Deserialize)]
struct RawUser {
    user: RawUserInner,
}

#[derive(Deserialize)]
struct RawUserInner {
    id: u64,
    #[serde(default)]
    username: Option<String>,
}

#[derive(Deserialize)]
struct RawTeams {
    teams: Vec<RawNamed>,
}

#[derive(Deserialize)]
struct RawTaskHead {
    status: RawStatus,
    list: RawNamed,
}

#[derive(Deserialize)]
struct RawError {
    #[serde(default)]
    err: String,
}

pub struct Client {
    http: reqwest::Client,
    token: String,
    base: String,
}

impl Client {
    pub fn new(token: &str) -> Self {
        Self::with_base(token, API)
    }

    /// For tests: a fake ClickUp server.
    pub fn with_base(token: &str, base: &str) -> Self {
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .user_agent("aifred")
            .build()
            .expect("the HTTP client settings are valid");
        Self { http, token: token.into(), base: base.trim_end_matches('/').into() }
    }

    /// Sends one call. Returns the reply body, or an error with ClickUp's message.
    async fn send(&self, req: reqwest::RequestBuilder) -> anyhow::Result<Vec<u8>> {
        let res = req.header("Authorization", &self.token).send().await.context("cannot reach ClickUp")?;
        let status = res.status();
        let body = res.bytes().await.context("cannot read the ClickUp reply")?.to_vec();
        if status.is_success() {
            return Ok(body);
        }
        let msg = serde_json::from_slice::<RawError>(&body).map(|e| e.err).unwrap_or_default();
        if status == reqwest::StatusCode::UNAUTHORIZED {
            bail!("ClickUp refused the token. Add a new token on the Settings page. ({msg})");
        }
        if status == reqwest::StatusCode::TOO_MANY_REQUESTS {
            bail!("ClickUp asks AIfred to slow down. Try again in one minute.");
        }
        bail!("ClickUp said {status}: {msg}")
    }

    async fn get(&self, path: &str, query: &[(&str, String)]) -> anyhow::Result<Vec<u8>> {
        self.send(self.http.get(format!("{}{path}", self.base)).query(query)).await
    }

    async fn get_json<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let body = self.get(path, &[]).await?;
        serde_json::from_slice(&body).with_context(|| format!("ClickUp sent a reply we cannot read ({path})"))
    }

    /// The user's ClickUp id and name. Also a quick check of the token.
    pub async fn me(&self) -> anyhow::Result<(u64, String)> {
        let u: RawUser = self.get_json("/user").await?;
        Ok((u.user.id, u.user.username.unwrap_or_default()))
    }

    /// The open tasks assigned to the user, in all workspaces.
    pub async fn my_tasks(&self) -> anyhow::Result<MyTasks> {
        let (me, _) = self.me().await?;
        let teams: RawTeams = self.get_json("/team").await?;
        let mut all = Vec::new();
        let mut cut = Vec::new();
        for team in teams.teams {
            check_task_id(&team.id)?;
            let mut done = false;
            for page in 0..MAX_PAGES {
                let query = [
                    ("page", page.to_string()),
                    ("assignees[]", me.to_string()),
                    ("subtasks", "true".to_string()),
                    ("include_closed", "false".to_string()),
                    ("order_by", "updated".to_string()),
                ];
                let body = self.get(&format!("/team/{}/task", team.id), &query).await?;
                let (items, last) = parse_tasks(&body, &team.name)?;
                all.extend(items);
                if last {
                    done = true;
                    break;
                }
            }
            if !done {
                cut.push(team.name);
            }
        }
        Ok(MyTasks { items: all, cut })
    }

    /// One task and its last comments, as ClickUp sends them. For the chat tools.
    ///
    /// The id can be ClickUp's own id ("86abc123") or the team's custom id
    /// ("DEV-42"). ClickUp finds a custom id only with custom_task_ids=true and
    /// the workspace id, so a custom id is tried in each workspace.
    pub async fn task_with_comments(&self, id: &str) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
        check_task_id(id)?;
        if !is_custom_id(id) {
            return self.task_and_comments(id, &[]).await;
        }
        let teams: RawTeams = self.get_json("/team").await?;
        let mut last_err = None;
        for team in teams.teams {
            check_task_id(&team.id)?;
            let query = [("custom_task_ids", "true".to_string()), ("team_id", team.id)];
            match self.task_and_comments(id, &query).await {
                Ok(found) => return Ok(found),
                Err(e) => last_err = Some(e),
            }
        }
        Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no ClickUp workspace has the task {id}")))
    }

    async fn task_and_comments(
        &self,
        id: &str,
        query: &[(&str, String)],
    ) -> anyhow::Result<(serde_json::Value, serde_json::Value)> {
        let read = |body: Vec<u8>| -> anyhow::Result<serde_json::Value> {
            serde_json::from_slice(&body).context("ClickUp sent a task we cannot read")
        };
        let task = read(self.get(&format!("/task/{id}"), query).await?)?;
        let comments = read(self.get(&format!("/task/{id}/comment"), query).await?)?;
        Ok((task, comments))
    }

    /// Moves the task to the closed status of its list.
    /// Returns the status it had before, so the window can undo.
    pub async fn mark_done(&self, id: &str) -> anyhow::Result<String> {
        check_task_id(id)?;
        let head: RawTaskHead = self.get_json(&format!("/task/{id}")).await?;
        if !is_open(&head.status.kind) {
            bail!("this task is already finished in ClickUp (status \"{}\")", head.status.status);
        }
        check_task_id(&head.list.id)?;
        let list = self.get(&format!("/list/{}", head.list.id), &[]).await?;
        let done = done_status(&list)?;
        self.set_status(id, &done).await?;
        Ok(head.status.status)
    }

    /// Sets the status of one task. The undo of mark_done uses it.
    pub async fn set_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        check_task_id(id)?;
        if status.trim().is_empty() {
            bail!("the status is empty");
        }
        let body = serde_json::json!({ "status": status });
        self.send(self.http.put(format!("{}/task/{id}", self.base)).json(&body)).await?;
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-clickup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const PAGE: &str = r##"{"tasks":[
        {"id":"86abc","custom_id":"DEV-42","name":"Write release notes","url":"https://app.clickup.com/t/86abc",
         "status":{"status":"in progress","color":"#4194f6","type":"custom"},
         "list":{"id":"901","name":"Sprint 12"},"priority":{"priority":"high","color":"#ffcc00"},
         "due_date":"1767225600000","date_updated":"1761900000000"},
        {"id":"86def","custom_id":null,"name":"Old one","url":"https://app.clickup.com/t/86def",
         "status":{"status":"complete","color":"#6bc950","type":"closed"},
         "list":{"id":"901","name":"Sprint 12"},"priority":null,"due_date":null,"date_updated":"1761800000000"},
        {"id":"86ghi","name":"Shipped one","url":"https://app.clickup.com/t/86ghi",
         "status":{"status":"shipped","color":"#6bc950","type":"done"},
         "list":{"id":"901","name":"Sprint 12"},"date_updated":"1761700000000"}
    ],"last_page":false}"##;

    #[test]
    fn parse_tasks_reads_a_page_and_skips_done_and_closed_tasks() {
        let (items, last) = parse_tasks(PAGE.as_bytes(), "Rymera").unwrap();
        assert!(!last, "ClickUp said there are more pages");
        assert_eq!(items.len(), 1, "the closed task and the done task are left out");
        let i = &items[0];
        assert_eq!(i.id, "86abc");
        assert_eq!(i.custom_id.as_deref(), Some("DEV-42"));
        assert_eq!(i.title, "Write release notes");
        assert_eq!(i.status, "in progress");
        assert_eq!(i.list, "Sprint 12");
        assert_eq!(i.workspace, "Rymera");
        assert_eq!(i.priority.as_deref(), Some("high"));
        assert_eq!(i.due, Some(1767225600000));
        assert_eq!(i.updated, 1761900000000);
    }

    #[test]
    fn a_short_page_without_last_page_is_the_last() {
        let (_, last) = parse_tasks(br#"{"tasks":[]}"#, "W").unwrap();
        assert!(last);
        assert!(parse_tasks(b"not json", "W").is_err());
    }

    #[test]
    fn done_status_is_the_closed_one_then_done() {
        let list = br#"{"statuses":[{"status":"to do","type":"open"},{"status":"shipped","type":"done"},{"status":"complete","type":"closed"}]}"#;
        assert_eq!(done_status(list).unwrap(), "complete");
        let list = br#"{"statuses":[{"status":"to do","type":"open"},{"status":"shipped","type":"done"}]}"#;
        assert_eq!(done_status(list).unwrap(), "shipped");
        assert!(done_status(br#"{"statuses":[{"status":"to do","type":"open"}]}"#).is_err());
    }

    #[test]
    fn task_ids_cannot_change_the_address() {
        assert!(check_task_id("86abc").is_ok());
        assert!(check_task_id("DEV-42").is_ok());
        for bad in ["", "../user", "a/b", "a?b=1", "a b"] {
            assert!(check_task_id(bad).is_err(), "{bad:?} must be refused");
        }
    }

    #[test]
    fn custom_ids_have_a_dash_and_clickup_ids_do_not() {
        assert!(is_custom_id("DEV-42"));
        assert!(!is_custom_id("86abc123"));
    }

    #[test]
    fn a_left_over_temp_file_does_not_make_the_token_readable() {
        let d = temp("token-tmp");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let tmp = d.join("clickup-token.tmp");
            std::fs::write(&tmp, "old").unwrap();
            std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o644)).unwrap();
            save_token(&d, "pk_1_ABC").unwrap();
            let mode = std::fs::metadata(d.join("clickup-token")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the user can read the token");
            assert!(!tmp.exists());
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn token_file_round_trip_is_private_and_checked() {
        // AIFRED_CLICKUP_TOKEN comes before the file, so this test needs it unset.
        if std::env::var_os("AIFRED_CLICKUP_TOKEN").is_some() {
            eprintln!("skipped: AIFRED_CLICKUP_TOKEN is set");
            return;
        }
        let d = temp("token");
        assert_eq!(token(&d), None);
        assert!(save_token(&d, "not a token!").is_err());
        save_token(&d, "  pk_123_ABC\n").unwrap();
        assert_eq!(token(&d).as_deref(), Some("pk_123_ABC"));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(d.join("clickup-token")).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the user can read the token");
        }
        remove_token(&d).unwrap();
        remove_token(&d).unwrap(); // No error when it is not there.
        assert_eq!(token(&d), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A fake ClickUp server. It answers each call with `reply(method, path)`,
    /// and records "METHOD path body" for each call.
    async fn fake_clickup(
        reply: impl Fn(&str, &str) -> (u16, String) + Send + Sync + 'static,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let log = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let seen = log.clone();
        let reply = std::sync::Arc::new(reply);
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut buf = Vec::new();
                let mut chunk = [0u8; 4096];
                // Read the head, then the body by its length.
                let (head, body) = loop {
                    let n = sock.read(&mut chunk).await.unwrap();
                    if n == 0 {
                        break (String::new(), String::new());
                    }
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some(end) = text.find("\r\n\r\n") {
                        let head = text[..end].to_string();
                        let len = head
                            .lines()
                            .find_map(|l| l.to_ascii_lowercase().strip_prefix("content-length:").map(|v| v.trim().parse::<usize>().unwrap()))
                            .unwrap_or(0);
                        if buf.len() >= end + 4 + len {
                            break (head, text[end + 4..end + 4 + len].to_string());
                        }
                    }
                };
                let first = head.lines().next().unwrap_or_default().to_string();
                let mut parts = first.split(' ');
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                assert!(head.contains("authorization: pk_test") || head.contains("Authorization: pk_test"), "{head}");
                seen.lock().unwrap().push(format!("{method} {path} {body}").trim().to_string());
                let (code, json) = reply(&method, &path);
                let res = format!(
                    "HTTP/1.1 {code} X\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{json}",
                    json.len()
                );
                sock.write_all(res.as_bytes()).await.unwrap();
            }
        });
        (format!("http://{addr}/api/v2"), log)
    }

    #[tokio::test]
    async fn my_tasks_reads_every_workspace_and_page() {
        let (base, log) = fake_clickup(|_, path| {
            let page = |n: usize, last: bool| {
                let tasks: Vec<String> = (0..n)
                    .map(|i| format!(r#"{{"id":"t{i}","name":"T{i}","url":"u","status":{{"status":"open","type":"open"}},"list":{{"id":"1","name":"L"}}}}"#))
                    .collect();
                format!(r#"{{"tasks":[{}],"last_page":{last}}}"#, tasks.join(","))
            };
            match path {
                "/api/v2/user" => (200, r#"{"user":{"id":7,"username":"okky"}}"#.into()),
                "/api/v2/team" => (200, r#"{"teams":[{"id":"111","name":"A"},{"id":"222","name":"B"}]}"#.into()),
                p if p.starts_with("/api/v2/team/111/task?page=0") => (200, page(2, false)),
                p if p.starts_with("/api/v2/team/111/task?page=1") => (200, page(1, true)),
                p if p.starts_with("/api/v2/team/222/task?page=0") => (200, page(1, true)),
                _ => (404, r#"{"err":"no"}"#.into()),
            }
        })
        .await;
        let got = Client::with_base("pk_test", &base).my_tasks().await.unwrap();
        assert!(got.cut.is_empty(), "every list is complete");
        let items = got.items;
        assert_eq!(items.len(), 4);
        assert_eq!(items.iter().filter(|i| i.workspace == "A").count(), 3);
        let calls = log.lock().unwrap().clone();
        let first = calls.iter().find(|c| c.contains("/team/111/task?page=0")).unwrap();
        assert!(first.contains("assignees%5B%5D=7"), "only tasks assigned to the user: {first}");
        assert!(first.contains("include_closed=false"), "{first}");
        assert!(calls.iter().all(|c| c.starts_with("GET ")), "reading changes nothing: {calls:?}");
    }

    #[tokio::test]
    async fn my_tasks_says_which_workspace_has_more_tasks_than_it_reads() {
        let (base, _) = fake_clickup(|_, path| match path {
            "/api/v2/user" => (200, r#"{"user":{"id":7}}"#.into()),
            "/api/v2/team" => (200, r#"{"teams":[{"id":"111","name":"Big"},{"id":"222","name":"Small"}]}"#.into()),
            // Workspace 111 always says there are more pages.
            p if p.starts_with("/api/v2/team/111/task") => (200, r#"{"tasks":[],"last_page":false}"#.into()),
            p if p.starts_with("/api/v2/team/222/task") => (200, r#"{"tasks":[],"last_page":true}"#.into()),
            _ => (404, r#"{"err":"no"}"#.into()),
        })
        .await;
        let got = Client::with_base("pk_test", &base).my_tasks().await.unwrap();
        assert_eq!(got.cut, vec!["Big"]);
        let w = cut_warning(&got.cut).unwrap();
        assert!(w.contains("more than 1000") && w.contains("Big"), "{w}");
        assert_eq!(cut_warning(&[]), None);
    }

    #[tokio::test]
    async fn a_custom_id_is_found_in_the_workspace_that_has_it() {
        let (base, log) = fake_clickup(|_, path| match path {
            "/api/v2/team" => (200, r#"{"teams":[{"id":"111","name":"A"},{"id":"222","name":"B"}]}"#.into()),
            p if p.starts_with("/api/v2/task/DEV-42?") && p.contains("team_id=222") => (200, r#"{"id":"86abc","name":"T"}"#.into()),
            p if p.starts_with("/api/v2/task/DEV-42/comment?") && p.contains("team_id=222") => (200, r#"{"comments":[]}"#.into()),
            "/api/v2/task/86abc" => (200, r#"{"id":"86abc","name":"T"}"#.into()),
            "/api/v2/task/86abc/comment" => (200, r#"{"comments":[]}"#.into()),
            _ => (404, r#"{"err":"Task not found"}"#.into()),
        })
        .await;
        let c = Client::with_base("pk_test", &base);
        let (task, _) = c.task_with_comments("DEV-42").await.unwrap();
        assert_eq!(task["id"], "86abc");
        let calls = log.lock().unwrap().clone();
        assert!(calls.iter().any(|c| c.contains("custom_task_ids=true")), "{calls:?}");
        let (task, _) = c.task_with_comments("86abc").await.unwrap();
        assert_eq!(task["id"], "86abc");
        let e = c.task_with_comments("NOPE-1").await.unwrap_err().to_string();
        assert!(e.contains("Task not found"), "{e}");
    }

    #[tokio::test]
    async fn mark_done_moves_the_task_to_the_closed_status_and_returns_the_old_one() {
        let (base, log) = fake_clickup(|method, path| match (method, path) {
            ("GET", "/api/v2/task/86abc") => {
                (200, r#"{"status":{"status":"in progress","type":"custom"},"list":{"id":"901","name":"L"}}"#.into())
            }
            ("GET", "/api/v2/list/901") => {
                (200, r#"{"statuses":[{"status":"to do","type":"open"},{"status":"complete","type":"closed"}]}"#.into())
            }
            ("PUT", "/api/v2/task/86abc") => (200, "{}".into()),
            _ => (404, r#"{"err":"no"}"#.into()),
        })
        .await;
        let c = Client::with_base("pk_test", &base);
        assert_eq!(c.mark_done("86abc").await.unwrap(), "in progress");
        let calls = log.lock().unwrap().clone();
        assert_eq!(calls.last().unwrap(), r#"PUT /api/v2/task/86abc {"status":"complete"}"#);
        assert!(c.mark_done("../user").await.is_err(), "a bad id never reaches ClickUp");
    }

    #[tokio::test]
    async fn a_refused_token_says_what_to_do() {
        let (base, _) = fake_clickup(|_, _| (401, r#"{"err":"Token invalid","ECODE":"OAUTH_025"}"#.into())).await;
        let e = Client::with_base("pk_test", &base).my_tasks().await.unwrap_err().to_string();
        assert!(e.contains("Settings page") && e.contains("Token invalid"), "{e}");
    }
}
