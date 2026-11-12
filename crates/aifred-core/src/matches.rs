//! Finished work that closes open work. After each download, AIfred also
//! saves the ClickUp tasks and the GitHub items that closed in the last
//! FINISHED_SECS (db::save_finished), also the ones that it never
//! had in its list. The brief gets the ones that closed as Done
//! (finished_for_ai). When an open AIfred task or an open mission step is
//! the same work as one of them, the brief says so, and AIfred marks it done
//! (apply).
//!
//! - Only AIfred's own tasks and mission steps. AIfred does not close
//!   ClickUp or GitHub items: the user does that there.
//! - The home page shows each match of the day with an undo (for_day_in,
//!   undo). AIfred does not ask first: the user sees the result and can
//!   undo it.
//! - A match that the user undid never comes again: apply() skips each
//!   pair of open work and finished item that it saw before.
//! - The finished items come from other people's text. The brief only names
//!   keys; this module checks each key and changes nothing else.

use std::collections::HashSet;
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db;
use crate::tasks::ChecklistItem;

/// How far back AIfred reads finished work, in seconds.
pub const FINISHED_SECS: u64 = 7 * 86_400;
/// At most this many finished items go to the brief, the newest first.
const MAX_FINISHED: usize = 40;
/// At most this many matches from one brief.
const MAX_MATCHES: usize = 10;
/// The longest reason, in characters.
const MAX_WHY: usize = 100;

/// One match from the brief's answer, before apply() checks it.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// "task:<id>" or "mission:<id>".
    pub key: String,
    /// The text of the mission step. "" for a task.
    pub step: String,
    /// The key of the finished item.
    pub by: String,
    pub why: String,
}

/// A match that the home page shows, with its undo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Match {
    pub id: i64,
    /// "task:<id>" or "mission:<id>".
    pub key: String,
    /// The task title, or the mission title.
    pub title: String,
    /// The mission step. "" for a task.
    pub step: String,
    pub by_key: String,
    pub by_title: String,
    pub why: String,
}

/// The finished items for the brief: the ClickUp and GitHub items that
/// closed as Done after `now` - FINISHED_SECS, the newest first.
pub(crate) fn finished_for_ai(c: &Connection, now: u64) -> anyhow::Result<Vec<Value>> {
    let mut q = c.prepare(
        "SELECT source || ':' || item_id, title, place, closed_at FROM work_item
         WHERE in_list = 0 AND close_reason = ?1 AND closed_at >= ?2
         ORDER BY closed_at DESC, item_id LIMIT ?3",
    )?;
    let rows = q.query_map(
        rusqlite::params![
            crate::status::DONE,
            now.saturating_sub(FINISHED_SECS) as i64,
            MAX_FINISHED as i64
        ],
        |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, String>(1)?,
                r.get::<_, String>(2)?,
                r.get::<_, i64>(3)?,
            ))
        },
    )?;
    // The pairs that AIfred saw before (apply skips them): the brief must
    // not name them again, else the reopened work gets no why and no mission.
    let mut seen = c.prepare("SELECT key, step FROM done_match WHERE by_key = ?1 ORDER BY id")?;
    let mut out = Vec::new();
    for row in rows {
        let (key, title, place, closed) = row?;
        let not_same: Vec<Value> = seen
            .query_map([&key], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            })?
            .filter_map(Result::ok)
            .map(|(k, step)| {
                match k
                    .strip_prefix("mission:")
                    .and_then(|i| i.parse::<i64>().ok())
                {
                    Some(id) => json!({ "mission": id, "step": step }),
                    None => json!({ "key": k }),
                }
            })
            .collect();
        let day = chrono::DateTime::from_timestamp(closed, 0)
            .map(|t| {
                t.with_timezone(&chrono::Local)
                    .format("%Y-%m-%d")
                    .to_string()
            })
            .unwrap_or_default();
        // `finished_at` is in Unix seconds, like `created` of a task: the
        // brief compares them (a repeating task added later is new work).
        let mut v = json!({ "key": key, "title": title, "place": place, "finished": day, "finished_at": closed });
        if !not_same.is_empty() {
            v["not_same_as"] = not_same.into();
        }
        out.push(v);
    }
    Ok(out)
}

/// The matches from `done` of the brief JSON. A task must be in
/// `open_tasks`, a mission in `missions`, and the finished item in
/// `finished`: the AI must not make up keys.
pub fn parse(
    v: &Value,
    open_tasks: &HashSet<&str>,
    missions: &HashSet<i64>,
    finished: &HashSet<&str>,
) -> Vec<Proposal> {
    let mut out: Vec<Proposal> = Vec::new();
    for m in v.as_array().into_iter().flatten() {
        let Some(by) = m["by"]
            .as_str()
            .map(str::trim)
            .filter(|b| finished.contains(b))
        else {
            continue;
        };
        let why = crate::text::one_line(m["why"].as_str().unwrap_or_default(), MAX_WHY);
        if why.is_empty() {
            continue;
        }
        let (key, step) = if let Some(k) = m["key"].as_str().map(str::trim) {
            if !k.starts_with("task:") || !open_tasks.contains(k) {
                continue;
            }
            (k.to_string(), String::new())
        } else if let Some(id) = m["mission"].as_i64().filter(|id| missions.contains(id)) {
            let step = crate::text::one_line(
                m["step"].as_str().unwrap_or_default(),
                crate::missions::MAX_STEP,
            );
            if step.is_empty() {
                continue;
            }
            (format!("mission:{id}"), step)
        } else {
            continue;
        };
        if out.iter().any(|p| p.key == key && p.step == step) {
            continue;
        }
        out.push(Proposal {
            key,
            step,
            by: by.to_string(),
            why,
        });
        if out.len() == MAX_MATCHES {
            break;
        }
    }
    out
}

/// The kind and the id of a match key: ("task", id) or ("mission", id).
fn split(key: &str) -> anyhow::Result<(&str, &str)> {
    match key.split_once(':') {
        Some((kind @ ("task" | "mission"), id)) if !id.is_empty() => Ok((kind, id)),
        _ => bail!("only a task or a mission step can be marked done by a match, not {key}"),
    }
}

/// The same step text: case and spaces do not count.
fn same_step(a: &str, b: &str) -> bool {
    a.trim().to_lowercase() == b.trim().to_lowercase()
}

/// Marks the open work of each match done and saves the match for the
/// brief of `day`. A pair of open work and finished item that AIfred saw
/// before is skipped, so a match that the user undid does not come again.
/// Work that is not open now is skipped. Returns how many it applied.
pub fn apply(dir: &Path, day: &str, matches: &[Proposal], now: u64) -> anyhow::Result<usize> {
    if matches.is_empty() {
        return Ok(0);
    }
    db::write(dir, |tx| {
        let mut n = 0;
        for m in matches {
            let seen: Vec<String> = tx
                .prepare("SELECT step FROM done_match WHERE key = ?1 AND by_key = ?2")?
                .query_map([&m.key, &m.by], |r| r.get(0))?
                .collect::<Result<_, _>>()?;
            if seen.iter().any(|s| same_step(s, &m.step)) {
                continue;
            }
            let Some(step) = close_in(tx, &m.key, &m.step, true, now)? else {
                continue;
            };
            tx.execute(
                "INSERT INTO done_match (day, key, step, by_key, why, made) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                rusqlite::params![day, m.key, step, m.by, m.why, now as i64],
            )?;
            n += 1;
        }
        Ok(n)
    })
}

/// Marks the task or the mission step done (`done` true) or open again.
/// Gives the step text as the checklist has it ("" for a task), or None
/// when there is nothing to change: the task is gone or has that state
/// already, or the mission is closed or has no such step.
fn close_in(
    tx: &Transaction,
    key: &str,
    step: &str,
    done: bool,
    now: u64,
) -> anyhow::Result<Option<String>> {
    let (kind, id) = split(key)?;
    match kind {
        "task" => {
            let was: Option<bool> = tx
                .query_row("SELECT done FROM user_task WHERE id = ?1", [id], |r| {
                    r.get(0)
                })
                .optional()?;
            if was != Some(!done) {
                return Ok(None);
            }
            crate::tasks::set_done_in(tx, id, done)?;
            Ok(Some(String::new()))
        }
        "mission" => {
            let raw: Option<String> = tx
                .query_row(
                    "SELECT checklist FROM mission WHERE id = ?1 AND status = 'open'",
                    [id],
                    |r| r.get(0),
                )
                .optional()?;
            let Some(raw) = raw else {
                return Ok(None);
            };
            let mut list: Vec<ChecklistItem> = serde_json::from_str(&raw)?;
            let Some(s) = list
                .iter_mut()
                .find(|s| s.done != done && same_step(&s.text, step))
            else {
                return Ok(None);
            };
            s.done = done;
            let text = s.text.clone();
            tx.execute(
                "UPDATE mission SET checklist = ?2, updated = ?3 WHERE id = ?1",
                rusqlite::params![id, serde_json::to_string(&list)?, now as i64],
            )?;
            Ok(Some(text))
        }
        _ => unreachable!("split() gives only task or mission"),
    }
}

/// The matches of the brief of `day` that the user did not undo.
pub(crate) fn for_day_in(c: &Connection, day: &str) -> anyhow::Result<Vec<Match>> {
    let mut q = c.prepare(
        "SELECT id, key, step, by_key, why FROM done_match WHERE day = ?1 AND undone = 0 ORDER BY id",
    )?;
    let rows: Vec<(i64, String, String, String, String)> = q
        .query_map([day], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    let mut out = Vec::new();
    for (id, key, step, by_key, why) in rows {
        let title = match split(&key)? {
            ("mission", m) => c
                .query_row("SELECT title FROM mission WHERE id = ?1", [m], |r| r.get(0))
                .optional()?,
            _ => crate::items::title_place(c, &key)?.map(|(t, _)| t),
        };
        // The task or the mission is gone: there is nothing to undo.
        let Some(title) = title else {
            continue;
        };
        let by_title = crate::items::title_place(c, &by_key)?
            .map(|(t, _)| t)
            .unwrap_or_else(|| by_key.clone());
        out.push(Match {
            id,
            key,
            title,
            step,
            by_key,
            by_title,
            why,
        });
    }
    Ok(out)
}

/// The undo of one match: the task or the step is open again, and the
/// brief never makes this match again.
pub fn undo(dir: &Path, id: i64, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        let row: Option<(String, String)> = tx
            .query_row(
                "SELECT key, step FROM done_match WHERE id = ?1 AND undone = 0",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let Some((key, step)) = row else {
            bail!("AIfred does not know the match {id}");
        };
        // The user can have opened it again already: then nothing changes.
        close_in(tx, &key, &step, false, now)?;
        tx.execute("UPDATE done_match SET undone = 1 WHERE id = ?1", [id])?;
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clickup::tests::temp;
    use crate::tasks::TaskStore;

    fn finished_row(d: &Path, id: &str, reason: &str, closed_at: u64) {
        db::write(d, |tx| {
            tx.execute(
                "INSERT INTO work_item (source, item_id, title, place, url, data, in_list, pos, first_seen, last_seen, closed_at, close_reason)
                 VALUES ('clickup', ?1, ?2, 'W / L', 'u', '{}', 0, 0, 1, 1, ?3, ?4)",
                rusqlite::params![id, format!("Done {id}"), closed_at as i64, reason],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn the_brief_gets_only_work_that_closed_as_done_lately() {
        let d = temp("matches-finished");
        let now = 100 * 86_400;
        finished_row(&d, "new", crate::status::DONE, now - 86_400);
        finished_row(&d, "old", crate::status::DONE, now - FINISHED_SECS - 1);
        finished_row(&d, "dropped", crate::status::CANCELLED, now - 60);
        let got = db::read(&d, |c| finished_for_ai(c, now)).unwrap();
        let keys: Vec<&str> = got.iter().filter_map(|v| v["key"].as_str()).collect();
        assert_eq!(keys, ["clickup:new"]);
        assert_eq!(got[0]["title"], "Done new");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_answer_gives_matches_with_known_keys_only() {
        let open: HashSet<&str> = ["task:a", "github:x/y#1"].into();
        let missions: HashSet<i64> = [3].into();
        let finished: HashSet<&str> = ["clickup:f"].into();
        let v = json!([
            {"key": "task:a", "by": "clickup:f", "why": " The same  invoice. "},
            {"key": "task:a", "by": "clickup:f", "why": "Twice."},
            {"key": "github:x/y#1", "by": "clickup:f", "why": "Not a local task."},
            {"key": "task:nope", "by": "clickup:f", "why": "Not open."},
            {"key": "task:a", "by": "clickup:made-up", "why": "Not finished."},
            {"key": "task:a", "by": "clickup:f", "why": " "},
            {"mission": 3, "step": " Send it ", "by": "clickup:f", "why": "Sent."},
            {"mission": 9, "step": "Send it", "by": "clickup:f", "why": "No such mission."},
            {"mission": 3, "step": "", "by": "clickup:f", "why": "No step."}
        ]);
        let got = parse(&v, &open, &missions, &finished);
        assert_eq!(
            got,
            [
                Proposal {
                    key: "task:a".into(),
                    step: String::new(),
                    by: "clickup:f".into(),
                    why: "The same invoice.".into(),
                },
                Proposal {
                    key: "mission:3".into(),
                    step: "Send it".into(),
                    by: "clickup:f".into(),
                    why: "Sent.".into(),
                },
            ]
        );
        assert!(parse(&json!(null), &open, &missions, &finished).is_empty());
    }

    #[test]
    fn a_match_marks_the_work_done_and_its_undo_stops_it_for_good() {
        let d = temp("matches-apply");
        finished_row(&d, "f", crate::status::DONE, 50);
        let s = TaskStore::new(&d);
        let task = s.add("Send the invoice", "", "you").unwrap();
        let t = format!("task:{}", task.id);
        let other = format!("task:{}", s.add("Other", "", "you").unwrap().id);
        db::write(&d, |tx| {
            crate::missions::save_in(
                tx,
                "2026-11-12",
                &[crate::missions::Proposal {
                    id: None,
                    title: "Bill Ana".into(),
                    keys: vec![other.clone()],
                    steps: vec!["Write it".into(), "Send it".into()],
                    doer: "you".into(),
                }],
                1,
            )
        })
        .unwrap();
        let m = crate::missions::today(&d, "2026-11-12").unwrap()[0].id;
        let mk = format!("mission:{m}");
        let by = |key: &str, step: &str| Proposal {
            key: key.into(),
            step: step.into(),
            by: "clickup:f".into(),
            why: "Same work.".into(),
        };

        let n = apply(
            &d,
            "2026-11-12",
            &[by(&t, ""), by(&mk, "send IT"), by(&mk, "No such step")],
            100,
        )
        .unwrap();
        assert_eq!(n, 2, "a step that is not there changes nothing");
        assert!(s.load().unwrap().iter().any(|x| x.id == task.id && x.done));
        let steps = crate::missions::get(&d, m).unwrap().checklist;
        assert!(!steps[0].done && steps[1].done);

        let list = db::read(&d, |c| for_day_in(c, "2026-11-12")).unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].title, "Send the invoice");
        assert_eq!(list[0].by_title, "Done f");
        assert_eq!(list[1].title, "Bill Ana");
        assert_eq!(list[1].step, "Send it", "the step as the checklist has it");

        // The same match again changes nothing.
        assert_eq!(apply(&d, "2026-11-12", &[by(&t, "")], 101).unwrap(), 0);
        undo(&d, list[0].id, 102).unwrap();
        assert!(s.load().unwrap().iter().any(|x| x.id == task.id && !x.done));
        assert_eq!(
            apply(&d, "2026-11-13", &[by(&t, "")], 103).unwrap(),
            0,
            "an undone match never comes again"
        );
        assert!(undo(&d, list[0].id, 104).is_err(), "only once");
        undo(&d, list[1].id, 105).unwrap();
        assert!(!crate::missions::get(&d, m).unwrap().checklist[1].done);
        assert_eq!(
            apply(&d, "2026-11-13", &[by(&mk, " SEND it ")], 106).unwrap(),
            0,
            "an undone step does not come again in other case"
        );
        let left = db::read(&d, |c| for_day_in(c, "2026-11-12")).unwrap();
        assert_eq!(left.len(), 0);

        // The brief learns the pairs that it must not name again.
        let got = db::read(&d, |c| finished_for_ai(c, 107)).unwrap();
        assert_eq!(
            got[0]["not_same_as"],
            json!([{ "key": t }, { "mission": m, "step": "Send it" }])
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
