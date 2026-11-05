//! The day plan: the items that the user wants to do on one day, in order,
//! and a time guess for each item. The window places the items on the
//! work day, around lunch (settings.rs). The user makes the plan by hand, or
//! AIfred makes it in the plan chat with the day_plan tools (aifred-tools).

use std::collections::{BTreeMap, HashSet};
use std::path::Path;

use anyhow::bail;
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};

use crate::{db, items};

/// One item in the plan.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlanItem {
    /// The item key (items.rs).
    pub key: String,
    /// Why the item is at this place, when AIfred placed it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub why: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DayPlan {
    /// "YYYY-MM-DD".
    pub day: String,
    /// "ai": AIfred made the plan. "self": the user made it.
    pub mode: String,
    /// The first item first.
    pub items: Vec<PlanItem>,
    /// When the plan changed last, in Unix seconds.
    pub updated: u64,
}

/// A time guess for an item.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Estimate {
    pub minutes: u32,
    /// "you" or "aifred".
    pub by: String,
}

/// The shortest and the longest time guess, in minutes.
pub const MIN_MINUTES: u32 = 5;
pub const MAX_MINUTES: u32 = 600;

/// Today in the user's own time zone, "YYYY-MM-DD".
pub fn today() -> String {
    chrono::Local::now().format("%F").to_string()
}

pub(crate) fn check_day(day: &str) -> anyhow::Result<()> {
    let ok = day.len() == 10
        && day.bytes().enumerate().all(|(i, b)| {
            if i == 4 || i == 7 {
                b == b'-'
            } else {
                b.is_ascii_digit()
            }
        });
    if !ok {
        bail!("the day \"{day}\" is not in the form YYYY-MM-DD");
    }
    Ok(())
}

/// The plan of `day`, or None when the day has no plan.
pub fn get(dir: &Path, day: &str) -> anyhow::Result<Option<DayPlan>> {
    check_day(day)?;
    db::read(dir, |c| {
        let row: Option<(String, String, u64)> = c
            .query_row(
                "SELECT mode, items, updated FROM day_plan WHERE day = ?1",
                [day],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        Ok(match row {
            Some((mode, items, updated)) => Some(DayPlan {
                day: day.into(),
                mode,
                items: serde_json::from_str(&items)?,
                updated,
            }),
            None => None,
        })
    })
}

/// Saves the plan of `day`. An item key that is twice in the list is kept
/// only at its first place.
///
/// A key that is already in the saved plan of `day` stays, also when its
/// item is now done: a finished item keeps its place on the day. A new key
/// must be of an open item.
///
/// `strict`: a new key of an item that is not open is an error, with the bad
/// keys in the text, so the AI can fix its call. Without `strict` (the
/// window), such keys are dropped: an item can close while the plan is open.
pub fn set(
    dir: &Path,
    day: &str,
    mode: &str,
    items: Vec<PlanItem>,
    strict: bool,
) -> anyhow::Result<DayPlan> {
    check_day(day)?;
    if mode != "ai" && mode != "self" {
        bail!("the plan mode must be \"ai\" or \"self\", not \"{mode}\"");
    }
    db::write(dir, |tx| {
        let old: HashSet<String> = tx
            .query_row("SELECT items FROM day_plan WHERE day = ?1", [day], |r| {
                r.get::<_, String>(0)
            })
            .optional()?
            .map(|j| serde_json::from_str::<Vec<PlanItem>>(&j))
            .transpose()?
            .unwrap_or_default()
            .into_iter()
            .map(|i| i.key)
            .collect();
        let mut seen = HashSet::new();
        let mut keep = Vec::new();
        let mut bad = Vec::new();
        for mut i in items {
            i.key = i.key.trim().to_string();
            i.why = i
                .why
                .map(|w| w.trim().to_string())
                .filter(|w| !w.is_empty());
            if !seen.insert(i.key.clone()) {
                continue;
            }
            if old.contains(&i.key) || items::is_open(tx, &i.key).unwrap_or(false) {
                keep.push(i);
            } else {
                bad.push(i.key);
            }
        }
        if strict && !bad.is_empty() {
            bail!(
                "these keys are not open items: {}. Take the keys from the overview tool",
                bad.join(", ")
            );
        }
        let plan = DayPlan {
            day: day.into(),
            mode: mode.into(),
            items: keep,
            updated: crate::files::now(),
        };
        tx.execute(
            "INSERT INTO day_plan (day, mode, items, updated) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT (day) DO UPDATE SET mode = ?2, items = ?3, updated = ?4",
            rusqlite::params![
                plan.day,
                plan.mode,
                serde_json::to_string(&plan.items)?,
                plan.updated as i64
            ],
        )?;
        Ok(plan)
    })
}

/// Removes the plan of `day`. The plan chat of the day stays.
pub fn clear(dir: &Path, day: &str) -> anyhow::Result<()> {
    check_day(day)?;
    db::write(dir, |tx| {
        tx.execute("DELETE FROM day_plan WHERE day = ?1", [day])?;
        Ok(())
    })
}

/// All time guesses, by item key.
pub fn estimates(dir: &Path) -> anyhow::Result<BTreeMap<String, Estimate>> {
    db::read(dir, |c| {
        let mut q = c.prepare("SELECT key, minutes, by FROM estimate")?;
        let rows = q.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                Estimate {
                    minutes: r.get(1)?,
                    by: r.get(2)?,
                },
            ))
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// Saves a time guess. None removes it. A guess from AIfred does not replace
/// a guess from the user, so AIfred cannot undo the user's own change.
/// Returns false when it kept the user's guess.
pub fn set_estimate(dir: &Path, key: &str, minutes: Option<u32>, by: &str) -> anyhow::Result<bool> {
    items::split(key)?;
    db::write(dir, |tx| set_estimate_in(tx, key, minutes, by))
}

pub(crate) fn set_estimate_in(
    tx: &rusqlite::Transaction,
    key: &str,
    minutes: Option<u32>,
    by: &str,
) -> anyhow::Result<bool> {
    if by != "you" && by != "aifred" {
        bail!("a time guess is by \"you\" or \"aifred\"");
    }
    if by == "aifred" {
        let theirs: Option<String> = tx
            .query_row("SELECT by FROM estimate WHERE key = ?1", [key], |r| {
                r.get(0)
            })
            .optional()?;
        if theirs.as_deref() == Some("you") {
            return Ok(false);
        }
    }
    match minutes {
        Some(m) => {
            let m = m.clamp(MIN_MINUTES, MAX_MINUTES);
            tx.execute(
                "INSERT INTO estimate (key, minutes, by) VALUES (?1, ?2, ?3) ON CONFLICT (key) DO UPDATE SET minutes = ?2, by = ?3",
                rusqlite::params![key, m, by],
            )?;
        }
        None => {
            tx.execute("DELETE FROM estimate WHERE key = ?1", [key])?;
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-plan-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn item(key: &str) -> PlanItem {
        PlanItem {
            key: key.into(),
            why: None,
        }
    }

    #[test]
    fn a_plan_is_saved_and_read_again() {
        let d = temp("save");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let b = format!("task:{}", s.add("B", "", "you").unwrap().id);
        assert_eq!(get(&d, "2026-11-04").unwrap(), None);
        let p = set(
            &d,
            "2026-11-04",
            "self",
            vec![item(&b), item(&a), item(&b)],
            true,
        )
        .unwrap();
        assert_eq!(
            p.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
            [b.as_str(), a.as_str()],
            "a key twice stays at its first place"
        );
        assert_eq!(get(&d, "2026-11-04").unwrap().unwrap().items, p.items);
        clear(&d, "2026-11-04").unwrap();
        assert_eq!(get(&d, "2026-11-04").unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_closed_item_is_an_error_for_the_ai_and_dropped_for_the_window() {
        let d = temp("closed");
        let s = TaskStore::new(&d);
        let t = s.add("A", "", "you").unwrap();
        let key = format!("task:{}", t.id);
        s.set_done(&t.id, true).unwrap();
        let e = set(&d, "2026-11-04", "ai", vec![item(&key)], true).unwrap_err();
        assert!(format!("{e}").contains(&key), "{e}");
        assert!(
            set(&d, "2026-11-04", "self", vec![item(&key)], false)
                .unwrap()
                .items
                .is_empty()
        );
        assert!(
            set(&d, "2026-11-04", "other", vec![], false).is_err(),
            "an unknown mode"
        );
        assert!(
            set(&d, "4 Nov", "self", vec![], false).is_err(),
            "a bad day"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_finished_item_keeps_its_place_when_the_plan_changes() {
        let d = temp("finished");
        let s = TaskStore::new(&d);
        let a = format!("task:{}", s.add("A", "", "you").unwrap().id);
        let b = s.add("B", "", "you").unwrap();
        let c = format!("task:{}", s.add("C", "", "you").unwrap().id);
        let bk = format!("task:{}", b.id);
        set(&d, "2026-11-04", "self", vec![item(&a), item(&bk)], false).unwrap();
        s.set_done(&b.id, true).unwrap();
        // The window moves C in; the AI copies the plan back. Both keep B.
        let p = set(
            &d,
            "2026-11-04",
            "self",
            vec![item(&c), item(&a), item(&bk)],
            false,
        )
        .unwrap();
        assert_eq!(
            p.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
            [c.as_str(), a.as_str(), bk.as_str()]
        );
        set(&d, "2026-11-04", "ai", vec![item(&bk), item(&a)], true).unwrap();
        // A done item that was not in the plan of that day is still new.
        assert!(set(&d, "2026-11-05", "ai", vec![item(&bk)], true).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn aifred_does_not_replace_the_users_time_guess() {
        let d = temp("estimate");
        assert!(set_estimate(&d, "github:a#1", Some(30), "aifred").unwrap());
        assert!(set_estimate(&d, "github:a#1", Some(45), "you").unwrap());
        assert!(!set_estimate(&d, "github:a#1", Some(90), "aifred").unwrap());
        assert_eq!(
            estimates(&d).unwrap()["github:a#1"],
            Estimate {
                minutes: 45,
                by: "you".into()
            }
        );
        set_estimate(&d, "github:a#1", Some(9999), "you").unwrap();
        assert_eq!(estimates(&d).unwrap()["github:a#1"].minutes, MAX_MINUTES);
        set_estimate(&d, "github:a#1", None, "you").unwrap();
        assert!(estimates(&d).unwrap().is_empty());
        assert!(set_estimate(&d, "nokey", Some(5), "you").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn today_is_a_date() {
        check_day(&today()).unwrap();
    }
}
