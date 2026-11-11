//! Meaning search: a small model on the computer turns each text into 384
//! numbers (a vector). Texts with a near meaning get near vectors, so
//! "money" can find "invoice" with no shared word.
//!
//! - Only the helper loads the model (the aifred-embed crate). It is the
//!   AIfred program in its helper mode, in its own process.
//!   This file sends texts to it over <data folder>/embed.sock. It starts
//!   the helper when the socket does not answer. The helper stops after 5
//!   minutes with no requests, so an idle app keeps no model in memory.
//! - The model runs on the processor and uses RAM, not the GPU. The first use
//!   downloads it into <data folder>/models (about 90 MB).
//! - Vectors are derived data: fill_missing() can make all of them again
//!   from the text. A search never makes vectors for the saved texts. It
//!   only turns the question into a vector.
//! - When a text changes, a trigger deletes its vector (db.rs, schema 3).
//!   The next fill_missing() makes a new one.
//! - If the model cannot load, meaning search is off and word search still works.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use crate::db;

/// The model name saved in the meta table. If it changes, the vectors are
/// made again. It must be the same as aifred_embed::MODEL (a test checks it).
pub const MODEL: &str = "Qdrant/all-MiniLM-L6-v2-onnx";
pub const DIMS: usize = 384;
/// Texts in one step of fill_missing: one helper request and one transaction.
const BATCH: i64 = 64;
/// The file names in the data folder, the same as in aifred_embed.
const SOCKET: &str = "embed.sock";
/// How long a new helper can take to open its socket.
const START_WAIT: Duration = Duration::from_secs(10);
/// How long an answer can take for a fill. The first start downloads the model.
const ANSWER_WAIT: Duration = Duration::from_secs(600);
/// How long an answer can take for a search. A search runs while the user or
/// the chat waits. A model that is on the disk loads in 1 or 2 seconds. When
/// the helper still downloads the model, the search goes on with word search
/// only, and the download goes on in the helper.
const QUERY_WAIT: Duration = Duration::from_secs(15);
/// After a failure, the next try waits this long. A try can download the
/// model, so a try on each search would make each search slow.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// Where the model files are. The tests share one folder, so the model
/// downloads only once for all tests.
pub fn models_dir(dir: &Path) -> PathBuf {
    if cfg!(test) {
        return PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/test-models"
        ));
    }
    dir.join("models")
}

/// The last failure, and when it was. One for each program.
static FAILED: Mutex<Option<(Instant, String)>> = Mutex::new(None);
/// When a search last waited QUERY_WAIT with no answer. Until RETRY_AFTER
/// later, searches do not wait for the helper again. This is not a failure:
/// a fill still waits, and the status stays the same.
static SLOW: Mutex<Option<Instant>> = Mutex::new(None);

/// The vector of one search text, for meaning search. It waits at most
/// QUERY_WAIT. The error says why meaning search is not used for this search.
pub fn embed_query(dir: &Path, text: &str) -> Result<Vec<f32>, ModelError> {
    if let Some(at) = *SLOW.lock().unwrap_or_else(|e| e.into_inner())
        && at.elapsed() < RETRY_AFTER
    {
        return Err(ModelError("the model is still loading".into()));
    }
    match embed_with(dir, &[text.to_string()], QUERY_WAIT) {
        Ok(mut v) => Ok(v.remove(0)),
        Err(HelperError::Slow) => {
            *SLOW.lock().unwrap_or_else(|e| e.into_inner()) = Some(Instant::now());
            Err(ModelError(format!(
                "the model did not answer in {} seconds, so this search uses word search only",
                QUERY_WAIT.as_secs()
            )))
        }
        Err(HelperError::Failed(e)) => Err(e),
    }
}

/// One vector for each text, from the helper. It starts the helper when it
/// does not run. The error says why meaning search is off. After a failure
/// (for example no network for the first download), the next try is after
/// RETRY_AFTER, so the app does not need a restart.
pub fn embed_texts(dir: &Path, texts: &[String]) -> Result<Vec<Vec<f32>>, ModelError> {
    embed_with(dir, texts, ANSWER_WAIT).map_err(|e| match e {
        HelperError::Failed(e) => e,
        HelperError::Slow => ModelError(format!(
            "the helper did not answer in {} seconds",
            ANSWER_WAIT.as_secs()
        )),
    })
}

/// Why the helper gave no vectors.
enum HelperError {
    /// No answer in the wait time. The helper can still be busy (for example
    /// with the download), so this is not a failure.
    Slow,
    Failed(ModelError),
}

fn embed_with(dir: &Path, texts: &[String], wait: Duration) -> Result<Vec<Vec<f32>>, HelperError> {
    if let Some((at, e)) = FAILED.lock().unwrap_or_else(|e| e.into_inner()).as_ref()
        && at.elapsed() < RETRY_AFTER
    {
        return Err(HelperError::Failed(ModelError(e.clone())));
    }
    let res = ask_helper(dir, texts, wait);
    let mut failed = FAILED.lock().unwrap_or_else(|e| e.into_inner());
    match res {
        Ok(v) => {
            *failed = None;
            *SLOW.lock().unwrap_or_else(|e| e.into_inner()) = None;
            Ok(v)
        }
        Err(None) => Err(HelperError::Slow),
        Err(Some(e)) => {
            *failed = Some((Instant::now(), e.clone()));
            Err(HelperError::Failed(ModelError(e)))
        }
    }
}

/// The socket path is at most about 108 bytes on Linux. A data folder with a
/// long path gets its socket in the private runtime folder of the user
/// ($XDG_RUNTIME_DIR), with a name from the data folder path.
const MAX_SOCKET_PATH: usize = 100;

fn socket_path(dir: &Path) -> Result<PathBuf, String> {
    let p = dir.join(SOCKET);
    if p.as_os_str().len() <= MAX_SOCKET_PATH {
        return Ok(p);
    }
    // Not /tmp: other users can open files there and read the texts.
    let run = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .ok_or("the data folder path is too long for the helper socket, and XDG_RUNTIME_DIR is not set")?;
    // FNV-1a: the same data folder always gets the same name.
    let hash = dir
        .as_os_str()
        .as_encoded_bytes()
        .iter()
        .fold(0xcbf2_9ce4_8422_2325_u64, |h, b| {
            (h ^ u64::from(*b)).wrapping_mul(0x100_0000_01b3)
        });
    Ok(run.join(format!("aifred-embed-{hash:016x}.sock")))
}

/// The vectors from the helper. Err(None): no answer in `wait`.
fn ask_helper(
    dir: &Path,
    texts: &[String],
    wait: Duration,
) -> Result<Vec<Vec<f32>>, Option<String>> {
    let request = serde_json::to_string(&serde_json::json!({ "texts": texts }))
        .map_err(|e| Some(e.to_string()))?;
    let socket = socket_path(dir).map_err(Some)?;
    // Two tries: a helper can stop for idle just when this connects.
    let mut last_error = String::new();
    for _ in 0..2 {
        let stream = match UnixStream::connect(&socket) {
            Ok(s) => s,
            Err(_) => {
                let helper = start_helper(dir, &socket).map_err(Some)?;
                connect_new(dir, &socket, helper).map_err(Some)?
            }
        };
        match send(stream, &request, wait) {
            Ok(line) => return read_answer(&line, texts.len()).map_err(Some),
            // The read wait ended. A second try would wait again.
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                return Err(None);
            }
            Err(e) => last_error = format!("the meaning search helper stopped: {e}"),
        }
    }
    Err(Some(last_error))
}

fn send(stream: UnixStream, request: &str, wait: Duration) -> std::io::Result<String> {
    stream.set_read_timeout(Some(wait))?;
    let mut out = stream.try_clone()?;
    out.write_all(request.as_bytes())?;
    out.write_all(b"\n")?;
    let mut line = String::new();
    if BufReader::new(stream).read_line(&mut line)? == 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::UnexpectedEof,
            "no answer",
        ));
    }
    Ok(line)
}

fn read_answer(line: &str, count: usize) -> Result<Vec<Vec<f32>>, String> {
    let v: serde_json::Value =
        serde_json::from_str(line).map_err(|e| format!("a bad answer from the helper: {e}"))?;
    if let Some(e) = v["error"].as_str() {
        return Err(e.to_string());
    }
    if v["model"].as_str() != Some(MODEL) {
        return Err(format!(
            "the helper uses another model ({}). Build all AIfred programs again.",
            v["model"]
        ));
    }
    let vectors: Vec<Vec<f32>> = serde_json::from_value(v["vectors"].clone())
        .map_err(|e| format!("a bad answer from the helper: {e}"))?;
    if vectors.len() != count || vectors.iter().any(|x| x.len() != DIMS) {
        return Err("a bad answer from the helper: wrong number of vectors".into());
    }
    Ok(vectors)
}

/// Waits until a new helper opens its socket. When the helper stops first,
/// the error is the last line of its log.
fn connect_new(
    dir: &Path,
    socket: &Path,
    mut helper: Option<std::process::Child>,
) -> Result<UnixStream, String> {
    let start = Instant::now();
    let result = loop {
        match UnixStream::connect(socket) {
            Ok(s) => break Ok(s),
            Err(e) if start.elapsed() > START_WAIT => {
                break Err(format!("the meaning search helper did not start: {e}"));
            }
            Err(_) => {}
        }
        if let Some(Ok(Some(_))) = helper.as_mut().map(|c| c.try_wait()) {
            // It stopped. Another helper can have started at the same time.
            if let Ok(s) = UnixStream::connect(socket) {
                break Ok(s);
            }
            let log = std::fs::read_to_string(dir.join("embed.log")).unwrap_or_default();
            let why = log.lines().last().unwrap_or("no message").to_string();
            return Err(format!("the meaning search helper stopped: {why}"));
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    // Collect the exit status when it stops, so no finished process stays listed.
    if let Some(mut c) = helper {
        std::thread::spawn(move || {
            let _ = c.wait();
        });
    }
    result
}

/// Starts the helper: the AIfred program in its helper mode
/// (`aifred embed-helper <data folder> <socket>`). When one already runs,
/// the new one stops at once.
#[cfg(not(test))]
fn start_helper(dir: &Path, socket: &Path) -> Result<Option<std::process::Child>, String> {
    use std::os::unix::process::CommandExt;
    // Only the AIfred program saves its path. A test program of another
    // part does not, so it never starts a copy of itself.
    let Some(program) = crate::program::path() else {
        return Err("the meaning search helper is missing: this program is not AIfred".into());
    };
    let log = std::fs::File::create(dir.join("embed.log"))
        .map_err(|e| format!("cannot make embed.log: {e}"))?;
    let child = std::process::Command::new(program)
        .arg(crate::program::EMBED_HELPER)
        .arg(dir)
        .arg(socket)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(log)
        // Its own process group: it keeps running when a tool server stops.
        .process_group(0)
        .spawn()
        .map_err(|e| format!("cannot start {}: {e}", program.display()))?;
    Ok(Some(child))
}

/// The tests run the helper in a thread, with the shared test model folder.
#[cfg(test)]
fn start_helper(dir: &Path, socket: &Path) -> Result<Option<std::process::Child>, String> {
    let (dir, socket) = (dir.to_path_buf(), socket.to_path_buf());
    std::thread::spawn(move || {
        if let Err(e) =
            aifred_embed::serve(&dir, &socket, &models_dir(&dir), Duration::from_secs(30))
        {
            eprintln!("test helper: {e:#}");
        }
    });
    Ok(None)
}

/// The vector as SQLite-vec reads it: 4 bytes for each number, little-endian.
pub(crate) fn to_blob(v: &[f32]) -> Vec<u8> {
    v.iter().flat_map(|x| x.to_le_bytes()).collect()
}

/// One kind of text that has vectors.
struct Kind {
    table: &'static str,
    vec: &'static str,
    /// The column that links a row to its vector.
    key: &'static str,
    /// The text that the vector comes from.
    text: &'static str,
    /// The rows that have a text.
    filter: &'static str,
}

const KINDS: [Kind; 5] = [
    Kind {
        table: "memory",
        vec: "memory_vec",
        key: "id",
        text: "text",
        filter: "1",
    },
    Kind {
        table: "turn",
        vec: "turn_vec",
        key: "id",
        text: "text",
        filter: "1",
    },
    Kind {
        table: "user_task",
        vec: "task_vec",
        key: "rid",
        text: "title || char(10) || notes",
        // The view has only the user's tasks (db.rs SCHEMA_19).
        filter: "1",
    },
    Kind {
        table: "work_item",
        vec: "work_item_vec",
        key: "rowid",
        text: "title || char(10) || place || char(10) || details",
        filter: "1",
    },
    Kind {
        table: "period_summary",
        vec: "period_vec",
        key: "id",
        text: "summary",
        filter: "1",
    },
];

/// Checks that the helper answers and the model works, with one short text.
/// A fill that finds nothing to do does not ask the helper, so it cannot tell.
pub fn check(dir: &Path) -> Result<(), ModelError> {
    embed_texts(dir, &["check".to_string()]).map(|_| ())
}

/// Only one fill at a time in one program. Two fills would make the same vectors.
static FILL: Mutex<()> = Mutex::new(());

/// The error of fill_missing when the model cannot load. Other fill errors
/// (for example a database that is busy) do not turn meaning search off, so
/// the app must not report them as this one.
#[derive(Debug)]
pub struct ModelError(pub String);

impl std::fmt::Display for ModelError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "meaning search is off: {}", self.0)
    }
}

impl std::error::Error for ModelError {}

/// Makes the vectors that are missing, with the helper. Returns how many it
/// made. When the model cannot load, the error is a ModelError.
pub fn fill_missing(dir: &Path) -> anyhow::Result<usize> {
    let _one = FILL.lock().unwrap_or_else(|e| e.into_inner());
    // Vectors of another model do not match the vectors of this one.
    db::write(dir, |tx| {
        let saved: Option<String> = db::meta(tx, "embed_model")?;
        if saved.as_deref() != Some(MODEL) {
            for k in &KINDS {
                tx.execute(&format!("DELETE FROM {}", k.vec), [])?;
            }
            db::set_meta(tx, "embed_model", MODEL)?;
        }
        Ok(())
    })?;
    let mut made = 0;
    for k in &KINDS {
        let missing = format!(
            "SELECT {key}, {text} FROM {table} WHERE {filter} AND NOT EXISTS (SELECT 1 FROM {vec} v WHERE v.rowid = {table}.{key}) LIMIT {BATCH}",
            filter = k.filter,
            key = k.key,
            text = k.text,
            table = k.table,
            vec = k.vec
        );
        let current = format!("SELECT {} FROM {} WHERE {} = ?1", k.text, k.table, k.key);
        loop {
            let rows: Vec<(i64, String)> = db::read(dir, |c| {
                let mut q = c.prepare(&missing)?;
                Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                    .collect::<Result<_, _>>()?)
            })?;
            if rows.is_empty() {
                break;
            }
            let texts: Vec<String> = rows.iter().map(|r| r.1.clone()).collect();
            let vectors = embed_texts(dir, &texts)?;
            made += db::write(dir, |tx| {
                let mut n = 0;
                for ((id, text), v) in rows.iter().zip(&vectors) {
                    // The text can change while the model works. Then this
                    // vector is old: the next fill makes the right one.
                    let now: Option<String> = tx.query_row(&current, [id], |r| r.get(0)).ok();
                    if now.as_ref() != Some(text) {
                        continue;
                    }
                    tx.execute(&format!("DELETE FROM {} WHERE rowid = ?1", k.vec), [id])?;
                    tx.execute(
                        &format!("INSERT INTO {} (rowid, embedding) VALUES (?1, ?2)", k.vec),
                        rusqlite::params![id, to_blob(v)],
                    )?;
                    n += 1;
                }
                Ok(n)
            })?;
            if rows.len() < BATCH as usize {
                break;
            }
        }
    }
    Ok(made)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    #[test]
    fn a_long_data_folder_gets_its_socket_in_the_runtime_folder() {
        let short = Path::new("https://rp2.alfred511.top/huggingface/home/sam/.aifred");
        assert_eq!(socket_path(short).unwrap(), short.join(SOCKET));
        let long = PathBuf::from(format!(
            "https://rp2.alfred511.top/huggingface/home/sam/{}",
            "x".repeat(120)
        ));
        if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| Path::new(d).is_dir()) {
            let p = socket_path(&long).unwrap();
            assert!(
                p.starts_with(&run) && p.as_os_str().len() <= 108,
                "{}",
                p.display()
            );
            assert_eq!(
                p,
                socket_path(&long).unwrap(),
                "the same folder, the same socket"
            );
            assert_ne!(p, socket_path(&long.join("y")).unwrap());
        }
    }

    #[test]
    fn the_model_name_and_the_files_match_the_helper() {
        assert_eq!(MODEL, aifred_embed::MODEL);
        assert_eq!(SOCKET, aifred_embed::SOCKET);
    }

    use crate::clickup::ClickupItem;
    use crate::clickup_list::ClickupStore;
    use crate::db::tests::temp;
    use crate::github::GithubItem;
    use crate::github_list::GithubStore;
    use crate::history::History;
    use crate::memory::MemoryStore;
    use crate::tasks::TaskStore;

    fn count(dir: &Path, table: &str) -> i64 {
        db::read(dir, |c| {
            Ok(c.query_row(&format!("SELECT count(*) FROM {table}"), [], |r| r.get(0))?)
        })
        .unwrap()
    }

    #[test]
    fn meaning_search_finds_related_words_in_all_kinds_of_data() {
        let d = temp("embed-all");
        let m = MemoryStore::new(&d);
        m.add("I prefer window seats", "you", false).unwrap();
        m.add("My cat is called Miso", "you", false).unwrap();
        let h = History::new(&d);
        h.append("user", "Remind me about the dentist on Tuesday", "claude")
            .unwrap();
        h.append("assistant", "Sure, noted.", "claude").unwrap();
        let t = TaskStore::new(&d);
        t.add("Send the invoice to Acme", "", "you").unwrap();
        t.add("Walk in the park", "", "you").unwrap();
        let gh: GithubItem = serde_json::from_value(serde_json::json!({
            "id": "acme/shop#1", "repo": "acme/shop", "number": 1, "title": "Fix the price rounding",
            "url": "u", "reason": "assigned", "updated": "2026-10-01T10:00:00Z"
        }))
        .unwrap();
        GithubStore::new(&d)
            .save_fetch(vec![gh], &[], 1000)
            .unwrap();
        let cu = |id: &str, title: &str| -> ClickupItem {
            serde_json::from_value(serde_json::json!({
                "id": id, "title": title, "url": "u", "status": "to do", "list": "Admin", "workspace": "Rymera", "updated": 1
            }))
            .unwrap()
        };
        let cs = ClickupStore::new(&d);
        cs.save(
            "acct",
            crate::clickup::MyTasks {
                items: vec![cu("a", "Q4 budget"), cu("b", "Team lunch")],
                cut: vec![],
            },
            1000,
        )
        .unwrap();

        let made = fill_missing(&d).unwrap();
        assert_eq!(made, 2 + 2 + 2 + 1 + 2);
        assert_eq!(fill_missing(&d).unwrap(), 0, "a second fill makes nothing");

        // No word is shared between each question and the text it must find.
        let facts = m.search("book my flight", Some(1)).unwrap();
        assert_eq!(facts[0].text, "I prefer window seats");
        assert_eq!(
            m.search("pets", Some(1)).unwrap()[0].text,
            "My cat is called Miso"
        );
        let chats = h.search("doctor appointment", Some(1)).unwrap();
        assert_eq!(chats[0].hit.text, "Remind me about the dentist on Tuesday");
        assert_eq!(
            t.search("billing", false, Some(1)).unwrap()[0].title,
            "Send the invoice to Acme"
        );
        let items = GithubStore::new(&d)
            .search("wrong cost", &[], false, Some(1))
            .unwrap();
        assert_eq!(items[0].item["title"], "Fix the price rounding");
        let tasks = cs.search("acct", "money plan", false, Some(1)).unwrap();
        assert_eq!(tasks[0].item["title"], "Q4 budget");

        // Only common words: no word search. The meaning search runs, but
        // nothing is related, so nothing comes back.
        assert!(crate::db::match_query("who are you").is_none());
        assert!(
            m.search("who are you", None).unwrap().is_empty(),
            "nothing related"
        );
        assert!(
            m.search("thanks!", None).unwrap().is_empty(),
            "nothing related"
        );
        assert!(
            t.search("write a haiku about the sea", false, None)
                .unwrap()
                .is_empty(),
            "nothing related"
        );

        // A word match still comes first.
        assert_eq!(
            t.search("park", false, Some(1)).unwrap()[0].title,
            "Walk in the park"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_changed_text_gets_a_new_vector_and_an_unchanged_download_keeps_its_vectors() {
        let d = temp("embed-change");
        let t = TaskStore::new(&d);
        let task = t.add("Buy milk", "", "you").unwrap();
        let gh: GithubItem = serde_json::from_value(serde_json::json!({
            "id": "acme/shop#1", "repo": "acme/shop", "number": 1, "title": "Fix the price rounding",
            "url": "u", "reason": "assigned", "updated": "2026-10-01T10:00:00Z"
        }))
        .unwrap();
        let g = GithubStore::new(&d);
        g.save_fetch(vec![gh.clone()], &[], 1000).unwrap();
        assert_eq!(fill_missing(&d).unwrap(), 2);

        g.save_fetch(vec![gh], &[], 2000).unwrap();
        assert_eq!(
            count(&d, "work_item_vec"),
            1,
            "the same item again keeps its vector"
        );
        g.save_details("acme/shop#1", "Prices show too many decimals")
            .unwrap();
        assert_eq!(
            count(&d, "work_item_vec"),
            0,
            "a new description deletes the old vector"
        );

        t.set_done(&task.id, true).unwrap();
        assert_eq!(count(&d, "task_vec"), 1, "done does not change the text");
        t.remove(&task.id).unwrap();
        assert_eq!(count(&d, "task_vec"), 0, "a deleted task loses its vector");
        assert_eq!(fill_missing(&d).unwrap(), 1, "only the changed item");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_fill_of_more_than_32_texts_works() {
        // The old, compressed model failed with more than 32 texts in one call.
        let d = temp("embed-many");
        let m = MemoryStore::new(&d);
        for i in 0..70 {
            m.add(&format!("Fact number {i}"), "you", false).unwrap();
        }
        assert_eq!(fill_missing(&d).unwrap(), 70);
        assert_eq!(count(&d, "memory_vec"), 70);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn vectors_of_another_model_are_made_again() {
        let d = temp("embed-model");
        MemoryStore::new(&d).add("I like tea", "you", true).unwrap();
        assert_eq!(fill_missing(&d).unwrap(), 1);
        db::write(&d, |tx| {
            Ok(tx.execute(
                "UPDATE meta SET value = 'old-model' WHERE key = 'embed_model'",
                [],
            )?)
        })
        .unwrap();
        assert!(
            MemoryStore::new(&d)
                .search("coffee", None)
                .unwrap()
                .is_empty(),
            "old vectors are not used"
        );
        assert_eq!(fill_missing(&d).unwrap(), 1, "all vectors again");
        assert_eq!(
            db::read(&d, |c| db::meta(c, "embed_model"))
                .unwrap()
                .as_deref(),
            Some(MODEL)
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
