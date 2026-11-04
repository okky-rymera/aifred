//! aifred terminal commands. Prototype.
//!
//!   aifred ask "<message>"     Ask one question in the current conversation.
//!   aifred chat                Talk in a loop. Empty line or Ctrl-D ends it.
//!   aifred remember "<fact>"   Add a fact to memory.
//!   aifred new                 Start a new conversation (old one is archived).
//!   aifred index               Make the missing meaning search vectors and
//!                              item summaries now.
//!
//! Data folder: $AIFRED_HOME, or ~/.aifred by default.

use std::io::Write;
use std::sync::Arc;

use aifred_core::backend::{Event, claude::ClaudeBackend};
use aifred_core::{Assistant, default_data_dir};

async fn ask(a: &Assistant<ClaudeBackend>, prompt: &str) -> anyhow::Result<()> {
    let mut on_event = |e: Event| match e {
        Event::ToolCall { server, tool, input } => eprintln!("[aifred] tool: {server}.{tool} {input}"),
        Event::Notice { message } => eprintln!("[aifred] {message}"),
    };
    let ans = a.ask(prompt, &mut on_event).await?;
    for f in &ans.memory {
        eprintln!("[aifred] from memory: {f}");
    }
    println!("{}\n", ans.text);
    let s = &ans.stats;
    eprintln!(
        "[aifred] startup={}ms total={}ms model={}ms tools={} cost={}",
        s.startup_ms,
        s.total_ms,
        ans.model_ms.map_or("?".into(), |v| v.to_string()),
        s.tool_calls.len(),
        ans.cost_usd.map_or("?".into(), |v| format!("${v:.4}")),
    );
    Ok(())
}

/// The work after an answer, as a task: a long conversation gets a new
/// summary (one more AI call), long new descriptions get item summaries (one
/// AI call with the fast model for each 15 items), and the new messages, facts, and tasks get
/// their meaning search vectors (the first time, this downloads the model).
/// Word search works without them. `chat` does not wait for this task, so
/// the user can type the next message at once.
fn after_answer(a: Arc<Assistant<ClaudeBackend>>) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        match a.summarize_if_long().await {
            Ok(true) => eprintln!("[aifred] saved a summary of this conversation"),
            Ok(false) => {}
            Err(e) => eprintln!("[aifred] cannot write the summary: {e:#}"),
        }
        // Items that the tool servers downloaded can need a summary.
        match a.summarize_items().await {
            Ok(0) => {}
            Ok(n) => eprintln!("[aifred] saved {n} item summaries"),
            Err(e) => eprintln!("[aifred] cannot write the item summaries: {e:#}"),
        }
        fill_vectors(a).await;
    })
}

/// fill_vectors blocks, so it runs on a thread for blocking work.
async fn fill_vectors(a: Arc<Assistant<ClaudeBackend>>) {
    match tokio::task::spawn_blocking(move || a.fill_vectors()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => eprintln!("[aifred] {e:#}"),
        Err(e) => eprintln!("[aifred] the meaning search fill stopped: {e}"),
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
        eprintln!("[aifred] warning: ANTHROPIC_API_KEY is set, so claude uses the API key, not your login.");
    }
    let dir = default_data_dir()?;
    let backend = ClaudeBackend { work_dir: dir.join("claude"), model: std::env::var("AIFRED_MODEL").ok() };
    let tools_dir = std::env::current_exe()?.parent().map(Into::into).unwrap_or_default();
    let a = Arc::new(Assistant::new(dir, backend, tools_dir)?);

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ask") if args.len() > 1 => {
            ask(&a, &args[1..].join(" ")).await?;
            // The program ends after this, so it waits for the work.
            let _ = after_answer(a.clone()).await;
            Ok(())
        }
        Some("chat") => {
            let mut work: Vec<tokio::task::JoinHandle<()>> = Vec::new();
            loop {
                print!("you> ");
                std::io::stdout().flush()?;
                let mut line = String::new();
                if std::io::stdin().read_line(&mut line)? == 0 || line.trim().is_empty() {
                    break;
                }
                match ask(&a, line.trim()).await {
                    Ok(()) => {
                        work.retain(|w| !w.is_finished());
                        work.push(after_answer(a.clone()));
                    }
                    Err(e) => eprintln!("[aifred] error: {e:#}"),
                }
            }
            // The summary and the vectors must not stop half-done.
            if work.iter().any(|w| !w.is_finished()) {
                eprintln!("[aifred] finishing the summary and the meaning search data...");
            }
            for w in work {
                let _ = w.await;
            }
            Ok(())
        }
        Some("remember") if args.len() > 1 => {
            a.remember(&args[1..].join(" "))?;
            println!("Saved to {}", aifred_core::db::path(&a.dir).display());
            Ok(())
        }
        Some("index") => {
            let n = a.fill_vectors()?;
            println!("Made {n} meaning search vectors.");
            // Long descriptions get their summaries for the overview tool.
            let n = a.summarize_items().await?;
            println!("Made {n} item summaries.");
            Ok(())
        }
        Some("new") => {
            if let Some(ended) = a.new_conversation()? {
                // The old conversation gets a summary, for search. It is one more AI call.
                match a.summarize_ended(ended).await {
                    Ok(true) => {
                        eprintln!("[aifred] saved a summary of the old conversation");
                        fill_vectors(a.clone()).await;
                    }
                    Ok(false) => {}
                    Err(e) => eprintln!("[aifred] cannot write the summary: {e:#}"),
                }
            }
            println!("Started a new conversation.");
            Ok(())
        }
        _ => {
            eprintln!("usage: aifred ask \"<message>\" | chat | remember \"<fact>\" | new | index");
            std::process::exit(2);
        }
    }
}
