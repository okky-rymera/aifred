//! The meaning search helper: one program that holds the model.
//!
//! - The app, the terminal commands, and the tool servers do not load the
//!   model. They send texts to this helper over a local socket
//!   (<data folder>/embed.sock) and get the vectors back. So only one copy of
//!   the model is in memory, also when many programs search at the same time.
//! - A client starts the helper when the socket does not answer
//!   (aifred-core/src/embed.rs). A lock file makes sure that only one helper runs.
//! - The helper stops itself after IDLE with no requests. All its memory goes
//!   back to the system. The next request starts it again (about 0.2 s).
//! - The model runs on the processor and uses RAM. It does not use the GPU.
//!
//! Protocol: one JSON line for each request, `{"texts": ["..."]}`, and one
//! JSON line for each answer, `{"model": "...", "vectors": [[...]]}` or
//! `{"error": "..."}`. A client can send more requests on one connection.

use std::fs::File;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::Context;
use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde::{Deserialize, Serialize};

/// The model name. aifred-core saves it with the vectors. When it changes,
/// all vectors are made again. aifred-core has the same name (a test checks it).
pub const MODEL: &str = "Qdrant/all-MiniLM-L6-v2-onnx";
/// The helper stops after this time with no requests.
pub const IDLE: Duration = Duration::from_secs(5 * 60);
/// The socket name in the data folder, when the path is short enough
/// (aifred-core/src/embed.rs, socket_path). aifred-core uses the same name.
pub const SOCKET: &str = "embed.sock";
pub const LOCK: &str = "embed.lock";
/// The model reads at most 256 word parts. More text only costs time.
const MAX_CHARS: usize = 2000;
/// The model files of the old, compressed model. The helper deletes them.
const OLD_MODEL_DIR: &str = "models--Xenova--all-MiniLM-L6-v2";

#[derive(Serialize, Deserialize)]
pub struct Request {
    pub texts: Vec<String>,
}

#[derive(Serialize, Deserialize, Default)]
pub struct Response {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub vectors: Option<Vec<Vec<f32>>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

pub struct Embedder(Mutex<TextEmbedding>);

impl Embedder {
    pub fn load(models_dir: &Path) -> anyhow::Result<Self> {
        std::fs::create_dir_all(models_dir)
            .with_context(|| format!("cannot make {}", models_dir.display()))?;
        let options = TextInitOptions::new(EmbeddingModel::AllMiniLML6V2)
            .with_cache_dir(models_dir.into())
            .with_show_download_progress(false)
            // Two threads: a fill in the background must not slow the computer.
            .with_intra_threads(2);
        let model =
            TextEmbedding::try_new(options).context("cannot load the meaning search model")?;
        Ok(Self(Mutex::new(model)))
    }

    /// One vector for each text. The model reads one text at a time: that was
    /// the fastest and used the least memory (64 long texts: 4.3 s and
    /// 175 MB, against 6.0 s and 602 MB in groups of 32).
    pub fn embed(&self, texts: &[String]) -> anyhow::Result<Vec<Vec<f32>>> {
        let cut: Vec<String> = texts
            .iter()
            .map(|t| t.chars().take(MAX_CHARS).collect())
            .collect();
        let mut model = self.0.lock().unwrap_or_else(|e| e.into_inner());
        model
            .embed(cut, Some(1))
            .context("the meaning search model failed")
    }
}

/// Runs the helper for data folder `dir` on `socket` until it is idle for
/// `idle`. Returns at once when another helper already runs for this folder.
pub fn serve(dir: &Path, socket: &Path, models_dir: &Path, idle: Duration) -> anyhow::Result<()> {
    let lock = File::create(dir.join(LOCK)).context("cannot make the helper lock file")?;
    if !take_lock(&lock) {
        return Ok(());
    }
    let socket = socket.to_path_buf();
    // The lock is ours, so an old socket file is from a helper that stopped.
    let _ = std::fs::remove_file(&socket);
    // Bind before the model loads: a client can connect at once and wait.
    let listener =
        UnixListener::bind(&socket).with_context(|| format!("cannot open {}", socket.display()))?;
    let _ = std::fs::remove_dir_all(models_dir.join(OLD_MODEL_DIR));

    let model = match Embedder::load(models_dir) {
        Ok(m) => Arc::new(m),
        Err(e) => {
            // Tell the clients that wait, then stop. The next request tries again.
            let message = format!("{e:#}");
            listener.set_nonblocking(true)?;
            while let Ok((stream, _)) = listener.accept() {
                let _ = stream.set_nonblocking(false);
                answer_error(stream, &message);
            }
            let _ = std::fs::remove_file(&socket);
            return Err(e);
        }
    };

    let state = Arc::new(State {
        busy: AtomicUsize::new(0),
        last: Mutex::new(Instant::now()),
    });
    // Non-blocking accept with a short wait: the loop can check for idle.
    // While idle, this wakes 20 times a second and does almost nothing.
    listener.set_nonblocking(true)?;
    loop {
        match listener.accept() {
            Ok((stream, _)) => {
                stream.set_nonblocking(false)?;
                state.touch();
                let (model, state) = (model.clone(), state.clone());
                std::thread::spawn(move || handle(stream, &model, &state));
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                if state.busy.load(Ordering::SeqCst) == 0 && state.idle_for() >= idle {
                    // Remove the socket first: a new client then starts a
                    // new helper and does not connect to this one.
                    let _ = std::fs::remove_file(&socket);
                    return Ok(());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e).context("the helper socket failed"),
        }
    }
}

/// Takes the lock. A helper that just stopped can still hold it for a short
/// time, so this waits up to 2 seconds. False: another helper runs.
fn take_lock(lock: &File) -> bool {
    let start = Instant::now();
    loop {
        if lock.try_lock().is_ok() {
            return true;
        }
        if start.elapsed() > Duration::from_secs(2) {
            return false;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

struct State {
    /// Requests that are running now.
    busy: AtomicUsize,
    /// When the last request started or ended.
    last: Mutex<Instant>,
}

impl State {
    fn touch(&self) {
        *self.last.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
    }

    fn idle_for(&self) -> Duration {
        self.last
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .elapsed()
    }
}

fn handle(stream: UnixStream, model: &Embedder, state: &State) {
    let Ok(read) = stream.try_clone() else { return };
    let mut out = stream;
    for line in BufReader::new(read).lines() {
        let Ok(line) = line else { return };
        state.busy.fetch_add(1, Ordering::SeqCst);
        let res = match serde_json::from_str::<Request>(&line) {
            Ok(req) => match model.embed(&req.texts) {
                Ok(v) => Response {
                    model: Some(MODEL.into()),
                    vectors: Some(v),
                    error: None,
                },
                Err(e) => Response {
                    error: Some(format!("{e:#}")),
                    ..Default::default()
                },
            },
            Err(e) => Response {
                error: Some(format!("a bad request: {e}")),
                ..Default::default()
            },
        };
        let sent = send(&mut out, &res);
        state.touch();
        state.busy.fetch_sub(1, Ordering::SeqCst);
        if sent.is_err() {
            return;
        }
    }
}

fn answer_error(stream: UnixStream, message: &str) {
    let mut out = stream;
    let _ = send(
        &mut out,
        &Response {
            error: Some(message.into()),
            ..Default::default()
        },
    );
}

fn send(out: &mut UnixStream, res: &Response) -> std::io::Result<()> {
    let mut line = serde_json::to_vec(res)?;
    line.push(b'\n');
    out.write_all(&line)
}

/// Where the model files are in a data folder.
pub fn models_dir(dir: &Path) -> PathBuf {
    dir.join("models")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-embed-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// The model folder that the aifred-core tests also use.
    fn models() -> PathBuf {
        PathBuf::from(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../target/test-models"
        ))
    }

    fn ask(dir: &Path, texts: &[&str]) -> Response {
        let stream = loop {
            match UnixStream::connect(dir.join(SOCKET)) {
                Ok(s) => break s,
                Err(_) => std::thread::sleep(Duration::from_millis(25)),
            }
        };
        let mut out = stream.try_clone().unwrap();
        let req = Request {
            texts: texts.iter().map(|t| t.to_string()).collect(),
        };
        out.write_all(format!("{}\n", serde_json::to_string(&req).unwrap()).as_bytes())
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).unwrap();
        serde_json::from_str(&line).unwrap()
    }

    #[test]
    fn the_helper_answers_more_than_32_texts_and_stops_when_idle() {
        let d = temp("idle");
        let (dd, m) = (d.clone(), models());
        let helper =
            std::thread::spawn(move || serve(&dd, &dd.join(SOCKET), &m, Duration::from_secs(1)));
        let texts: Vec<String> = (0..40).map(|i| format!("text {i}")).collect();
        let refs: Vec<&str> = texts.iter().map(String::as_str).collect();
        let res = ask(&d, &refs);
        assert_eq!(res.error, None);
        assert_eq!(res.model.as_deref(), Some(MODEL));
        assert_eq!(
            res.vectors.unwrap().len(),
            40,
            "the old model failed with more than 32 texts"
        );
        helper.join().unwrap().unwrap();
        assert!(
            !d.join(SOCKET).exists(),
            "an idle helper removes its socket and stops"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_second_helper_for_the_same_folder_stops_at_once() {
        let d = temp("one");
        let (dd, m) = (d.clone(), models());
        let first =
            std::thread::spawn(move || serve(&dd, &dd.join(SOCKET), &m, Duration::from_secs(5)));
        assert!(ask(&d, &["hello"]).vectors.is_some());
        let start = Instant::now();
        serve(&d, &d.join(SOCKET), &models(), Duration::from_secs(5)).unwrap();
        assert!(
            start.elapsed() < Duration::from_secs(4),
            "the second helper did not wait for idle"
        );
        assert!(
            ask(&d, &["still here"]).vectors.is_some(),
            "the first helper still answers"
        );
        first.join().unwrap().unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_same_text_gets_the_same_vector_alone_and_with_others() {
        // Vectors from a fill and from a question must match.
        let d = temp("same");
        let (dd, m) = (d.clone(), models());
        let helper =
            std::thread::spawn(move || serve(&dd, &dd.join(SOCKET), &m, Duration::from_secs(1)));
        let alone = ask(&d, &["send the invoice"]).vectors.unwrap().remove(0);
        let with = ask(&d, &["walk in the park", "send the invoice"])
            .vectors
            .unwrap()
            .remove(1);
        let diff: f32 = alone.iter().zip(&with).map(|(a, b)| (a - b).abs()).sum();
        assert!(diff < 1e-3, "diff {diff}");
        helper.join().unwrap().unwrap();
        std::fs::remove_dir_all(&d).unwrap();
    }
}
