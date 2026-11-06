//! One connection to the user's Google account, for all Google features:
//! the calendar, Gmail, and Gmail drafts now, and later Docs and Sheets.
//!
//! Each user makes their own Google app registration (an OAuth client of
//! the type "Desktop app") in Google Cloud Console and gives AIfred its
//! client ID and secret. Google says that a desktop app cannot keep the
//! secret secret, so it is only a part of the registration.
//!
//! The login follows Google's guide for desktop apps: the browser opens
//! Google's login page, Google sends the user back to a short-lived web
//! server on 127.0.0.1, and PKCE protects the code. AIfred keeps the refresh
//! token in google.json, which only the user can read.
//!
//! Google does not let a desktop app add access to an existing login. So a
//! login asks for the access of all features. When a new feature needs more
//! access, the user connects again, and the new token replaces the old one.

use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{Context, bail};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};

use crate::files;

/// Read the user's calendars.
pub const CALENDAR: &str = "https://www.googleapis.com/auth/calendar.readonly";
/// Read the user's email.
pub const GMAIL: &str = "https://www.googleapis.com/auth/gmail.readonly";
/// Make and delete Gmail drafts (drafts.rs). Google has no narrower access
/// for drafts: this access can also send email. AIfred never sends: no
/// code of AIfred calls Gmail's send.
pub const GMAIL_COMPOSE: &str = "https://www.googleapis.com/auth/gmail.compose";
/// The account's email, so the window can show which account is connected.
const IDENTITY: &[&str] = &["openid", "email"];

/// A feature and the access it needs.
pub struct Feature {
    pub name: &'static str,
    pub scopes: &'static [&'static str],
    /// The user can turn the feature off on the Settings page. Then the
    /// login does not ask for its access, and AIfred does not use it.
    pub optional: bool,
    /// An optional feature that is off until the user turns it on: a
    /// feature that changes things in Google.
    pub starts_off: bool,
    /// The feature that this feature needs. When that feature is off, this
    /// feature is off too, and the login does not ask for its access.
    pub needs: Option<&'static str>,
    /// Removes what AIfred saved of the feature: when the user turns it
    /// off, or when the login ends.
    pub forget: fn(&Path) -> anyhow::Result<()>,
}

/// All features that use Google. A login asks for the access of all the
/// features that are not turned off.
pub const FEATURES: &[Feature] = &[
    Feature {
        name: "Calendar",
        scopes: &[CALENDAR],
        optional: false,
        starts_off: false,
        needs: None,
        forget: crate::calendar::forget,
    },
    Feature {
        name: "Gmail",
        scopes: &[GMAIL],
        optional: true,
        starts_off: false,
        needs: None,
        forget: crate::gmail::forget,
    },
    Feature {
        name: DRAFTS,
        scopes: &[GMAIL_COMPOSE],
        optional: true,
        starts_off: true,
        // A draft is a reply to an email that AIfred read in Gmail.
        needs: Some("Gmail"),
        // The drafts are in Gmail, and the log of AIfred's actions stays.
        forget: |_| Ok(()),
    },
];

/// The name of the Gmail drafts feature.
pub const DRAFTS: &str = "Gmail drafts";

/// The longest wait for the user to finish the login in the browser.
pub const LOGIN_WAIT: Duration = Duration::from_secs(300);
/// A token that ends within this many seconds is renewed first.
const RENEW_EARLY_SECS: u64 = 60;

/// Google's addresses. Tests use a fake server.
#[derive(Clone)]
pub struct Endpoints {
    pub auth: String,
    pub token: String,
    pub revoke: String,
}

impl Default for Endpoints {
    fn default() -> Self {
        Self {
            auth: "https://accounts.google.com/o/oauth2/v2/auth".into(),
            token: "https://oauth2.googleapis.com/token".into(),
            revoke: "https://oauth2.googleapis.com/revoke".into(),
        }
    }
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
struct Saved {
    #[serde(default)]
    client_id: String,
    #[serde(default)]
    client_secret: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    access_token: Option<String>,
    /// When the access token ends, in Unix seconds.
    #[serde(default)]
    expires_at: u64,
    /// The access that the user allowed. The user can untick some on Google's page.
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default)]
    email: Option<String>,
    /// The id of the login that waits for the browser. Only that login can
    /// save its tokens. A new login or a disconnect changes or removes it,
    /// so an old login that ends late cannot connect the account again.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pending: Option<String>,
    /// The optional features that the user turned off. A choice of the
    /// user, so a new login or a new registration keeps it.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    off: Vec<String>,
    /// The features with `starts_off` that the user turned on.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    on: Vec<String>,
}

impl Saved {
    /// True when the user turned the feature `f` off, or did not turn it on.
    fn own_off(&self, f: &Feature) -> bool {
        f.optional
            && if f.starts_off {
                !self.on.iter().any(|o| o == f.name)
            } else {
                self.off.iter().any(|o| o == f.name)
            }
    }

    /// True when the feature `name` is off: by the user's choice, or
    /// because a feature that it needs is off.
    fn is_off(&self, name: &str) -> bool {
        FEATURES
            .iter()
            .find(|f| f.name == name)
            .is_some_and(|f| self.own_off(f) || f.needs.is_some_and(|n| self.is_off(n)))
    }

    /// The names of the optional features that are off now.
    fn turned_off(&self) -> Vec<String> {
        FEATURES
            .iter()
            .filter(|f| f.optional && self.is_off(f.name))
            .map(|f| f.name.to_string())
            .collect()
    }
}

fn path(dir: &Path) -> PathBuf {
    dir.join("google.json")
}

fn load(dir: &Path) -> anyhow::Result<Saved> {
    files::load(&path(dir))
}

fn save(dir: &Path, s: &Saved) -> anyhow::Result<()> {
    files::save_private(&path(dir), s)
}

/// Runs `f` with google.json locked. Every read and save that changes the
/// file uses it: the window and the tool servers can renew a token at the
/// same time, and one must not overwrite the change of the other. Never
/// wait for the network inside `f`.
fn locked<R>(dir: &Path, f: impl FnOnce() -> anyhow::Result<R>) -> anyhow::Result<R> {
    files::locked(&path(dir), f)
}

/// Removes the login and keeps the registration. A file that AIfred
/// cannot read is replaced, so the user can connect again. With
/// `keep_pending`, a login that waits for the browser can still finish.
fn clear_login(dir: &Path, keep_pending: bool) -> anyhow::Result<()> {
    let s = load(dir).unwrap_or_default();
    save(
        dir,
        &Saved {
            client_id: s.client_id,
            client_secret: s.client_secret,
            pending: if keep_pending { s.pending } else { None },
            off: s.off,
            on: s.on,
            ..Saved::default()
        },
    )
}

/// What the window shows. Never a secret or a token.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct GoogleStatus {
    /// The client ID of the user's registration, or "" when there is none.
    pub client_id: String,
    pub secret_set: bool,
    pub connected: bool,
    pub email: Option<String>,
    /// Each feature, and true when the login allows it.
    pub features: Vec<(String, bool)>,
    /// The features that the user can turn off.
    pub optional: Vec<String>,
    /// The features that the user turned off.
    pub off: Vec<String>,
}

pub fn status(dir: &Path) -> anyhow::Result<GoogleStatus> {
    let s = load(dir)?;
    let connected = s.refresh_token.is_some();
    Ok(GoogleStatus {
        client_id: s.client_id.clone(),
        secret_set: !s.client_secret.is_empty(),
        connected,
        email: s.email.clone(),
        features: FEATURES
            .iter()
            .map(|f| {
                let on = connected && f.scopes.iter().all(|x| s.scopes.iter().any(|g| g == x));
                (f.name.to_string(), on)
            })
            .collect(),
        optional: FEATURES
            .iter()
            .filter(|f| f.optional)
            .map(|f| f.name.to_string())
            .collect(),
        off: s.turned_off(),
    })
}

/// Who the login belongs to: the registration and the Google account.
/// None when Google is not connected. The saved calendar keeps it, so the
/// app never shows events of a login that ended (calendar.rs).
pub fn account(dir: &Path) -> anyhow::Result<Option<String>> {
    let s = load(dir)?;
    Ok(s.refresh_token
        .as_ref()
        .map(|_| format!("{} {}", s.client_id, s.email.as_deref().unwrap_or_default())))
}

/// True when Google is connected and the login allows `scope`. A login
/// from before a feature was added does not allow it: the user must
/// connect again.
/// A feature that the user turned off counts as not allowed.
pub fn allows(dir: &Path, scope: &str) -> anyhow::Result<bool> {
    let s = load(dir)?;
    let turned_off = s.turned_off();
    let off = FEATURES
        .iter()
        .any(|f| f.scopes.contains(&scope) && turned_off.iter().any(|o| o == f.name));
    Ok(!off && s.refresh_token.is_some() && s.scopes.iter().any(|g| g == scope))
}

/// True when Google is connected and the login allows `scope`, also when
/// the user turned the feature off. Undo of a Gmail draft uses it.
pub(crate) fn login_allows(dir: &Path, scope: &str) -> anyhow::Result<bool> {
    let s = load(dir)?;
    Ok(s.refresh_token.is_some() && s.scopes.iter().any(|g| g == scope))
}

/// Turns an optional feature on or off. Off: AIfred removes what it saved
/// of the feature, and the next login does not ask for its access. On: the
/// user must connect again when the login does not allow it yet.
pub fn set_feature(dir: &Path, name: &str, on: bool) -> anyhow::Result<GoogleStatus> {
    let Some(f) = FEATURES.iter().find(|f| f.name == name) else {
        bail!("there is no Google feature \"{name}\"");
    };
    if !f.optional {
        bail!("{name} cannot be turned off");
    }
    locked(dir, || {
        // A file that AIfred cannot read stays: a tick of a box must not
        // remove the registration and the login.
        let mut s = load(dir).context("cannot change the Google features")?;
        if on && let Some(n) = f.needs.filter(|n| s.is_off(n)) {
            bail!("{name} needs {n}. Tick \"Use {n}\" first");
        }
        s.off.retain(|o| o != name);
        s.on.retain(|o| o != name);
        match (f.starts_off, on) {
            (true, true) => s.on.push(name.to_string()),
            (false, false) => s.off.push(name.to_string()),
            _ => {}
        }
        save(dir, &s)
    })?;
    if !on {
        (f.forget)(dir)?;
    }
    status(dir)
}

/// A login with no real tokens, for the tests of other modules.
#[cfg(test)]
pub(crate) fn fake_login(dir: &Path, email: &str) {
    save(
        dir,
        &Saved {
            client_id: "123-abc.apps.googleusercontent.com".into(),
            client_secret: "sec".into(),
            refresh_token: Some(format!("refresh-{email}")),
            scopes: vec![CALENDAR.into(), GMAIL.into(), GMAIL_COMPOSE.into()],
            email: Some(email.into()),
            ..Saved::default()
        },
    )
    .unwrap();
}

/// Removes `scope` from the fake login, as a new login without that
/// feature would.
#[cfg(test)]
pub(crate) fn fake_drop_scope(dir: &Path, scope: &str) {
    let mut s = load(dir).unwrap();
    s.scopes.retain(|g| g != scope);
    save(dir, &s).unwrap();
}

/// Gives the fake login an access token that does not end, so a call
/// to a fake Google server needs no token renewal.
#[cfg(test)]
pub(crate) fn fake_access(dir: &Path, token: &str) {
    let mut s = load(dir).unwrap();
    s.access_token = Some(token.into());
    s.expires_at = u64::MAX / 2;
    save(dir, &s).unwrap();
}

/// Saves the user's registration. A new client ID ends the old login: its
/// token belongs to the old registration.
pub fn set_client(
    dir: &Path,
    client_id: &str,
    client_secret: &str,
) -> anyhow::Result<GoogleStatus> {
    let (id, secret) = (client_id.trim(), client_secret.trim());
    if !id.ends_with(".apps.googleusercontent.com") || id.contains(char::is_whitespace) {
        bail!(
            "this does not look like a Google client ID. It ends with .apps.googleusercontent.com"
        );
    }
    if secret.is_empty() || secret.contains(char::is_whitespace) {
        bail!("the client secret is empty. Copy it from the same client in Google Cloud Console");
    }
    let new_client = locked(dir, || {
        // A file that AIfred cannot read is replaced: the user gives a new
        // registration, and this is the way to repair the file.
        let old = load(dir).unwrap_or_default();
        let new_client = old.client_id != id;
        let s = if !new_client {
            Saved {
                client_secret: secret.into(),
                ..old
            }
        } else {
            Saved {
                client_id: id.into(),
                client_secret: secret.into(),
                off: old.off,
                on: old.on,
                ..Saved::default()
            }
        };
        save(dir, &s)?;
        Ok(new_client)
    })?;
    if new_client {
        // The saved events and emails belong to the old login.
        forget_saved(dir)?;
    }
    status(dir)
}

fn random_text(bytes: usize) -> anyhow::Result<String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("no random numbers: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(b))
}

/// The PKCE challenge of a verifier: base64url(SHA-256(verifier)).
fn challenge(verifier: &str) -> String {
    URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()))
}

/// All scopes that a login asks for: not the scopes of the features in `off`.
fn all_scopes(off: &[String]) -> Vec<&'static str> {
    let mut v: Vec<&str> = IDENTITY.to_vec();
    for f in FEATURES.iter().filter(|f| !off.iter().any(|o| o == f.name)) {
        for s in f.scopes {
            if !v.contains(s) {
                v.push(s);
            }
        }
    }
    v
}

/// A login that waits for the browser.
pub struct Login {
    listener: tokio::net::TcpListener,
    /// The page to open in the browser.
    pub url: String,
    redirect: String,
    verifier: String,
    state: String,
    ends: Endpoints,
    /// The registration that the login page uses. The code from Google
    /// works only with this client, also when the user saves another
    /// registration during the wait.
    client_id: String,
    client_secret: String,
    /// Saved::pending of this login.
    id: String,
}

/// Starts a login: a web server on 127.0.0.1 that waits for Google, and
/// the address of Google's login page.
pub async fn begin(dir: &Path, ends: Endpoints) -> anyhow::Result<Login> {
    let id = random_text(16)?;
    let id2 = id.clone();
    // This login is now the only one that can save its tokens.
    let s = files::off_thread(dir, move |dir| {
        locked(dir, || {
            let mut s = load(dir)?;
            if s.client_id.is_empty() || s.client_secret.is_empty() {
                bail!("add the client ID and the client secret of your Google registration first");
            }
            s.pending = Some(id2);
            save(dir, &s)?;
            Ok(s)
        })
    })
    .await?;
    let listener = tokio::net::TcpListener::bind(("127.0.0.1", 0))
        .await
        .context("cannot start the login web server")?;
    let redirect = format!("http://127.0.0.1:{}", listener.local_addr()?.port());
    let verifier = random_text(48)?;
    let state = random_text(16)?;
    let url = reqwest::Url::parse_with_params(
        &ends.auth,
        &[
            ("client_id", s.client_id.as_str()),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", &all_scopes(&s.turned_off()).join(" ")),
            ("code_challenge", &challenge(&verifier)),
            ("code_challenge_method", "S256"),
            ("state", &state),
            // A refresh token, also when the user connected before.
            ("access_type", "offline"),
            ("prompt", "consent"),
        ],
    )?
    .to_string();
    Ok(Login {
        listener,
        url,
        redirect,
        verifier,
        state,
        ends,
        client_id: s.client_id,
        client_secret: s.client_secret,
        id,
    })
}

/// What came back to the login web server.
#[derive(Debug, PartialEq)]
enum Callback {
    Code(String),
    Denied(String),
    /// Another request, for example the browser's icon request.
    Other,
}

/// Reads the first line of an HTTP request: "GET /?code=...&state=... HTTP/1.1".
fn read_callback(request: &str, state: &str) -> anyhow::Result<Callback> {
    let target = request
        .lines()
        .next()
        .and_then(|l| l.strip_prefix("GET "))
        .and_then(|l| l.split(' ').next())
        .unwrap_or("");
    let Ok(url) = reqwest::Url::parse(&format!("http://127.0.0.1{target}")) else {
        return Ok(Callback::Other);
    };
    if url.path() != "/" {
        return Ok(Callback::Other);
    }
    let get = |k: &str| {
        url.query_pairs()
            .find(|(n, _)| n == k)
            .map(|(_, v)| v.into_owned())
    };
    if get("state").as_deref() != Some(state) {
        // Not from this login: another page cannot give us a code.
        return Ok(Callback::Other);
    }
    if let Some(e) = get("error") {
        return Ok(Callback::Denied(e));
    }
    match get("code") {
        Some(c) => Ok(Callback::Code(c)),
        None => Ok(Callback::Other),
    }
}

async fn reply(stream: &mut tokio::net::TcpStream, status: &str, text: &str) {
    let body = format!(
        "<!doctype html><meta charset=utf-8><title>AIfred</title><body style=\"font-family:sans-serif;padding:40px\"><h2>{text}</h2></body>"
    );
    let msg = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(msg.as_bytes()).await;
    let _ = stream.shutdown().await;
}

/// The longest first line of a request that the login web server reads.
const MAX_REQUEST_LINE: usize = 8192;

/// Reads until the first line of the request is complete. The line can
/// come in more than one part. None when the connection sends nothing in
/// 10 seconds, fails, or sends a line that is too long.
async fn read_request_line<R: tokio::io::AsyncRead + Unpin>(stream: &mut R) -> Option<String> {
    let read = async {
        let mut buf = Vec::new();
        let mut chunk = [0u8; 2048];
        loop {
            let n = stream.read(&mut chunk).await.ok()?;
            buf.extend_from_slice(&chunk[..n]);
            if n == 0 || buf.windows(2).any(|w| w == b"\r\n") {
                return Some(String::from_utf8_lossy(&buf).to_string());
            }
            if buf.len() > MAX_REQUEST_LINE {
                return None;
            }
        }
    };
    tokio::time::timeout(Duration::from_secs(10), read)
        .await
        .ok()
        .flatten()
}

/// Answers one connection to the login web server. Some(code), or
/// Some(error) when the user said no. None for any other request, and for a
/// connection that sends nothing in 10 seconds or that fails.
async fn serve(mut stream: tokio::net::TcpStream, state: &str) -> Option<anyhow::Result<String>> {
    let request = read_request_line(&mut stream).await?;
    match read_callback(&request, state) {
        Ok(Callback::Code(code)) => {
            reply(
                &mut stream,
                "200 OK",
                "AIfred is connected to Google. You can close this tab.",
            )
            .await;
            Some(Ok(code))
        }
        Ok(Callback::Denied(e)) => {
            reply(
                &mut stream,
                "200 OK",
                "AIfred did not get access. You can close this tab.",
            )
            .await;
            Some(Err(anyhow::anyhow!("Google did not give access ({e})")))
        }
        Ok(Callback::Other) | Err(_) => {
            reply(&mut stream, "404 Not Found", "Not found").await;
            None
        }
    }
}

/// Waits for Google to send the browser back, then gets the tokens and saves
/// them. `cancel` stops the login at any step before the save, for example
/// when the user starts a new login or disconnects Google.
pub async fn finish(
    dir: &Path,
    login: Login,
    cancel: impl std::future::Future<Output = ()>,
) -> anyhow::Result<GoogleStatus> {
    tokio::select! {
        r = finish_steps(dir, login) => r,
        () = cancel => bail!("the login stopped, because a new login started or Google was disconnected"),
    }
}

async fn finish_steps(dir: &Path, login: Login) -> anyhow::Result<GoogleStatus> {
    // Each connection gets its own task. The browser can open an extra
    // connection that sends nothing, or reset one: that must not block or
    // stop the connection that brings the code.
    let wait = async {
        let (tx, mut rx) = tokio::sync::mpsc::channel::<anyhow::Result<String>>(1);
        loop {
            tokio::select! {
                r = login.listener.accept() => {
                    let (stream, _) = r?;
                    let (tx, state) = (tx.clone(), login.state.clone());
                    tokio::spawn(async move {
                        if let Some(answer) = serve(stream, &state).await {
                            let _ = tx.send(answer).await;
                        }
                    });
                }
                Some(answer) = rx.recv() => return answer,
            }
        }
    };
    let code = tokio::time::timeout(LOGIN_WAIT, wait)
        .await
        .map_err(|_| anyhow::anyhow!("the Google login took more than 5 minutes. Try again"))??;
    let t = token_call(
        &login.ends,
        &[
            ("grant_type", "authorization_code"),
            ("code", &code),
            ("redirect_uri", &login.redirect),
            ("code_verifier", &login.verifier),
            ("client_id", &login.client_id),
            ("client_secret", &login.client_secret),
        ],
    )
    .await?;
    let Some(refresh) = t.refresh_token.clone() else {
        bail!(
            "Google sent no refresh token. Remove AIfred at myaccount.google.com/permissions, then connect again"
        );
    };
    let email = t.id_token.as_deref().and_then(email_of);
    let (client_id, id) = (login.client_id, login.id);
    files::off_thread(dir, move |dir| {
        save_login(dir, &client_id, &id, refresh, t, email)?;
        status(dir)
    })
    .await
}

/// The last step of a login: saves the tokens. Runs off the async threads.
fn save_login(
    dir: &Path,
    client_id: &str,
    id: &str,
    refresh: String,
    t: TokenReply,
    email: Option<String>,
) -> anyhow::Result<()> {
    let (other_account, lost) = locked(dir, || {
        let s = load(dir)?;
        if s.client_id != client_id {
            bail!("the Google registration changed during the login. Click Connect Google again");
        }
        if s.pending.as_deref() != Some(id) {
            bail!("the login stopped, because a new login started or Google was disconnected");
        }
        let other_account = s.email.is_some() && s.email != email;
        let scopes: Vec<String> = t.scope.split_whitespace().map(String::from).collect();
        // The features that this login does not allow, for example Gmail
        // when the user unticked it on Google's page.
        let lost: Vec<&Feature> = FEATURES
            .iter()
            .filter(|f| !f.scopes.iter().all(|x| scopes.iter().any(|g| g == x)))
            .collect();
        save(
            dir,
            &Saved {
                refresh_token: Some(refresh),
                access_token: Some(t.access_token),
                expires_at: files::now() + t.expires_in,
                scopes,
                email,
                pending: None,
                ..s
            },
        )?;
        Ok((other_account, lost))
    })?;
    if other_account {
        // The saved events and emails belong to the old account.
        forget_saved(dir)?;
    } else {
        // Without the access, AIfred cannot update what it saved: remove it.
        lost.iter().try_for_each(|f| (f.forget)(dir))?;
    }
    Ok(())
}

#[derive(Deserialize)]
struct TokenReply {
    access_token: String,
    #[serde(default)]
    expires_in: u64,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    scope: String,
    #[serde(default)]
    id_token: Option<String>,
}

/// Google does not accept the saved login any more: the user removed
/// AIfred's access, or the login expired.
#[derive(Debug)]
struct LoginEnded;

impl std::fmt::Display for LoginEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "Google ended AIfred's access (the login can expire or be removed). Connect Google again in Settings",
        )
    }
}

impl std::error::Error for LoginEnded {}

async fn token_call(ends: &Endpoints, form: &[(&str, &str)]) -> anyhow::Result<TokenReply> {
    let r = reqwest::Client::new()
        .post(&ends.token)
        .form(form)
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("cannot reach Google")?;
    let status = r.status();
    let body = r.text().await?;
    if !status.is_success() {
        let v: serde_json::Value = serde_json::from_str(&body).unwrap_or_default();
        let e = v["error"].as_str().unwrap_or("");
        if e == "invalid_grant" {
            return Err(LoginEnded.into());
        }
        let d = v["error_description"].as_str().unwrap_or(&body);
        bail!("Google refused the login: {e} {d}");
    }
    serde_json::from_str(&body).context("Google's token answer is not valid")
}

/// The email in an id token from Google. The token came straight from
/// Google over HTTPS, so AIfred reads it without a signature check.
fn email_of(id_token: &str) -> Option<String> {
    let payload = id_token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    v["email"].as_str().map(String::from)
}

/// An access token that allows `scope`, renewed when it is old. None when
/// Google is not connected. An error when the login does not allow `scope`.
pub async fn token_for(
    dir: &Path,
    scope: &str,
    ends: &Endpoints,
) -> anyhow::Result<Option<String>> {
    let s = files::off_thread(dir, load).await?;
    let Some(refresh) = s.refresh_token.clone() else {
        return Ok(None);
    };
    if !s.scopes.iter().any(|g| g == scope) {
        let name = FEATURES
            .iter()
            .find(|f| f.scopes.contains(&scope))
            .map_or("this feature", |f| f.name);
        bail!(
            "the Google login does not allow {name}. Connect Google again in Settings and allow it"
        );
    }
    if let Some(t) = &s.access_token
        && s.expires_at > files::now() + RENEW_EARLY_SECS
    {
        return Ok(Some(t.clone()));
    }
    let t = match token_call(
        ends,
        &[
            ("grant_type", "refresh_token"),
            ("refresh_token", &refresh),
            ("client_id", &s.client_id),
            ("client_secret", &s.client_secret),
        ],
    )
    .await
    {
        Ok(t) => t,
        Err(e) if e.is::<LoginEnded>() => {
            files::off_thread(dir, move |dir| drop_login(dir, &refresh)).await?;
            return Err(e);
        }
        Err(e) => return Err(e),
    };
    // Another AIfred program can have changed the file meanwhile: read it
    // again. After a disconnect or a new login, the new token is not saved.
    let token = t.access_token.clone();
    files::off_thread(dir, move |dir| {
        locked(dir, || {
            let mut now = load(dir)?;
            if now.refresh_token.as_deref() == Some(refresh.as_str()) {
                now.access_token = Some(t.access_token);
                now.expires_at = files::now() + t.expires_in;
                save(dir, &now)?;
            }
            Ok(())
        })
    })
    .await?;
    Ok(Some(token))
}

/// Forgets an access token that Google did not accept (HTTP 401), so the
/// next token_for() asks Google for a new one. When Google also refuses
/// the refresh token, token_for() removes the login.
pub async fn reject_access_token(dir: &Path, token: &str) -> anyhow::Result<()> {
    let token = token.to_string();
    files::off_thread(dir, move |dir| {
        locked(dir, || {
            let mut s = load(dir)?;
            if s.access_token.as_deref() == Some(token.as_str()) {
                s.access_token = None;
                s.expires_at = 0;
                save(dir, &s)?;
            }
            Ok(())
        })
    })
    .await
}

/// Deletes a login that Google does not accept any more, so the window
/// shows "Connect Google" again. The registration stays. Another AIfred
/// program can have saved a new login meanwhile: that one stays.
fn drop_login(dir: &Path, refresh: &str) -> anyhow::Result<()> {
    let dropped = locked(dir, || {
        if load(dir)?.refresh_token.as_deref() != Some(refresh) {
            return Ok(false);
        }
        // A new login can wait for the browser: it replaces this one.
        clear_login(dir, true)?;
        Ok(true)
    })?;
    if !dropped {
        return Ok(());
    }
    // The saved events and emails belong to the ended login.
    forget_saved(dir)
}

/// Removes what AIfred saved from Google: the calendar events and the
/// emails. The user's own "waiting for" list stays.
fn forget_saved(dir: &Path) -> anyhow::Result<()> {
    FEATURES.iter().try_for_each(|f| (f.forget)(dir))
}

/// Ends the login: Google forgets the token, and AIfred deletes it. The
/// registration stays, so the user can connect again with one click.
/// The login is deleted first, so a token renewal that ends during the
/// call to Google cannot save the old login again. A file that AIfred
/// cannot read is replaced.
pub async fn disconnect(dir: &Path, ends: &Endpoints) -> anyhow::Result<GoogleStatus> {
    let refresh = files::off_thread(dir, |dir| {
        locked(dir, || {
            let refresh = load(dir).ok().and_then(|s| s.refresh_token);
            // Also stops a login that waits for the browser or for Google.
            clear_login(dir, false)?;
            Ok(refresh)
        })
    })
    .await?;
    if let Some(t) = &refresh {
        // When Google cannot be reached, the token is still gone here.
        let _ = reqwest::Client::new()
            .post(&ends.revoke)
            .form(&[("token", t.as_str())])
            .timeout(Duration::from_secs(10))
            .send()
            .await;
    }
    // The saved events and emails belong to the ended login.
    files::off_thread(dir, |dir| {
        forget_saved(dir)?;
        status(dir)
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-google-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const ID: &str = "123-abc.apps.googleusercontent.com";

    #[test]
    fn pkce_matches_the_example_in_rfc_7636() {
        assert_eq!(
            challenge("dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"),
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM"
        );
        assert_ne!(random_text(48).unwrap(), random_text(48).unwrap());
        assert!((43..=128).contains(&random_text(48).unwrap().len()));
    }

    #[test]
    fn the_callback_needs_the_state_of_this_login() {
        let r = |line: &str| read_callback(&format!("{line}\r\nHost: x\r\n\r\n"), "s1").unwrap();
        assert_eq!(
            r("GET /?state=s1&code=c%2F1 HTTP/1.1"),
            Callback::Code("c/1".into())
        );
        assert_eq!(
            r("GET /?state=s1&error=access_denied HTTP/1.1"),
            Callback::Denied("access_denied".into())
        );
        assert_eq!(r("GET /?state=other&code=c HTTP/1.1"), Callback::Other);
        assert_eq!(r("GET /favicon.ico HTTP/1.1"), Callback::Other);
        assert_eq!(r("OPTIONS * HTTP/1.1"), Callback::Other);
        assert_eq!(r("GET //[bad HTTP/1.1"), Callback::Other, "not a URL");
    }

    #[test]
    fn gmail_can_be_turned_off_and_on() {
        let d = temp("feature");
        fake_login(&d, "me@x.com");
        crate::gmail::tests::save_fake(
            &d,
            "me@x.com",
            vec![crate::gmail::tests::mail("r1", "reply", 100)],
            vec![],
            vec![],
            1000,
        );
        assert!(allows(&d, GMAIL).unwrap());
        assert!(set_feature(&d, "Calendar", false).is_err(), "not optional");
        assert!(set_feature(&d, "Docs", false).is_err());

        // Gmail drafts is off until the user turns it on, also when the
        // login allows it.
        assert!(!allows(&d, GMAIL_COMPOSE).unwrap());
        assert!(!all_scopes(&status(&d).unwrap().off).contains(&GMAIL_COMPOSE));
        let st = set_feature(&d, DRAFTS, true).unwrap();
        assert!(st.off.is_empty(), "{:?}", st.off);
        assert!(allows(&d, GMAIL_COMPOSE).unwrap());
        assert!(all_scopes(&st.off).contains(&GMAIL_COMPOSE));
        let st = set_feature(&d, DRAFTS, false).unwrap();
        assert_eq!(st.off, [DRAFTS]);

        let st = set_feature(&d, "Gmail", false).unwrap();
        assert_eq!(st.off, ["Gmail", DRAFTS]);
        assert_eq!(st.optional, ["Gmail", DRAFTS]);
        assert!(
            !allows(&d, GMAIL).unwrap(),
            "off, also when the login allows it"
        );
        assert!(allows(&d, CALENDAR).unwrap());
        assert!(
            crate::gmail::cached(&d, u64::MAX, 1000).unwrap().is_none(),
            "the saved emails are gone"
        );
        assert!(
            !all_scopes(&st.off).contains(&GMAIL),
            "the login does not ask for it"
        );
        assert!(all_scopes(&st.off).contains(&CALENDAR));
        // Gmail drafts needs Gmail: it cannot go on while Gmail is off.
        let e = set_feature(&d, DRAFTS, true).unwrap_err();
        assert!(e.to_string().contains("needs Gmail"), "{e}");
        assert!(!all_scopes(&status(&d).unwrap().off).contains(&GMAIL_COMPOSE));

        // Gmail off also turns drafts off, and on again gives them back.
        set_feature(&d, "Gmail", true).unwrap();
        set_feature(&d, DRAFTS, true).unwrap();
        let st = set_feature(&d, "Gmail", false).unwrap();
        assert_eq!(st.off, ["Gmail", DRAFTS]);
        assert!(!allows(&d, GMAIL_COMPOSE).unwrap());
        assert!(!all_scopes(&st.off).contains(&GMAIL_COMPOSE));

        // A new registration keeps the choice.
        let st = set_client(&d, "9-z.apps.googleusercontent.com", "s").unwrap();
        assert_eq!(st.off, ["Gmail", DRAFTS]);

        let st = set_feature(&d, "Gmail", true).unwrap();
        assert!(st.off.is_empty(), "{:?}", st.off);
        assert!(all_scopes(&st.off).contains(&GMAIL));
        assert!(all_scopes(&st.off).contains(&GMAIL_COMPOSE));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_registration_is_checked_and_kept_private() {
        let d = temp("client");
        assert!(set_client(&d, "not-an-id", "x").is_err());
        assert!(set_client(&d, ID, " ").is_err());
        let st = set_client(&d, &format!(" {ID} "), "sec").unwrap();
        assert_eq!(
            (st.client_id.as_str(), st.secret_set, st.connected),
            (ID, true, false)
        );
        assert_eq!(
            st.features,
            [
                ("Calendar".to_string(), false),
                ("Gmail".to_string(), false),
                (DRAFTS.to_string(), false)
            ]
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(path(&d)).unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o600, "only the user can read the file");
        }
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_email_comes_from_the_id_token() {
        let payload = URL_SAFE_NO_PAD.encode(br#"{"email":"me@x.com"}"#);
        assert_eq!(
            email_of(&format!("h.{payload}.sig")).as_deref(),
            Some("me@x.com")
        );
        assert_eq!(email_of("bad"), None);
    }

    /// A fake Google token server: it answers each POST with `reply` and
    /// keeps the request bodies.
    async fn fake_google(
        reply: &'static str,
    ) -> (Endpoints, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        fake_google_status("200 OK", reply).await
    }

    async fn fake_google_status(
        status: &'static str,
        reply: &'static str,
    ) -> (Endpoints, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        let l = tokio::net::TcpListener::bind(("127.0.0.1", 0))
            .await
            .unwrap();
        let base = format!("http://127.0.0.1:{}", l.local_addr().unwrap().port());
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let s = seen.clone();
        tokio::spawn(async move {
            loop {
                let (mut c, _) = l.accept().await.unwrap();
                // Read until the headers and the full body are in.
                let mut buf = Vec::new();
                let req = loop {
                    let mut chunk = [0u8; 4096];
                    let n = c.read(&mut chunk).await.unwrap();
                    buf.extend_from_slice(&chunk[..n]);
                    let text = String::from_utf8_lossy(&buf).to_string();
                    if let Some((head, body)) = text.split_once("\r\n\r\n") {
                        let len = head
                            .lines()
                            .find_map(|l| {
                                l.to_ascii_lowercase()
                                    .strip_prefix("content-length:")
                                    .map(|v| v.trim().parse::<usize>().unwrap_or(0))
                            })
                            .unwrap_or(0);
                        if body.len() >= len || n == 0 {
                            break text;
                        }
                    } else if n == 0 {
                        break text;
                    }
                };
                s.lock()
                    .unwrap()
                    .push(req.split("\r\n\r\n").nth(1).unwrap_or("").to_string());
                let msg = format!(
                    "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                c.write_all(msg.as_bytes()).await.unwrap();
            }
        });
        let ends = Endpoints {
            auth: format!("{base}/auth"),
            token: format!("{base}/token"),
            revoke: format!("{base}/revoke"),
        };
        (ends, seen)
    }

    #[tokio::test]
    async fn a_login_saves_the_tokens_and_renews_an_old_one() {
        let d = temp("login");
        set_client(&d, ID, "sec").unwrap();
        let id_token = format!("h.{}.s", URL_SAFE_NO_PAD.encode(br#"{"email":"me@x.com"}"#));
        let reply: &'static str = Box::leak(
            format!(
                r#"{{"access_token":"a1","expires_in":3600,"refresh_token":"r1","scope":"openid {CALENDAR}","id_token":"{id_token}"}}"#
            )
            .into_boxed_str(),
        );
        let (ends, seen) = fake_google(reply).await;
        // Saved Gmail data of an earlier login.
        crate::db::write(&d, |tx| {
            tx.execute(
                "INSERT INTO mail_hidden (thread, last_at, hidden_at) VALUES ('t1', 1, 1)",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        let login = begin(&d, ends.clone()).await.unwrap();
        let url = reqwest::Url::parse(&login.url).unwrap();
        let q = |k: &str| {
            url.query_pairs()
                .find(|(n, _)| n == k)
                .unwrap()
                .1
                .into_owned()
        };
        assert_eq!(q("code_challenge_method"), "S256");
        assert!(q("scope").contains(CALENDAR) && q("scope").contains(GMAIL));
        let (redirect, state) = (q("redirect_uri"), q("state"));
        // The browser opens an extra connection that sends nothing, and
        // one that closes at once. They must not delay or stop the login.
        let host = redirect.trim_start_matches("http://").to_string();
        let _idle = tokio::net::TcpStream::connect(&host).await.unwrap();
        drop(tokio::net::TcpStream::connect(&host).await.unwrap());
        // The browser comes back with the code.
        let browser = tokio::spawn(async move {
            reqwest::get(format!("{redirect}/?state={state}&code=c1"))
                .await
                .unwrap()
                .text()
                .await
                .unwrap()
        });
        let st = tokio::time::timeout(
            Duration::from_secs(5),
            finish(&d, login, std::future::pending()),
        )
        .await
        .expect("the idle connection must not make the login wait")
        .unwrap();
        assert!(browser.await.unwrap().contains("connected"));
        assert_eq!(
            (st.connected, st.email.as_deref()),
            (true, Some("me@x.com"))
        );
        // Google gave only the calendar: the user unticked Gmail.
        assert_eq!(
            st.features,
            [
                ("Calendar".to_string(), true),
                ("Gmail".to_string(), false),
                (DRAFTS.to_string(), false)
            ]
        );
        assert!(allows(&d, CALENDAR).unwrap() && !allows(&d, GMAIL).unwrap());
        // Without the Gmail access, the saved Gmail data is gone.
        let hidden: i64 = crate::db::read(&d, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM mail_hidden", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(hidden, 0);
        let sent = seen.lock().unwrap()[0].clone();
        assert!(
            sent.contains("code=c1") && sent.contains("code_verifier="),
            "{sent}"
        );

        // A fresh token needs no call. An old one is renewed.
        assert_eq!(
            token_for(&d, CALENDAR, &ends).await.unwrap().as_deref(),
            Some("a1")
        );
        assert_eq!(seen.lock().unwrap().len(), 1);
        let mut s = load(&d).unwrap();
        s.expires_at = 0;
        save(&d, &s).unwrap();
        token_for(&d, CALENDAR, &ends).await.unwrap();
        assert!(seen.lock().unwrap()[1].contains("grant_type=refresh_token"));
        assert!(
            token_for(&d, "https://www.googleapis.com/auth/documents", &ends)
                .await
                .is_err(),
            "not allowed"
        );

        let st = disconnect(&d, &ends).await.unwrap();
        assert_eq!(
            (st.connected, st.client_id.as_str()),
            (false, ID),
            "the registration stays"
        );
        assert_eq!(token_for(&d, CALENDAR, &ends).await.unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[tokio::test]
    async fn a_login_that_google_ended_is_deleted() {
        let d = temp("ended");
        set_client(&d, ID, "sec").unwrap();
        save(
            &d,
            &Saved {
                refresh_token: Some("r1".into()),
                scopes: vec![CALENDAR.into()],
                email: Some("me@x.com".into()),
                ..load(&d).unwrap()
            },
        )
        .unwrap();
        let account = account(&d).unwrap().unwrap();
        let d2 = crate::calendar::Downloaded {
            account,
            events: vec![],
            cut: false,
        };
        assert!(crate::calendar::save(&d, d2, 1).unwrap().is_some());
        assert!(status(&d).unwrap().connected);
        let (ends, _) = fake_google_status(
            "400 Bad Request",
            r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
        )
        .await;
        let e = token_for(&d, CALENDAR, &ends).await.unwrap_err();
        assert!(e.to_string().contains("Connect Google again"), "{e}");
        let st = status(&d).unwrap();
        assert_eq!(
            (st.connected, st.client_id.as_str()),
            (false, ID),
            "the login goes, the registration stays"
        );
        assert_eq!(
            crate::calendar::cached(&d, u64::MAX, 0).unwrap(),
            None,
            "the old events go too"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_client_forgets_the_saved_events() {
        let d = temp("newclient");
        fake_login(&d, "me@x.com");
        let account = account(&d).unwrap().unwrap();
        let d2 = crate::calendar::Downloaded {
            account,
            events: vec![],
            cut: false,
        };
        crate::calendar::save(&d, d2, 1).unwrap();
        set_client(&d, ID, "sec2").unwrap();
        assert!(
            crate::calendar::cached(&d, u64::MAX, 0).unwrap().is_some(),
            "the same client keeps them"
        );
        set_client(&d, "456-def.apps.googleusercontent.com", "sec").unwrap();
        assert_eq!(crate::calendar::cached(&d, u64::MAX, 0).unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[tokio::test]
    async fn a_download_that_ends_after_disconnect_is_not_shown() {
        let d = temp("disconnect");
        fake_login(&d, "me@x.com");
        let account = account(&d).unwrap().unwrap();
        let old = crate::calendar::Downloaded {
            account,
            events: vec![],
            cut: false,
        };
        crate::calendar::save(&d, old.clone(), 1).unwrap();
        let ends = Endpoints {
            revoke: "http://127.0.0.1:9/revoke".into(),
            ..Endpoints::default()
        };
        disconnect(&d, &ends).await.unwrap();
        assert_eq!(crate::calendar::cached(&d, u64::MAX, 0).unwrap(), None);
        // The download started before the disconnect and ends after it.
        assert_eq!(crate::calendar::save(&d, old.clone(), 2).unwrap(), None);
        assert_eq!(crate::calendar::cached(&d, u64::MAX, 0).unwrap(), None);
        // Another account connects: the old account's list is not shown.
        fake_login(&d, "other@x.com");
        assert_eq!(crate::calendar::save(&d, old, 3).unwrap(), None);
        assert_eq!(crate::calendar::cached(&d, u64::MAX, 0).unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_registration_repairs_a_broken_file() {
        let d = temp("broken");
        std::fs::write(path(&d), "not json").unwrap();
        assert!(status(&d).is_err());
        let st = set_client(&d, ID, "sec").unwrap();
        assert_eq!((st.client_id.as_str(), st.connected), (ID, false));
        // An old file without the secret is not broken.
        std::fs::write(path(&d), format!(r#"{{"client_id":"{ID}"}}"#)).unwrap();
        assert!(!status(&d).unwrap().secret_set);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn saves_at_the_same_time_do_not_fail() {
        let d = temp("together");
        fake_login(&d, "me@x.com");
        let handles: Vec<_> = (0..8)
            .map(|i| {
                let d = d.clone();
                std::thread::spawn(move || {
                    locked(&d, || {
                        let mut s = load(&d)?;
                        s.access_token = Some(format!("a{i}"));
                        save(&d, &s)
                    })
                })
            })
            .collect();
        for h in handles {
            h.join().unwrap().unwrap();
        }
        assert!(load(&d).unwrap().access_token.is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[tokio::test]
    async fn a_login_uses_the_registration_of_its_start() {
        let d = temp("swap");
        set_client(&d, ID, "sec").unwrap();
        let (ends, seen) = fake_google(
            r#"{"access_token":"a1","expires_in":3600,"refresh_token":"r1","scope":"openid"}"#,
        )
        .await;
        let login = begin(&d, ends).await.unwrap();
        let url = reqwest::Url::parse(&login.url).unwrap();
        let q = |k: &str| {
            url.query_pairs()
                .find(|(n, _)| n == k)
                .unwrap()
                .1
                .into_owned()
        };
        let (redirect, state) = (q("redirect_uri"), q("state"));
        // The user saves another registration while the login waits.
        set_client(&d, "456-def.apps.googleusercontent.com", "sec2").unwrap();
        tokio::spawn(async move {
            let _ = reqwest::get(format!("{redirect}/?state={state}&code=c1")).await;
        });
        let e = finish(&d, login, std::future::pending()).await.unwrap_err();
        assert!(e.to_string().contains("registration changed"), "{e}");
        let sent = seen.lock().unwrap()[0].clone();
        assert!(sent.contains(&format!("client_id={ID}")), "{sent}");
        assert!(!status(&d).unwrap().connected, "no login of the old client");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[tokio::test]
    async fn a_rejected_access_token_is_renewed_next_time() {
        let d = temp("reject");
        fake_login(&d, "me@x.com");
        let mut s = load(&d).unwrap();
        s.access_token = Some("a1".into());
        s.expires_at = files::now() + 3600;
        save(&d, &s).unwrap();
        reject_access_token(&d, "other").await.unwrap();
        assert_eq!(load(&d).unwrap().access_token.as_deref(), Some("a1"));
        reject_access_token(&d, "a1").await.unwrap();
        let s = load(&d).unwrap();
        assert_eq!((s.access_token, s.expires_at), (None, 0));
        assert!(s.refresh_token.is_some(), "the login stays");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[tokio::test]
    async fn a_request_line_in_two_parts_is_read_in_full() {
        let (mut client, mut server) = tokio::io::duplex(64);
        let reader = tokio::spawn(async move { read_request_line(&mut server).await });
        client.write_all(b"GET /?state=s1&co").await.unwrap();
        client.flush().await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        client
            .write_all(b"de=c1 HTTP/1.1\r\nHost: x\r\n\r\n")
            .await
            .unwrap();
        let line = reader.await.unwrap().unwrap();
        assert_eq!(
            read_callback(&line, "s1").unwrap(),
            Callback::Code("c1".into())
        );
    }

    fn token_reply() -> TokenReply {
        TokenReply {
            access_token: "a1".into(),
            expires_in: 3600,
            refresh_token: Some("r1".into()),
            scope: CALENDAR.into(),
            id_token: None,
        }
    }

    #[tokio::test]
    async fn a_login_that_ends_after_disconnect_or_a_new_login_is_not_saved() {
        let d = temp("late");
        set_client(&d, ID, "sec").unwrap();
        let ends = Endpoints {
            revoke: "http://127.0.0.1:9/revoke".into(),
            ..Endpoints::default()
        };
        // The user clicks Disconnect while Google sends the tokens.
        let login = begin(&d, ends.clone()).await.unwrap();
        disconnect(&d, &ends).await.unwrap();
        let e = save_login(&d, ID, &login.id, "r1".into(), token_reply(), None).unwrap_err();
        assert!(e.to_string().contains("login stopped"), "{e}");
        assert!(!status(&d).unwrap().connected);
        // A second login replaces the first one.
        let first = begin(&d, ends.clone()).await.unwrap();
        let second = begin(&d, ends.clone()).await.unwrap();
        assert!(save_login(&d, ID, &first.id, "r1".into(), token_reply(), None).is_err());
        save_login(&d, ID, &second.id, "r2".into(), token_reply(), None).unwrap();
        assert!(status(&d).unwrap().connected);
        assert_eq!(load(&d).unwrap().pending, None, "the id is used once");
        assert!(save_login(&d, ID, &second.id, "r3".into(), token_reply(), None).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[tokio::test]
    async fn a_new_login_stops_the_old_wait() {
        let d = temp("cancel");
        set_client(&d, ID, "sec").unwrap();
        let login = begin(&d, Endpoints::default()).await.unwrap();
        let e = finish(&d, login, async {}).await.unwrap_err();
        assert!(e.to_string().contains("new login"), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
