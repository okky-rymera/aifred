//! aifred terminal commands. Prototype.
//!
//!   aifred ask "<message>"     Ask one question in the current conversation.
//!   aifred chat                Talk in a loop. Empty line or Ctrl-D ends it.
//!   aifred remember "<fact>"   Add a fact to memory.
//!   aifred new                 Start a new conversation (old one is archived).
//!
//! Data folder: $AIFRED_HOME, or ~/.aifred by default.

use std::io::Write;

use aifred_core::backend::{Event, claude::ClaudeBackend};
use aifred_core::{Assistant, default_data_dir};

async fn ask(a: &Assistant<ClaudeBackend>, prompt: &str) -> anyhow::Result<()> {
    let mut on_event = |e: Event| match e {
        Event::ToolCall { server, tool, input } => eprintln!("[aifred] tool: {server}.{tool} {input}"),
        Event::Notice { message } => eprintln!("[aifred] {message}"),
    };
    let ans = a.ask(prompt, &mut on_event).await?;
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    if std::env::var_os("ANTHROPIC_API_KEY").is_some() {
        eprintln!("[aifred] warning: ANTHROPIC_API_KEY is set, so claude uses the API key, not your login.");
    }
    let dir = default_data_dir()?;
    let backend = ClaudeBackend { work_dir: dir.join("claude"), model: std::env::var("AIFRED_MODEL").ok() };
    let tools_dir = std::env::current_exe()?.parent().map(Into::into).unwrap_or_default();
    let a = Assistant::new(dir, backend, tools_dir)?;

    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("ask") if args.len() > 1 => ask(&a, &args[1..].join(" ")).await,
        Some("chat") => loop {
            print!("you> ");
            std::io::stdout().flush()?;
            let mut line = String::new();
            if std::io::stdin().read_line(&mut line)? == 0 || line.trim().is_empty() {
                return Ok(());
            }
            if let Err(e) = ask(&a, line.trim()).await {
                eprintln!("[aifred] error: {e:#}");
            }
        },
        Some("remember") if args.len() > 1 => {
            a.remember(&args[1..].join(" "))?;
            println!("Saved to {}", a.dir.join("memory.md").display());
            Ok(())
        }
        Some("new") => {
            a.new_conversation()?;
            println!("Started a new conversation.");
            Ok(())
        }
        _ => {
            eprintln!("usage: aifred ask \"<message>\" | chat | remember \"<fact>\" | new");
            std::process::exit(2);
        }
    }
}
