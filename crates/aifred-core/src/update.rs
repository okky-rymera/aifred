//! Updates of the AIfred program file from the GitHub releases.
//!
//! 1. `check` asks GitHub for the releases and finds the newest one that is
//!    newer than this program and has a program file for this computer.
//! 2. `download` saves that file next to the program under a temporary name.
//!    It compares the SHA-256 sum with the sum that GitHub gives, and it runs
//!    the new file once (`<file> version`) to see that it works on this
//!    computer. Only then does it rename the file to `.aifred-update`. So
//!    `.aifred-update` is always a complete, checked file.
//! 3. `apply` puts the new file in place of the program.
//!
//! Only one window at a time can update the program: it holds `UpdateLock`.
//!
//! The window applies the update only when it restarts or quits. Before
//! that, the program file stays the old one. This is important: Claude Code
//! starts the AI tool servers from the program file for each message, and a
//! new tool server must not run next to an old window.

use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use anyhow::{Context, bail};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// The GitHub releases of AIfred. All releases, also pre-releases: the
/// `/releases/latest` address skips pre-releases.
pub const RELEASES: &str = "https://api.github.com/repos/okky-rymera/aifred/releases?per_page=20";

/// The name of the downloaded and checked file, next to the program file.
const STAGED: &str = ".aifred-update";
/// A download that is not complete or not checked has this name, plus the
/// process number and this end. `discard` removes old ones.
const PART_END: &str = ".part";
/// The lock file of the window that updates the program.
const LOCK: &str = ".aifred-update.lock";

/// The download stops when no data comes for this time.
const READ_TIMEOUT: Duration = Duration::from_secs(60);
/// The download stops after this time, also when data still comes.
const DOWNLOAD_TIMEOUT: Duration = Duration::from_secs(30 * 60);

/// A release that can replace this program.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Release {
    /// For example "0.1.2", without the "v".
    pub version: String,
    /// The program file for this computer.
    pub url: String,
    /// The SHA-256 sum that GitHub gives for the file, in hex.
    pub sha256: String,
    /// The release page, with the list of changes.
    pub page: String,
}

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    #[serde(default)]
    draft: bool,
    html_url: String,
    #[serde(default)]
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    /// "sha256:<hex>". GitHub adds it to each uploaded file.
    digest: Option<String>,
}

/// The name of the release file for this computer, for example
/// "aifred-0.1.2-linux-x86_64".
pub fn asset_name(version: &str) -> String {
    format!("aifred-{version}-{}-{}", std::env::consts::OS, std::env::consts::ARCH)
}

/// The newest release in a GitHub release list (`json`) that is newer than
/// `current`. A release without a file for this computer, or without a
/// SHA-256 sum, does not count.
pub fn newest(json: &[u8], current: &str) -> anyhow::Result<Option<Release>> {
    let current = semver::Version::parse(current).with_context(|| format!("bad program version {current:?}"))?;
    let releases: Vec<ApiRelease> = serde_json::from_slice(json).context("GitHub sent a release list that AIfred cannot read")?;
    let mut best: Option<(semver::Version, Release)> = None;
    for r in releases.into_iter().filter(|r| !r.draft) {
        let Ok(v) = semver::Version::parse(r.tag_name.trim_start_matches('v')) else { continue };
        if v <= current || best.as_ref().is_some_and(|(b, _)| v <= *b) {
            continue;
        }
        let name = asset_name(&v.to_string());
        let Some(a) = r.assets.into_iter().find(|a| a.name == name) else { continue };
        let Some(sha) = a.digest.as_deref().and_then(|d| d.strip_prefix("sha256:")) else { continue };
        let release = Release { version: v.to_string(), url: a.browser_download_url, sha256: sha.to_ascii_lowercase(), page: r.html_url };
        best = Some((v, release));
    }
    Ok(best.map(|(_, r)| r))
}

/// One web client for all checks and downloads.
fn http() -> anyhow::Result<&'static reqwest::Client> {
    static CLIENT: OnceLock<reqwest::Client> = OnceLock::new();
    if let Some(c) = CLIENT.get() {
        return Ok(c);
    }
    let c = reqwest::Client::builder()
        .user_agent("aifred-updater")
        .connect_timeout(Duration::from_secs(15))
        .read_timeout(READ_TIMEOUT)
        .build()?;
    Ok(CLIENT.get_or_init(|| c))
}

/// Only the window that holds this lock checks, downloads, and applies
/// updates. The lock ends when the process stops, also after a crash.
#[derive(Debug)]
pub struct UpdateLock {
    _file: File,
}

impl UpdateLock {
    /// Takes the lock next to `program`. Returns None when another window
    /// has it. With `wait`, tries again for that time first: a restart
    /// starts the new window before the old one stops.
    pub fn take(program: &Path, wait: Option<Duration>) -> anyhow::Result<Option<UpdateLock>> {
        let path = program.with_file_name(LOCK);
        let file = File::options()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("AIfred cannot write to {}, so it cannot update itself", path.display()))?;
        let until = Instant::now() + wait.unwrap_or_default();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Some(UpdateLock { _file: file })),
                Err(std::fs::TryLockError::WouldBlock) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(200));
                }
                Err(std::fs::TryLockError::WouldBlock) => return Ok(None),
                Err(std::fs::TryLockError::Error(e)) => return Err(e).context("cannot lock the update file"),
            }
        }
    }
}

/// Asks GitHub for a release that is newer than `current`.
pub async fn check(current: &str) -> anyhow::Result<Option<Release>> {
    check_at(RELEASES, current).await
}

/// `check` with another address. For tests.
pub async fn check_at(url: &str, current: &str) -> anyhow::Result<Option<Release>> {
    let reply = http()?
        .get(url)
        .header("Accept", "application/vnd.github+json")
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .context("cannot reach GitHub")?
        .error_for_status()
        .context("GitHub refused the release list")?;
    newest(&reply.bytes().await?, current)
}

/// Where `download` saves the new program file: next to `program`, so that
/// `apply` can replace the program in one step.
pub fn staged_path(program: &Path) -> PathBuf {
    program.with_file_name(STAGED)
}

/// True when a checked new program file waits for `apply`.
pub fn is_staged(program: &Path) -> bool {
    staged_path(program).is_file()
}

/// Removes the downloaded file and all incomplete downloads. Call it only
/// with the `UpdateLock`, so it does not remove the files of another window.
pub fn discard(program: &Path) {
    let _ = std::fs::remove_file(staged_path(program));
    let Some(folder) = program.parent() else { return };
    let Ok(entries) = std::fs::read_dir(folder) else { return };
    for e in entries.flatten() {
        let name = e.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(STAGED) && name.ends_with(PART_END) {
            let _ = std::fs::remove_file(e.path());
        }
    }
}

/// Downloads the release file next to `program` and checks it. Only a
/// complete, checked file replaces the waiting file, in one rename. On an
/// error, the waiting file from before stays. The program file does not change.
pub async fn download(release: &Release, program: &Path) -> anyhow::Result<PathBuf> {
    let staged = staged_path(program);
    let part = program.with_file_name(format!("{STAGED}.{}{PART_END}", std::process::id()));
    let result = save_and_check(release, &part).await;
    let result = result.and_then(|()| std::fs::rename(&part, &staged).context("cannot keep the downloaded file"));
    if result.is_err() {
        let _ = std::fs::remove_file(&part);
    }
    result.map(|()| staged)
}

async fn save_and_check(release: &Release, part: &Path) -> anyhow::Result<()> {
    tokio::time::timeout(DOWNLOAD_TIMEOUT, save(release, part))
        .await
        .with_context(|| format!("the download did not end in {} minutes", DOWNLOAD_TIMEOUT.as_secs() / 60))??;
    check_file(release, part).await
}

async fn save(release: &Release, part: &Path) -> anyhow::Result<()> {
    let folder = part.parent().unwrap_or(Path::new("."));
    let mut file = File::create(part)
        .with_context(|| format!("AIfred cannot write to {}, so it cannot update itself", folder.display()))?;
    let mut reply = http()?.get(&release.url).send().await.context("cannot download the update")?.error_for_status()?;
    let mut sum = Sha256::new();
    while let Some(chunk) = reply.chunk().await.context("the download stopped")? {
        sum.update(&chunk);
        file.write_all(&chunk)?;
    }
    file.sync_all()?;
    drop(file);
    let got: String = sum.finalize().iter().map(|b| format!("{b:02x}")).collect();
    if got != release.sha256 {
        bail!("the downloaded file is damaged (its SHA-256 sum is not the sum from GitHub)");
    }
    Ok(())
}

async fn check_file(release: &Release, part: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(part, std::fs::Permissions::from_mode(0o755))?;
    }
    // The new file must start on this computer. For example, a release can
    // need a newer system library.
    let run = tokio::process::Command::new(part).arg("version").kill_on_drop(true).output();
    let out = tokio::time::timeout(Duration::from_secs(30), run)
        .await
        .context("the new program did not answer in 30 seconds")?
        .context("the new program cannot start on this computer")?;
    let said = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if !out.status.success() || said != release.version {
        let err = String::from_utf8_lossy(&out.stderr);
        bail!("the new program does not work on this computer: {}", err.trim());
    }
    Ok(())
}

/// Puts the downloaded file in place of `program`. Returns false when no
/// file waits. A running program keeps its old file until it stops.
pub fn apply(program: &Path) -> anyhow::Result<bool> {
    let staged = staged_path(program);
    if !staged.is_file() {
        return Ok(false);
    }
    std::fs::rename(&staged, program).with_context(|| format!("cannot replace {}", program.display()))?;
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn release(tag: &str, file_version: &str, digest: Option<&str>) -> serde_json::Value {
        json!({
            "tag_name": tag,
            "draft": false,
            "html_url": format!("https://github.com/okky-rymera/aifred/releases/tag/{tag}"),
            "assets": [{
                "name": asset_name(file_version),
                "browser_download_url": format!("https://example.com/{file_version}"),
                "digest": digest,
            }]
        })
    }

    fn newest_of(list: serde_json::Value, current: &str) -> Option<String> {
        newest(list.to_string().as_bytes(), current).unwrap().map(|r| r.version)
    }

    #[test]
    fn the_newest_newer_release_with_a_file_for_this_computer_wins() {
        let list = json!([
            release("v0.1.1", "0.1.1", Some("sha256:AA")),
            release("v0.3.0", "0.3.0", Some("sha256:bb")),
            release("v0.2.0", "0.2.0", Some("sha256:cc")),
        ]);
        let r = newest(list.to_string().as_bytes(), "0.1.1").unwrap().unwrap();
        assert_eq!(r.version, "0.3.0");
        assert_eq!(r.sha256, "bb");
        assert_eq!(r.url, "https://example.com/0.3.0");
        assert!(r.page.ends_with("/v0.3.0"));
    }

    #[test]
    fn no_update_when_this_program_is_the_newest() {
        let list = json!([release("v0.1.1", "0.1.1", Some("sha256:aa")), release("v0.1.0", "0.1.0", Some("sha256:aa"))]);
        assert_eq!(newest_of(list, "0.1.1"), None);
        assert_eq!(newest_of(json!([]), "0.1.1"), None);
    }

    #[test]
    fn releases_that_cannot_be_installed_safely_are_skipped() {
        let mut draft = release("v0.5.0", "0.5.0", Some("sha256:aa"));
        draft["draft"] = json!(true);
        let list = json!([
            draft,
            release("v0.4.0", "0.4.0", None),               // no SHA-256 sum
            release("v0.3.0", "9.9.9", Some("sha256:aa")),  // no file for this version
            release("nightly", "nightly", Some("sha256:aa")), // not a version
            release("v0.2.0", "0.2.0", Some("sha256:aa")),
        ]);
        assert_eq!(newest_of(list, "0.1.1"), Some("0.2.0".into()));
    }

    #[test]
    fn a_pre_release_version_is_older_than_its_release() {
        let list = json!([release("v0.2.0-beta.1", "0.2.0-beta.1", Some("sha256:aa"))]);
        assert_eq!(newest_of(list.clone(), "0.1.1"), Some("0.2.0-beta.1".into()));
        assert_eq!(newest_of(list, "0.2.0"), None);
    }

    fn folder(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-update-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// A one-file web server: each request gets `body`.
    async fn serve(body: Vec<u8>) -> String {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else { return };
                let mut head = Vec::new();
                let mut b = [0u8; 1024];
                while !head.windows(4).any(|w| w == b"\r\n\r\n") {
                    match sock.read(&mut b).await {
                        Ok(0) | Err(_) => break,
                        Ok(n) => head.extend_from_slice(&b[..n]),
                    }
                }
                let top = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
                let _ = sock.write_all(top.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            }
        });
        format!("http://{addr}/file")
    }

    fn sha(bytes: &[u8]) -> String {
        Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
    }

    /// A fake new program: a shell script that answers `version`.
    fn fake_program(version: &str) -> Vec<u8> {
        format!("#!/bin/sh\n[ \"$1\" = version ] && echo {version}\n").into_bytes()
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_checked_download_waits_until_apply_replaces_the_program() {
        let d = folder("ok");
        let program = d.join("aifred");
        std::fs::write(&program, "old").unwrap();
        let body = fake_program("0.2.0");
        let r = Release { version: "0.2.0".into(), url: serve(body.clone()).await, sha256: sha(&body), page: String::new() };

        let staged = download(&r, &program).await.unwrap();
        assert_eq!(staged, d.join(".aifred-update"));
        assert!(is_staged(&program));
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "old", "the download does not change the program");

        assert!(apply(&program).unwrap());
        assert_eq!(std::fs::read(&program).unwrap(), body);
        assert!(!is_staged(&program));
        assert!(!apply(&program).unwrap(), "nothing waits now");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_damaged_download_is_removed() {
        let d = folder("bad-sum");
        let program = d.join("aifred");
        std::fs::write(&program, "old").unwrap();
        let body = fake_program("0.2.0");
        let r = Release { version: "0.2.0".into(), url: serve(body).await, sha256: sha(b"other"), page: String::new() };
        let e = download(&r, &program).await.unwrap_err();
        assert!(format!("{e:#}").contains("damaged"), "got {e:#}");
        assert!(!is_staged(&program));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_program_that_does_not_start_is_removed() {
        let d = folder("bad-run");
        let program = d.join("aifred");
        std::fs::write(&program, "old").unwrap();
        // It says another version, like a file that does not belong to the release.
        let body = fake_program("0.1.0");
        let r = Release { version: "0.2.0".into(), url: serve(body.clone()).await, sha256: sha(&body), page: String::new() };
        let e = download(&r, &program).await.unwrap_err();
        assert!(format!("{e:#}").contains("does not work"), "got {e:#}");
        assert!(!is_staged(&program));
        assert_eq!(std::fs::read_to_string(&program).unwrap(), "old");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn a_failed_download_keeps_the_checked_file_from_before() {
        let d = folder("keep");
        let program = d.join("aifred");
        std::fs::write(&program, "old").unwrap();
        let good = fake_program("0.2.0");
        let r = Release { version: "0.2.0".into(), url: serve(good.clone()).await, sha256: sha(&good), page: String::new() };
        download(&r, &program).await.unwrap();

        let newer = fake_program("0.3.0");
        let r = Release { version: "0.3.0".into(), url: serve(newer).await, sha256: sha(b"other"), page: String::new() };
        assert!(download(&r, &program).await.is_err());
        assert_eq!(std::fs::read(staged_path(&program)).unwrap(), good, "the 0.2.0 file stays");
        let names: Vec<String> = std::fs::read_dir(&d).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into()).collect();
        assert!(!names.iter().any(|n| n.ends_with(PART_END)), "no incomplete file stays: {names:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn discard_removes_the_waiting_file_and_incomplete_downloads() {
        let d = folder("discard");
        let program = d.join("aifred");
        std::fs::write(&program, "old").unwrap();
        std::fs::write(staged_path(&program), "new").unwrap();
        std::fs::write(d.join(".aifred-update.123.part"), "half").unwrap();
        std::fs::write(d.join("notes.part"), "other").unwrap();
        discard(&program);
        assert!(!is_staged(&program));
        assert!(!d.join(".aifred-update.123.part").exists());
        assert!(d.join("notes.part").exists(), "other files stay");
        assert!(program.exists());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_one_window_gets_the_update_lock() {
        let d = folder("lock");
        let program = d.join("aifred");
        let first = UpdateLock::take(&program, None).unwrap();
        assert!(first.is_some());
        assert!(UpdateLock::take(&program, Some(Duration::from_millis(300))).unwrap().is_none());
        drop(first);
        assert!(UpdateLock::take(&program, None).unwrap().is_some(), "the lock is free after the first window stops");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
