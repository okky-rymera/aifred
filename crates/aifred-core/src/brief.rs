//! The brief: a short text at the top of the home screen. It says what needs
//! the user today and why. AIfred writes it once a day, or when the user
//! asks, in one AI call with no tools: the call gets the overview (overview.rs).
//! The same call gives a reason for the items that need the user, and a time
//! guess for each open item that has none (plan.rs).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::{Context, bail};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::db;

/// The model of the brief: careful enough to pick what matters, and fast.
pub const MODEL: &str = "sonnet";

pub const RULES: &str = "\
You are AIfred, a personal work assistant. You write the user's brief for today: a short text at the top of their to-do list.
You get today's date, the work day, today's plan if there is one, and all open items. Each item has a key, like task:t123, github:owner/repo#7, or clickup:86a.

Write the brief:
- Start with one sentence that says how many things need the user today. Then 2 to 4 short sentences about the most important items and why they matter now.
- Name an item as a Markdown link to its key, for example [the PayPal bug](item:github:acme/shop#7). Use the key exactly as you got it.
- Use plain, short words. No greeting, no list, no headings. At most 80 words.
- Items that need the user: urgent or overdue items, items due today, review requests that wait, and new items that block others.

Also give:
- why: for each item that needs the user today, one short reason, at most 15 words. Example: \"It waits for your review for 3 days.\"
- minutes: for each open item in `needs_guess`, how many minutes the work takes, as a whole number from 5 to 600.

Answer with only a JSON object: {\"text\": \"...\", \"why\": {\"<key>\": \"...\"}, \"minutes\": {\"<key>\": 30}}";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Brief {
    /// "YYYY-MM-DD".
    pub day: String,
    /// When AIfred read the tasks and items for it, in Unix seconds. A change
    /// after this time can be missing from the brief.
    pub written: u64,
    /// Markdown. Links to "item:<key>" name items.
    pub text: String,
    /// A short reason for each item that needs the user, by item key.
    pub why: BTreeMap<String, String>,
}

/// The brief of `day`, or None when AIfred has not written it.
pub fn get(dir: &Path, day: &str) -> anyhow::Result<Option<Brief>> {
    db::read(dir, |c| {
        let row: Option<(u64, String, String)> = c
            .query_row(
                "SELECT written, text, why FROM brief WHERE day = ?1",
                [day],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((written, text, why)) => Some(Brief {
                day: day.into(),
                written,
                text,
                why: serde_json::from_str(&why)?,
            }),
            None => None,
        })
    })
}

/// Saves the brief, and the time guesses for items that have none yet.
pub fn save(dir: &Path, brief: &Brief, minutes: &BTreeMap<String, u32>) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        tx.execute(
            "INSERT INTO brief (day, written, text, why) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (day) DO UPDATE SET written = ?2, text = ?3, why = ?4",
            rusqlite::params![
                brief.day,
                brief.written as i64,
                brief.text,
                serde_json::to_string(&brief.why)?
            ],
        )?;
        for (key, m) in minutes {
            let has: Option<i64> = tx
                .query_row("SELECT 1 FROM estimate WHERE key = ?1", [key], |r| r.get(0))
                .optional()?;
            if has.is_none() {
                crate::plan::set_estimate_in(tx, key, Some(*m), "aifred")?;
            }
        }
        Ok(())
    })
}

/// The keys of all items in an overview (overview.rs).
pub fn keys(overview: &Value) -> Vec<String> {
    let lists = [
        &overview["local_tasks"],
        &overview["github"]["items"],
        &overview["clickup"]["tasks"],
    ];
    lists
        .into_iter()
        .flat_map(|l| l.as_array().into_iter().flatten())
        .filter_map(|i| i["key"].as_str().map(String::from))
        .collect()
}

/// The text of the AI call. `now` is the date and time, `day` the work day
/// and lunch, `plan` today's plan as JSON or null.
pub fn prompt(
    overview: &Value,
    now: &str,
    day: &str,
    plan: &Value,
    needs_guess: &[String],
) -> String {
    format!(
        "Today: {now}\nWork day: {day}\nToday's plan: {plan}\nneeds_guess: {}\n\nOpen items:\n{overview}",
        serde_json::to_string(needs_guess).unwrap_or_default()
    )
}

/// The text, the reasons by key, and the time guesses by key.
pub type Parsed = (String, BTreeMap<String, String>, BTreeMap<String, u32>);

/// The brief, the reasons, and the time guesses from the AI's answer. Keys
/// that are not in `known` are dropped: the AI must not make up items.
pub fn parse(answer: &str, known: &[String]) -> anyhow::Result<Parsed> {
    // The JSON object, also when the AI put a code fence or words around it.
    let start = answer
        .find('{')
        .context("the brief answer has no JSON object")?;
    let end = answer
        .rfind('}')
        .context("the brief answer has no JSON object")?;
    let v: Value =
        serde_json::from_str(&answer[start..=end]).context("the brief answer is not valid JSON")?;
    let text = v["text"].as_str().unwrap_or_default().trim().to_string();
    if text.is_empty() {
        bail!("the brief answer has no text");
    }
    let known: HashSet<&str> = known.iter().map(String::as_str).collect();
    let why = v["why"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| known.contains(k.as_str()))
        .filter_map(|(k, w)| w.as_str().map(|w| (k.clone(), w.trim().to_string())))
        .filter(|(_, w)| !w.is_empty())
        .collect();
    let minutes = v["minutes"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(k, _)| known.contains(k.as_str()))
        .filter_map(|(k, m)| {
            m.as_f64()
                .map(|m| (k.clone(), m.round().clamp(5.0, 600.0) as u32))
        })
        .collect();
    Ok((text, why, minutes))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn the_answer_gives_text_reasons_and_guesses_for_known_items() {
        let known = vec!["task:t1".to_string(), "github:a/b#7".to_string()];
        let answer =
            "```json\n{\"text\": \"Two things need you. [The bug](item:github:a/b#7) is urgent.\",
            \"why\": {\"github:a/b#7\": \"Customers are blocked.\", \"task:nope\": \"made up\"},
            \"minutes\": {\"task:t1\": 20.4, \"github:a/b#7\": 2000, \"clickup:x\": 10}}\n```";
        let (text, why, minutes) = parse(answer, &known).unwrap();
        assert!(text.starts_with("Two things"));
        assert_eq!(why.len(), 1, "an unknown key is dropped");
        assert_eq!(why["github:a/b#7"], "Customers are blocked.");
        assert_eq!(
            minutes,
            BTreeMap::from([
                ("task:t1".to_string(), 20),
                ("github:a/b#7".to_string(), 600)
            ])
        );
        assert!(parse("no json here", &known).is_err());
        assert!(parse("{\"text\": \"\"}", &known).is_err());
    }

    #[test]
    fn keys_come_from_all_lists() {
        let o = json!({
            "local_tasks": [{"key": "task:t1"}],
            "github": {"items": [{"key": "github:a/b#7"}]},
            "clickup": {"tasks": [{"key": "clickup:86a"}]}
        });
        assert_eq!(keys(&o), ["task:t1", "github:a/b#7", "clickup:86a"]);
    }

    #[test]
    fn a_brief_is_saved_with_the_guesses_for_items_with_none() {
        let d = std::env::temp_dir().join(format!("aifred-brief-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        crate::plan::set_estimate(&d, "task:t1", Some(45), "you").unwrap();
        let b = Brief {
            day: "2026-11-04".into(),
            written: 5,
            text: "Hi.".into(),
            why: BTreeMap::new(),
        };
        let m = BTreeMap::from([("task:t1".to_string(), 10), ("task:t2".to_string(), 25)]);
        save(&d, &b, &m).unwrap();
        assert_eq!(get(&d, "2026-11-04").unwrap(), Some(b));
        let e = crate::plan::estimates(&d).unwrap();
        assert_eq!(
            (e["task:t1"].minutes, e["task:t2"].minutes),
            (45, 25),
            "a guess that exists stays"
        );
        assert_eq!(get(&d, "2026-11-05").unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
