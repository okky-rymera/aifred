//! Missions. A mission is the next
//! small piece of work: usually one item, sometimes two small items that the
//! user does together. For a tree of subtasks, it is the next open subtask,
//! not the full tree. The home page shows only the
//! missions of today, not each item.
//!
//! - The brief call (brief.rs) gives the missions of today, most important
//!   first. save_in() keeps at most MAX_TODAY of them. It counts only the
//!   missions that it keeps, so a dropped mission does not use a place.
//! - An item is in at most one mission (the key of `mission_item`).
//! - A mission has only the items that the brief named. The other items
//!   of their tree (links.rs) do not come with them: a mission is the next
//!   small piece of work.
//! - AIfred groups items at once. It does not ask the user.
//! - A mission keeps its id, so its checklist stays, when the brief gives
//!   that id and some items stay, or when half of its open items or more stay.
//! - The checklist is the mission's own. A tick does not change the items.
//! - The user can add an item to a mission or take one out (add_item(),
//!   remove_item(), with restore() as the undo). An item that the user
//!   added stays in that mission: the brief cannot take it away or put it
//!   in another mission. An item that the user took out never goes back in
//!   that mission by the brief. Only the user can add it again.
//! - The reason, the next action, and the goal of a mission are those of its
//!   first open item (the first item in the brief, until it closes). So the
//!   brief does not write them twice.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::db;
use crate::tasks::ChecklistItem;
use crate::text::one_line;

/// At most this many missions for one day.
pub const MAX_TODAY: usize = 7;
/// parse() keeps at most this many proposals. It is more than MAX_TODAY:
/// save_in() can drop some of them, and the next ones then fill the places.
const MAX_PROPOSALS: usize = 3 * MAX_TODAY;
/// The longest title, in characters.
const MAX_TITLE: usize = 80;
/// At most this many steps in a new mission.
const MAX_STEPS: usize = 6;
/// The longest step, in characters.
const MAX_STEP: usize = 100;
/// An open mission that did not change for this many days goes away.
const STALE_DAYS: u64 = 14;

/// One mission of the brief's answer, before save_in() checks it.
#[derive(Debug, Clone, PartialEq)]
pub struct Proposal {
    /// The id of an earlier mission that is the same work, from the AI.
    pub id: Option<i64>,
    pub title: String,
    /// The items, the item of the next action first.
    pub keys: Vec<String>,
    /// The steps of a new mission.
    pub steps: Vec<String>,
    /// "you" or "aifred".
    pub doer: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MissionItem {
    pub key: String,
    pub title: String,
    /// False when the item is done or closed.
    pub open: bool,
    /// Who put the item in the mission: "you" or "aifred".
    pub by: String,
}

/// How far the mission is. AIfred shows each part, so the user sees how
/// it got the value.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Progress {
    pub steps_done: usize,
    pub steps: usize,
    pub items_closed: usize,
    pub items: usize,
    /// (steps_done + items_closed) / (steps + items), from 0 to 100.
    pub percent: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Mission {
    pub id: i64,
    pub title: String,
    pub doer: String,
    /// The day of the last brief that ranked it.
    pub day: Option<String>,
    /// 1 is the first mission of that brief.
    pub rank: Option<u32>,
    /// "open" or "done".
    pub status: String,
    /// The first item is the item of the next action.
    pub items: Vec<MissionItem>,
    pub checklist: Vec<ChecklistItem>,
    pub progress: Progress,
}

/// The missions from the AI's answer: `missions` of the brief JSON. Keys
/// that are not in `known` are dropped, and so is a mission with no key.
pub fn parse(v: &Value, known: &HashSet<&str>) -> Vec<Proposal> {
    let mut out = Vec::new();
    for m in v.as_array().into_iter().flatten() {
        let mut keys: Vec<String> = Vec::new();
        for k in m["keys"].as_array().into_iter().flatten() {
            if let Some(k) = k.as_str().map(str::trim)
                && known.contains(k)
                && !keys.iter().any(|x| x == k)
            {
                keys.push(k.to_string());
            }
        }
        if keys.is_empty() {
            continue;
        }
        let title = one_line(m["title"].as_str().unwrap_or_default(), MAX_TITLE);
        if title.is_empty() {
            continue;
        }
        let steps = m["steps"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|s| s.as_str().map(|s| one_line(s, MAX_STEP)))
            .filter(|s| !s.is_empty())
            .take(MAX_STEPS)
            .collect();
        let doer = if m["doer"].as_str() == Some("aifred") {
            "aifred"
        } else {
            "you"
        };
        out.push(Proposal {
            id: m["id"].as_i64(),
            title,
            keys,
            steps,
            doer: doer.into(),
        });
        if out.len() == MAX_PROPOSALS {
            break;
        }
    }
    out
}

/// The open missions, for the AI (overview.rs): the id, the title, the
/// keys, and the steps. With them the brief can keep a mission's id.
pub(crate) fn for_ai(c: &Connection) -> anyhow::Result<Vec<Value>> {
    let mut out = Vec::new();
    for m in open_in(c)? {
        let mut v = json!({
            "id": m.id,
            "title": m.title,
            "keys": m.items.iter().map(|i| i.key.as_str()).collect::<Vec<_>>(),
        });
        if !m.checklist.is_empty() {
            v["steps"] = format!("{} of {} done", m.progress.steps_done, m.progress.steps).into();
        }
        // The brief cannot move these items (module text).
        let kept: Vec<&str> = m
            .items
            .iter()
            .filter(|i| i.open && i.by == "you")
            .map(|i| i.key.as_str())
            .collect();
        if !kept.is_empty() {
            v["added_by_user"] = kept.into();
        }
        let removed = out_of(c, m.id)?;
        if !removed.is_empty() {
            v["removed_by_user"] = removed.into();
        }
        if m.items.iter().any(|i| !i.open) {
            v["closed"] = m
                .items
                .iter()
                .filter(|i| !i.open)
                .map(|i| i.key.as_str())
                .collect::<Vec<_>>()
                .into();
        }
        out.push(v);
    }
    Ok(out)
}

/// Saves the missions of the brief of `day`, in rank order. See the module
/// text for the rules. `now` is in Unix seconds.
pub fn save_in(
    tx: &Transaction,
    day: &str,
    proposals: &[Proposal],
    now: u64,
) -> anyhow::Result<()> {
    let old: HashMap<i64, Vec<String>> = open_keys(tx)?;
    // The items that the user added to a mission stay in that mission.
    let mine: HashMap<String, i64> = {
        let mut q = tx.prepare(
            "SELECT i.key, i.mission FROM mission_item i JOIN mission m ON m.id = i.mission
             WHERE m.status = 'open' AND i.placed_by = 'you'",
        )?;
        q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?
    };
    // A new brief of the same day ranks again.
    tx.execute(
        "UPDATE mission SET rank = NULL WHERE day = ?1 AND status = 'open'",
        [day],
    )?;
    let mut claimed: HashSet<String> = HashSet::new();
    let mut used: HashSet<i64> = HashSet::new();
    let mut rank = 0;
    for p in proposals {
        if rank == MAX_TODAY {
            break;
        }
        let mut keys: Vec<String> = Vec::new();
        for k in &p.keys {
            // Only the items that the brief named: a mission is the next
            // small piece of work, not a full tree of subtasks.
            if claimed.contains(k) || keys.contains(k) || !crate::items::is_open(tx, k)? {
                continue;
            }
            keys.push(k.clone());
        }
        if keys.is_empty() {
            continue;
        }
        let given = p.id.filter(|id| !used.contains(id)).filter(|id| {
            old.get(id)
                .is_some_and(|o| o.iter().any(|k| keys.contains(k)))
        });
        // An item that the user added to a mission stays in it. So when the
        // brief names that item, the brief ranks that mission. Else the item
        // drops out of this proposal, and its mission does not show today.
        let users = keys
            .iter()
            .filter_map(|k| mine.get(k).copied())
            .find(|m| !used.contains(m));
        let pick = match (users, given) {
            (Some(m), _) => Some(m),
            (None, Some(id)) => Some(id),
            (None, None) => same_work(tx, &old, &used, &keys)?,
        };
        // The user's choices win over the AI's (module text).
        let out = match pick {
            Some(id) => out_of(tx, id)?,
            None => Vec::new(),
        };
        keys.retain(|k| mine.get(k).is_none_or(|m| Some(*m) == pick) && !out.contains(k));
        if keys.is_empty() {
            continue;
        }
        rank += 1;
        let id = match pick {
            Some(id) => {
                // The items that the user added stay. So do the closed
                // items, for the progress.
                for k in &old[&id] {
                    if !keys.contains(k)
                        && !claimed.contains(k)
                        && (mine.get(k) == Some(&id) || !crate::items::is_open(tx, k)?)
                        && crate::items::title_place(tx, k)?.is_some()
                    {
                        keys.push(k.clone());
                    }
                }
                tx.execute(
                    "UPDATE mission SET title = ?2, doer = ?3, day = ?4, rank = ?5, updated = ?6 WHERE id = ?1",
                    rusqlite::params![id, p.title, p.doer, day, rank, now as i64],
                )?;
                if !p.steps.is_empty() {
                    tx.execute(
                        "UPDATE mission SET checklist = ?2 WHERE id = ?1 AND checklist = '[]'",
                        rusqlite::params![id, steps_json(&p.steps)?],
                    )?;
                }
                id
            }
            None => {
                tx.execute(
                    "INSERT INTO mission (title, doer, checklist, day, rank, status, created, updated)
                     VALUES (?1, ?2, ?3, ?4, ?5, 'open', ?6, ?6)",
                    rusqlite::params![p.title, p.doer, steps_json(&p.steps)?, day, rank, now as i64],
                )?;
                tx.last_insert_rowid()
            }
        };
        used.insert(id);
        tx.execute("DELETE FROM mission_item WHERE mission = ?1", [id])?;
        for (pos, k) in keys.iter().enumerate() {
            // An item moves from its earlier mission to this one.
            tx.execute("DELETE FROM mission_item WHERE key = ?1", [k])?;
            let by = if mine.get(k) == Some(&id) {
                "you"
            } else {
                "aifred"
            };
            tx.execute(
                "INSERT INTO mission_item (key, mission, pos, placed_by) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![k, id, pos as i64, by],
            )?;
            claimed.insert(k.clone());
        }
    }
    // A mission with no items left goes away. So does a mission that this
    // brief did not rank and that did not change for STALE_DAYS.
    tx.execute(
        "DELETE FROM mission WHERE status = 'open' AND id NOT IN (SELECT mission FROM mission_item)",
        [],
    )?;
    tx.execute(
        "DELETE FROM mission WHERE status = 'open' AND day IS NOT ?1 AND updated < ?2",
        rusqlite::params![day, now.saturating_sub(STALE_DAYS * 86_400) as i64],
    )?;
    Ok(())
}

/// The earlier open mission that has half of its open items or more in
/// `keys`, and the most of them. None when no mission has.
fn same_work(
    c: &Connection,
    old: &HashMap<i64, Vec<String>>,
    used: &HashSet<i64>,
    keys: &[String],
) -> anyhow::Result<Option<i64>> {
    let mut best: Option<(usize, i64)> = None;
    for (id, ks) in old {
        if used.contains(id) {
            continue;
        }
        let mut open = 0;
        let mut shared = 0;
        for k in ks {
            if crate::items::is_open(c, k)? {
                open += 1;
                if keys.contains(k) {
                    shared += 1;
                }
            }
        }
        if shared > 0
            && shared * 2 >= open
            && best.is_none_or(|(n, b)| shared > n || (shared == n && *id < b))
        {
            best = Some((shared, *id));
        }
    }
    Ok(best.map(|(_, id)| id))
}

/// The items that the user took out of mission `id`.
fn out_of(c: &Connection, id: i64) -> anyhow::Result<Vec<String>> {
    let mut q = c.prepare_cached("SELECT key FROM mission_out WHERE mission = ?1 ORDER BY key")?;
    Ok(q.query_map([id], |r| r.get(0))?.collect::<Result<_, _>>()?)
}

fn steps_json(steps: &[String]) -> anyhow::Result<String> {
    let list: Vec<ChecklistItem> = steps
        .iter()
        .map(|s| ChecklistItem {
            text: s.clone(),
            done: false,
        })
        .collect();
    Ok(serde_json::to_string(&list)?)
}

/// The keys of each open mission, in order.
fn open_keys(c: &Connection) -> anyhow::Result<HashMap<i64, Vec<String>>> {
    let mut q = c.prepare(
        "SELECT i.mission, i.key FROM mission_item i JOIN mission m ON m.id = i.mission
         WHERE m.status = 'open' ORDER BY i.mission, i.pos",
    )?;
    let mut out: HashMap<i64, Vec<String>> = HashMap::new();
    for r in q.query_map([], |r| Ok((r.get::<_, i64>(0)?, r.get::<_, String>(1)?)))? {
        let (id, k) = r?;
        out.entry(id).or_default().push(k);
    }
    Ok(out)
}

const COLUMNS: &str = "id, title, doer, day, rank, status, checklist";

fn load(c: &Connection, r: &rusqlite::Row) -> anyhow::Result<Mission> {
    let id: i64 = r.get(0)?;
    let checklist: Vec<ChecklistItem> = serde_json::from_str(&r.get::<_, String>(6)?)?;
    let mut q = c.prepare_cached(
        "SELECT key, placed_by FROM mission_item WHERE mission = ?1 ORDER BY pos, rowid",
    )?;
    let mut items = Vec::new();
    for k in q.query_map([id], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })? {
        let (key, by) = k?;
        // An item that AIfred does not know now (for example a deleted
        // task) is not shown.
        let Some((title, _)) = crate::items::title_place(c, &key)? else {
            continue;
        };
        let open = crate::items::is_open(c, &key)?;
        items.push(MissionItem {
            key,
            title,
            open,
            by,
        });
    }
    let steps_done = checklist.iter().filter(|s| s.done).count();
    let items_closed = items.iter().filter(|i| !i.open).count();
    let all = checklist.len() + items.len();
    let percent = ((steps_done + items_closed) * 100)
        .checked_div(all)
        .unwrap_or(0) as u32;
    Ok(Mission {
        id,
        title: r.get(1)?,
        doer: r.get(2)?,
        day: r.get(3)?,
        rank: r.get(4)?,
        status: r.get(5)?,
        progress: Progress {
            steps_done,
            steps: checklist.len(),
            items_closed,
            items: items.len(),
            percent,
        },
        checklist,
        items,
    })
}

fn query(
    c: &Connection,
    filter: &str,
    params: impl rusqlite::Params,
) -> anyhow::Result<Vec<Mission>> {
    let mut q = c.prepare(&format!("SELECT {COLUMNS} FROM mission WHERE {filter}"))?;
    let mut rows = q.query(params)?;
    let mut out = Vec::new();
    while let Some(r) = rows.next()? {
        out.push(load(c, r)?);
    }
    Ok(out)
}

fn open_in(c: &Connection) -> anyhow::Result<Vec<Mission>> {
    query(c, "status = 'open' ORDER BY id", [])
}

fn get_in(c: &Connection, id: i64) -> anyhow::Result<Mission> {
    query(c, "id = ?1", [id])?
        .pop()
        .ok_or_else(|| anyhow::anyhow!("AIfred does not know the mission {id}"))
}

/// One mission, open or done.
pub fn get(dir: &Path, id: i64) -> anyhow::Result<Mission> {
    db::read(dir, |c| get_in(c, id))
}

/// The text about one mission that goes into a message of its chat: the
/// mission with its items, its checklist and its progress, and the user's
/// notes about each item (context.rs). It has no item description: other
/// people write those, so the AI reads them with its tools, as in the main chat.
pub fn chat_context(dir: &Path, id: i64) -> anyhow::Result<String> {
    let m = get(dir, id)?;
    let mut notes = String::new();
    for i in &m.items {
        if let Some(n) = crate::context::get(dir, &i.key)? {
            notes.push_str(&format!(
                "- {} ({}): {}\n",
                i.key,
                crate::context::notes_label(&n.by),
                one_line(&n.text, 600)
            ));
        }
    }
    let notes = if notes.is_empty() {
        String::new()
    } else {
        format!(
            "[Task context: your notes about the items. Each line says where the notes come from.]\n{notes}"
        )
    };
    Ok(format!("{}\n{notes}", json!(m)))
}

/// The missions of the brief of `day`, in rank order. A mission with an
/// item that the user corrected on that day (learn.rs) is not in the list.
pub fn today(dir: &Path, day: &str) -> anyhow::Result<Vec<Mission>> {
    crate::plan::check_day(day)?;
    db::read(dir, |c| today_in(c, day))
}

/// today(), with a connection that is open.
pub(crate) fn today_in(c: &Connection, day: &str) -> anyhow::Result<Vec<Mission>> {
    let corrected = crate::learn::corrected_on(c, day)?;
    Ok(query(
        c,
        "status = 'open' AND day = ?1 AND rank IS NOT NULL ORDER BY rank",
        [day],
    )?
    .into_iter()
    // A mission whose items AIfred does not know now (for example after a
    // disconnect of ClickUp) shows nothing useful.
    .filter(|m| !m.items.is_empty() && !m.items.iter().any(|i| corrected.contains(&i.key)))
    .collect())
}

/// Ticks step `index` of the mission's checklist, or takes the tick away.
/// The items do not change.
pub fn set_step(
    dir: &Path,
    id: i64,
    index: usize,
    done: bool,
    now: u64,
) -> anyhow::Result<Mission> {
    db::write(dir, |tx| {
        let raw: Option<String> = tx
            .query_row(
                "SELECT checklist FROM mission WHERE id = ?1 AND status = 'open'",
                [id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(raw) = raw else {
            bail!("AIfred does not know the open mission {id}");
        };
        let mut list: Vec<ChecklistItem> = serde_json::from_str(&raw)?;
        let Some(step) = list.get_mut(index) else {
            bail!("the mission has no step {}", index + 1);
        };
        step.done = done;
        tx.execute(
            "UPDATE mission SET checklist = ?2, updated = ?3 WHERE id = ?1",
            rusqlite::params![id, serde_json::to_string(&list)?, now as i64],
        )?;
        get_in(tx, id)
    })
}

/// The mission is done. The caller closes the items that the user picked
/// first. The items, the rank, and the checklist stay, so reopen() can undo
/// this. The items are free all the same: save_in() and the other reads use
/// only open missions, and save_in() moves an item to its new mission.
pub fn finish(dir: &Path, id: i64, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        let n = tx.execute(
            "UPDATE mission SET status = 'done', updated = ?2 WHERE id = ?1 AND status = 'open'",
            rusqlite::params![id, now as i64],
        )?;
        if n == 0 {
            bail!("AIfred does not know the open mission {id}");
        }
        Ok(())
    })
}

/// The undo of finish(): the mission is open again, with its checklist.
/// It does not get back an item that a later brief put in another mission.
pub fn reopen(dir: &Path, id: i64, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        let n = tx.execute(
            "UPDATE mission SET status = 'open', updated = ?2 WHERE id = ?1 AND status = 'done'",
            rusqlite::params![id, now as i64],
        )?;
        if n == 0 {
            bail!("AIfred does not know the done mission {id}");
        }
        Ok(())
    })
}

/// The user says that the brief of `day` is wrong about the mission: one
/// correction (learn.rs) of each open item, all in one write. So either all
/// corrections are saved or none is. A mission with no open item is an
/// error: a correction of a closed item hides nothing.
pub fn correct(
    dir: &Path,
    id: i64,
    kind: &str,
    day: &str,
    now: u64,
) -> anyhow::Result<Vec<crate::learn::Corrected>> {
    db::write(dir, |tx| {
        check_open(tx, id)?;
        let m = get_in(tx, id)?;
        let open: Vec<&MissionItem> = m.items.iter().filter(|i| i.open).collect();
        if open.is_empty() {
            bail!("All items of this mission are closed. Finish the mission instead");
        }
        open.iter()
            .map(|i| crate::learn::add_in(tx, &i.key, kind, day, now))
            .collect()
    })
}

/// Where an item was in a mission.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub mission: i64,
    pub pos: i64,
    /// "you" or "aifred".
    pub by: String,
}

/// What restore() needs to undo add_item() or remove_item().
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Before {
    pub key: String,
    /// The mission that the change was about.
    pub mission: i64,
    /// Where the item was before the change. None: in no open mission.
    pub spot: Option<Spot>,
    /// True when the user had taken the item out of `mission`.
    pub out: bool,
}

/// The result of a change of a mission's items.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Changed {
    pub mission: Mission,
    pub before: Before,
}

/// Where `key` is now, in an open mission.
fn spot_in(c: &Connection, key: &str) -> anyhow::Result<Option<Spot>> {
    Ok(c.query_row(
        "SELECT i.mission, i.pos, i.placed_by FROM mission_item i JOIN mission m ON m.id = i.mission
         WHERE i.key = ?1 AND m.status = 'open'",
        [key],
        |r| {
            Ok(Spot {
                mission: r.get(0)?,
                pos: r.get(1)?,
                by: r.get(2)?,
            })
        },
    )
    .optional()?)
}

fn is_out(c: &Connection, id: i64, key: &str) -> anyhow::Result<bool> {
    Ok(c.query_row(
        "SELECT 1 FROM mission_out WHERE mission = ?1 AND key = ?2",
        rusqlite::params![id, key],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

fn check_open(c: &Connection, id: i64) -> anyhow::Result<()> {
    if !is_open_in(c, id)? {
        bail!("AIfred does not know the open mission {id}");
    }
    Ok(())
}

/// True when mission `id` is open. A database error is an error, not "no".
fn is_open_in(c: &Connection, id: i64) -> anyhow::Result<bool> {
    Ok(c.query_row(
        "SELECT 1 FROM mission WHERE id = ?1 AND status = 'open'",
        [id],
        |_| Ok(()),
    )
    .optional()?
    .is_some())
}

/// The user adds the open item `key` to mission `id`, at the end. The item
/// leaves its other mission. The brief cannot take it away (module text).
pub fn add_item(dir: &Path, id: i64, key: &str, now: u64) -> anyhow::Result<Changed> {
    db::write(dir, |tx| {
        check_open(tx, id)?;
        if crate::items::title_place(tx, key)?.is_none() || !crate::items::is_open(tx, key)? {
            bail!("AIfred does not know the open item {key}");
        }
        let spot = spot_in(tx, key)?;
        if spot.as_ref().is_some_and(|s| s.mission == id) {
            bail!("The item is in this mission already");
        }
        let before = Before {
            key: key.into(),
            mission: id,
            spot,
            out: is_out(tx, id, key)?,
        };
        // The earlier mission stays, also with no items, so the undo can put
        // the item back. The next brief removes an empty mission.
        tx.execute("DELETE FROM mission_item WHERE key = ?1", [key])?;
        tx.execute(
            "DELETE FROM mission_out WHERE mission = ?1 AND key = ?2",
            rusqlite::params![id, key],
        )?;
        tx.execute(
            "INSERT INTO mission_item (key, mission, pos, placed_by)
             VALUES (?1, ?2, (SELECT COALESCE(MAX(pos), -1) + 1 FROM mission_item WHERE mission = ?2), 'you')",
            rusqlite::params![key, id],
        )?;
        tx.execute(
            "UPDATE mission SET updated = ?2 WHERE id = ?1",
            rusqlite::params![id, now as i64],
        )?;
        Ok(Changed {
            mission: get_in(tx, id)?,
            before,
        })
    })
}

/// The user takes `key` out of mission `id`. The brief never puts it back
/// in that mission. At least one open item must stay, so a mission does not
/// go away: for that, the user has "Done" and "Not today".
pub fn remove_item(dir: &Path, id: i64, key: &str, now: u64) -> anyhow::Result<Changed> {
    db::write(dir, |tx| {
        check_open(tx, id)?;
        let spot = spot_in(tx, key)?.filter(|s| s.mission == id);
        let Some(spot) = spot else {
            bail!("The item is not in this mission");
        };
        let m = get_in(tx, id)?;
        if !m.items.iter().any(|i| i.open && i.key != key) {
            bail!("This is the last open item of the mission. Use Done or Not today instead");
        }
        tx.execute("DELETE FROM mission_item WHERE key = ?1", [key])?;
        tx.execute(
            "INSERT OR REPLACE INTO mission_out (mission, key, ts) VALUES (?1, ?2, ?3)",
            rusqlite::params![id, key, now as i64],
        )?;
        tx.execute(
            "UPDATE mission SET updated = ?2 WHERE id = ?1",
            rusqlite::params![id, now as i64],
        )?;
        Ok(Changed {
            mission: get_in(tx, id)?,
            before: Before {
                key: key.into(),
                mission: id,
                spot: Some(spot),
                out: false,
            },
        })
    })
}

/// The undo of add_item() and remove_item(): the item goes back to where it
/// was. When its earlier mission is gone or done, the item is in no mission.
///
/// The undo fails when the item moved again after the change, for example
/// when a brief put it in a new mission. The undo must not take it from there.
pub fn restore(dir: &Path, before: &Before, now: u64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        // remove_item() is the only change where the earlier spot is in
        // `before.mission`: add_item() refuses an item in that mission.
        let removed = before
            .spot
            .as_ref()
            .is_some_and(|s| s.mission == before.mission);
        // After remove_item() the item is in no open mission. After
        // add_item() it is in `before.mission`.
        let moved = match spot_in(tx, &before.key)? {
            Some(_) if removed => true,
            Some(s) => s.mission != before.mission,
            None => false,
        };
        if moved {
            bail!("The item moved to another mission after this change, so AIfred cannot undo it");
        }
        tx.execute("DELETE FROM mission_item WHERE key = ?1", [&before.key])?;
        if let Some(s) = &before.spot
            && is_open_in(tx, s.mission)?
        {
            let by = if s.by == "you" { "you" } else { "aifred" };
            tx.execute(
                "INSERT INTO mission_item (key, mission, pos, placed_by) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![before.key, s.mission, s.pos, by],
            )?;
        }
        if before.out && is_open_in(tx, before.mission)? {
            tx.execute(
                "INSERT OR REPLACE INTO mission_out (mission, key, ts) VALUES (?1, ?2, ?3)",
                rusqlite::params![before.mission, before.key, now as i64],
            )?;
        } else {
            tx.execute(
                "DELETE FROM mission_out WHERE mission = ?1 AND key = ?2",
                rusqlite::params![before.mission, before.key],
            )?;
        }
        tx.execute(
            "UPDATE mission SET updated = ?2 WHERE id IN (?1, ?3)",
            rusqlite::params![
                before.mission,
                now as i64,
                before.spot.as_ref().map(|s| s.mission)
            ],
        )?;
        Ok(())
    })
}

/// Takes an item that is gone out of its mission (tasks.rs). That mission
/// goes away when it is open and has no items left. Other empty missions
/// stay: add_item() can empty a mission on purpose, so its undo works.
pub(crate) fn forget_in(tx: &Transaction, key: &str) -> anyhow::Result<()> {
    let Some(id) = tx
        .query_row(
            "SELECT mission FROM mission_item WHERE key = ?1",
            [key],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(());
    };
    tx.execute("DELETE FROM mission_item WHERE key = ?1", [key])?;
    tx.execute(
        "DELETE FROM mission WHERE id = ?1 AND status = 'open'
           AND NOT EXISTS (SELECT 1 FROM mission_item WHERE mission = ?1)",
        [id],
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-mission-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn p(id: Option<i64>, title: &str, keys: &[&str], steps: &[&str]) -> Proposal {
        Proposal {
            id,
            title: title.into(),
            keys: keys.iter().map(|k| k.to_string()).collect(),
            steps: steps.iter().map(|s| s.to_string()).collect(),
            doer: "you".into(),
        }
    }

    fn save(d: &Path, day: &str, list: &[Proposal], now: u64) {
        db::write(d, |tx| save_in(tx, day, list, now)).unwrap();
    }

    fn keys(m: &Mission) -> Vec<&str> {
        m.items.iter().map(|i| i.key.as_str()).collect()
    }

    /// Three open tasks: their keys.
    fn three(d: &Path) -> (String, String, String) {
        let s = TaskStore::new(d);
        let k = |t: &str| format!("task:{}", s.add(t, "", "you").unwrap().id);
        (k("A"), k("B"), k("C"))
    }

    #[test]
    fn the_answer_gives_missions_with_known_keys_only() {
        let known: HashSet<&str> = ["task:a", "task:b"].into();
        let v = json!([
            {"id": 3, "title": " Ship  the fix ", "keys": ["task:a", "task:nope", "task:a"], "steps": ["Do A", " ", 5], "doer": "aifred"},
            {"title": "No known key", "keys": ["task:nope"]},
            {"title": "", "keys": ["task:b"]},
            {"title": "B", "keys": ["task:b"], "doer": "robot"}
        ]);
        let got = parse(&v, &known);
        assert_eq!(
            got,
            vec![
                Proposal {
                    id: Some(3),
                    title: "Ship the fix".into(),
                    keys: vec!["task:a".into()],
                    steps: vec!["Do A".into()],
                    doer: "aifred".into(),
                },
                p(None, "B", &["task:b"], &[]),
            ]
        );
        let many: Vec<Value> = (0..10)
            .map(|_| json!({"title": "A", "keys": ["task:a"]}))
            .collect();
        assert_eq!(parse(&json!(many), &known).len(), 10);
        let more: Vec<Value> = (0..50)
            .map(|_| json!({"title": "A", "keys": ["task:a"]}))
            .collect();
        assert_eq!(parse(&json!(more), &known).len(), MAX_PROPOSALS);
        assert!(parse(&Value::Null, &known).is_empty());
    }

    #[test]
    fn a_mission_keeps_its_id_and_its_steps_when_its_items_stay() {
        let d = temp("keep");
        let (a, b, c) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[p(None, "Ship", &[&a, &b], &["One", "Two"])],
            100,
        );
        let m = today(&d, "2026-11-08").unwrap();
        assert_eq!(m.len(), 1);
        assert_eq!(keys(&m[0]), [a.as_str(), b.as_str()]);
        let id = m[0].id;
        set_step(&d, id, 0, true, 101).unwrap();

        // The next day, with no id from the AI: half of the items stay.
        save(
            &d,
            "2026-11-09",
            &[p(None, "Ship it", &[&b, &c], &["New"])],
            200,
        );
        let m = today(&d, "2026-11-09").unwrap();
        assert_eq!(m[0].id, id, "half of the open items stay: the same mission");
        assert_eq!(m[0].title, "Ship it");
        assert_eq!(m[0].checklist.len(), 2, "the steps stay");
        assert!(m[0].checklist[0].done);
        assert!(
            today(&d, "2026-11-08").unwrap().is_empty(),
            "only the last brief ranks it"
        );

        // An id from the AI with no shared item is not trusted.
        save(&d, "2026-11-10", &[p(Some(id), "Other", &[&a], &[])], 300);
        let m = today(&d, "2026-11-10").unwrap();
        assert_ne!(m[0].id, id);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_is_in_one_mission_and_its_subtasks_do_not_come_with_it() {
        let d = temp("one");
        let (a, b, c) = three(&d);
        crate::links::link(&d, &b, &c, 1).unwrap();
        save(
            &d,
            "2026-11-08",
            &[
                p(None, "First", &[&a, &b], &[]),
                p(None, "Second", &[&a, &c], &[]),
            ],
            100,
        );
        let m = today(&d, "2026-11-08").unwrap();
        assert_eq!(m.len(), 2);
        assert_eq!(
            keys(&m[0]),
            [a.as_str(), b.as_str()],
            "c is under b, but the brief did not name it"
        );
        assert_eq!(
            keys(&m[1]),
            [c.as_str()],
            "a is in the first mission already"
        );
        assert_eq!(m[0].rank, Some(1));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn progress_counts_steps_and_closed_items_and_closed_items_stay() {
        let d = temp("progress");
        let (a, b, _) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[p(None, "Ship", &[&a, &b], &["One", "Two"])],
            100,
        );
        let id = today(&d, "2026-11-08").unwrap()[0].id;
        set_step(&d, id, 1, true, 101).unwrap();
        TaskStore::new(&d)
            .set_done(a.strip_prefix("task:").unwrap(), true)
            .unwrap();
        let m = &today(&d, "2026-11-08").unwrap()[0];
        assert_eq!(
            m.progress,
            Progress {
                steps_done: 1,
                steps: 2,
                items_closed: 1,
                items: 2,
                percent: 50
            }
        );
        assert!(set_step(&d, id, 5, true, 102).is_err());

        // The next brief knows only b. The closed item a stays for the progress.
        save(&d, "2026-11-09", &[p(Some(id), "Ship", &[&b], &[])], 200);
        let m = &today(&d, "2026-11-09").unwrap()[0];
        assert_eq!(m.id, id);
        assert_eq!(keys(m), [b.as_str(), a.as_str()]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_finished_mission_lets_its_items_go() {
        let d = temp("finish");
        let (a, b, _) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[p(None, "Ship", &[&a, &b], &["One"])],
            100,
        );
        let id = today(&d, "2026-11-08").unwrap()[0].id;
        set_step(&d, id, 0, true, 100).unwrap();
        finish(&d, id, 101).unwrap();
        assert!(today(&d, "2026-11-08").unwrap().is_empty());
        assert!(finish(&d, id, 102).is_err(), "it is done already");

        // The undo gives back the mission with its items and its checklist.
        reopen(&d, id, 103).unwrap();
        assert!(reopen(&d, id, 104).is_err(), "it is open already");
        let m = today(&d, "2026-11-08").unwrap();
        assert_eq!(m[0].id, id);
        assert_eq!(keys(&m[0]), [a.as_str(), b.as_str()]);
        assert!(m[0].checklist[0].done);

        finish(&d, id, 105).unwrap();
        save(&d, "2026-11-08", &[p(Some(id), "Ship", &[&a], &[])], 106);
        let m = today(&d, "2026-11-08").unwrap();
        assert_ne!(m[0].id, id, "a done mission does not open again");
        assert_eq!(keys(&m[0]), [a.as_str()], "its items are free");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_mission_correction_saves_all_or_nothing() {
        let d = temp("correct-all");
        let (a, b, _) = three(&d);
        save(&d, "2026-11-08", &[p(None, "Ship", &[&a, &b], &[])], 100);
        let id = today(&d, "2026-11-08").unwrap()[0].id;
        assert!(correct(&d, id, "nope", "2026-11-08", 101).is_err());
        let n: i64 = db::read(&d, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM correction", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(n, 0, "a wrong kind saves nothing");
        let got = correct(&d, id, crate::learn::NOT_TODAY, "2026-11-08", 102).unwrap();
        assert_eq!(got.len(), 2);

        // A mission with only closed items cannot be corrected.
        let s = TaskStore::new(&d);
        for k in [&a, &b] {
            s.set_done(k.strip_prefix("task:").unwrap(), true).unwrap();
        }
        assert!(correct(&d, id, crate::learn::NOT_TODAY, "2026-11-09", 103).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_done_mission_cannot_be_corrected() {
        let d = temp("correct-done");
        let (a, _, _) = three(&d);
        save(&d, "2026-11-08", &[p(None, "Ship", &[&a], &[])], 100);
        let id = today(&d, "2026-11-08").unwrap()[0].id;
        finish(&d, id, 101).unwrap();
        assert!(correct(&d, id, crate::learn::NOT_TODAY, "2026-11-08", 102).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn undo_does_not_take_an_item_from_a_newer_mission() {
        let d = temp("restore-moved");
        let (a, b, _) = three(&d);
        save(&d, "2026-11-08", &[p(None, "First", &[&a, &b], &[])], 100);
        let first = today(&d, "2026-11-08").unwrap()[0].id;
        let c = remove_item(&d, first, &b, 101).unwrap();
        // A brief gives b a mission of its own before the user clicks Undo.
        save(
            &d,
            "2026-11-08",
            &[
                p(Some(first), "First", &[&a], &[]),
                p(None, "B", &[&b], &[]),
            ],
            102,
        );
        assert!(restore(&d, &c.before, 103).is_err());
        let m = today(&d, "2026-11-08").unwrap();
        assert_eq!(keys(&m[1]), [b.as_str()], "b stays in its new mission");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_dropped_proposal_does_not_use_a_place() {
        let d = temp("fill");
        let (a, _, _) = three(&d);
        let s = TaskStore::new(&d);
        let mut ps = vec![p(None, "Old", &[&a], &[])];
        let mut extra = Vec::new();
        for n in 0..MAX_TODAY {
            extra.push(format!(
                "task:{}",
                s.add(&format!("T{n}"), "", "you").unwrap().id
            ));
        }
        // a is closed, so the first proposal drops out.
        s.set_done(a.strip_prefix("task:").unwrap(), true).unwrap();
        for (n, k) in extra.iter().enumerate() {
            ps.push(p(None, &format!("M{n}"), &[k], &[]));
        }
        save(&d, "2026-11-08", &ps, 100);
        assert_eq!(today(&d, "2026-11-08").unwrap().len(), MAX_TODAY);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_correction_hides_the_mission_on_that_day() {
        let d = temp("correct");
        let (a, b, _) = three(&d);
        save(&d, "2026-11-08", &[p(None, "Ship", &[&a, &b], &[])], 100);
        crate::learn::add(&d, &b, crate::learn::NOT_TODAY, "2026-11-08", 101).unwrap();
        assert!(today(&d, "2026-11-08").unwrap().is_empty());
        // A mission with only items that AIfred does not know is not shown.
        let (c, _, _) = three(&d);
        save(&d, "2026-11-09", &[p(None, "Gone", &[&c], &[])], 200);
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE task SET id = 'other' WHERE id = ?1",
                [c.strip_prefix("task:").unwrap()],
            )?;
            Ok(())
        })
        .unwrap();
        assert!(today(&d, "2026-11-09").unwrap().is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_brief_keeps_the_items_that_the_user_added_or_took_out() {
        let d = temp("user");
        let (a, b, c) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[
                p(None, "First", &[&a, &b], &[]),
                p(None, "Second", &[&c], &[]),
            ],
            100,
        );
        let m = today(&d, "2026-11-08").unwrap();
        let (first, second) = (m[0].id, m[1].id);

        // The user takes b out of the first mission and adds c to it.
        remove_item(&d, first, &b, 101).unwrap();
        let got = add_item(&d, first, &c, 102).unwrap();
        assert_eq!(keys(&got.mission), [a.as_str(), c.as_str()]);
        assert_eq!(got.mission.items[1].by, "you");
        assert!(add_item(&d, first, &c, 103).is_err(), "c is in it already");

        // The next brief wants b back in the first mission and c alone.
        save(
            &d,
            "2026-11-09",
            &[
                p(Some(first), "First", &[&a, &b], &[]),
                p(None, "C", &[&c], &[]),
            ],
            200,
        );
        let m = today(&d, "2026-11-09").unwrap();
        assert_eq!(
            m.len(),
            1,
            "c stays where the user put it, so no second mission"
        );
        assert_eq!(m[0].id, first);
        assert_eq!(
            keys(&m[0]),
            [a.as_str(), c.as_str()],
            "b does not come back"
        );
        let ai = db::read(&d, for_ai).unwrap();
        let v = ai.iter().find(|v| v["id"] == first).unwrap();
        assert_eq!(v["added_by_user"], json!([c]));
        assert_eq!(v["removed_by_user"], json!([b]));
        assert!(
            db::read(&d, |c| get_in(c, second)).is_err(),
            "the second mission had no items left"
        );

        // b can have its own mission. Only the user puts it back in the first.
        save(&d, "2026-11-10", &[p(None, "B", &[&b], &[])], 300);
        add_item(&d, first, &b, 301).unwrap();
        let m = db::read(&d, |c| get_in(c, first)).unwrap();
        assert_eq!(keys(&m), [a.as_str(), c.as_str(), b.as_str()]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_that_the_user_added_brings_its_mission_to_today() {
        let d = temp("bring");
        let (a, b, c) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[p(None, "First", &[&a], &[]), p(None, "Second", &[&b], &[])],
            100,
        );
        let first = today(&d, "2026-11-08").unwrap()[0].id;
        add_item(&d, first, &c, 101).unwrap();
        // The next brief names c alone, in a new mission.
        save(&d, "2026-11-09", &[p(None, "C", &[&c], &[])], 200);
        let m = today(&d, "2026-11-09").unwrap();
        assert_eq!(m.len(), 1, "{m:?}");
        assert_eq!(
            m[0].id, first,
            "c shows in the mission where the user put it"
        );
        assert!(keys(&m[0]).contains(&c.as_str()));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_deleted_item_does_not_remove_a_mission_that_an_undo_needs() {
        let d = temp("forget");
        let (a, b, c) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[
                p(None, "First", &[&a], &[]),
                p(None, "Second", &[&b], &[]),
                p(None, "Third", &[&c], &[]),
            ],
            100,
        );
        let m = today(&d, "2026-11-08").unwrap();
        let (first, second, third) = (m[0].id, m[1].id, m[2].id);
        // The user moves a, the only item of the first mission, to the second.
        let moved = add_item(&d, second, &a, 101).unwrap();
        // Another item is deleted before the user clicks Undo.
        db::write(&d, |tx| forget_in(tx, &c)).unwrap();
        assert!(
            db::read(&d, |x| get_in(x, third)).is_err(),
            "the mission of the deleted item had no items left"
        );
        restore(&d, &moved.before, 102).unwrap();
        let m = db::read(&d, |x| get_in(x, first)).unwrap();
        assert_eq!(keys(&m), [a.as_str()], "the undo puts a back");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_undo_puts_an_added_or_removed_item_back() {
        let d = temp("undo");
        let (a, b, c) = three(&d);
        save(
            &d,
            "2026-11-08",
            &[
                p(None, "First", &[&a, &b], &[]),
                p(None, "Second", &[&c], &[]),
            ],
            100,
        );
        let m = today(&d, "2026-11-08").unwrap();
        let (first, second) = (m[0].id, m[1].id);

        // The undo of a remove: b is back in its place, and not "taken out".
        let r = remove_item(&d, first, &a, 101).unwrap();
        restore(&d, &r.before, 102).unwrap();
        let m = db::read(&d, |c| get_in(c, first)).unwrap();
        assert_eq!(keys(&m), [a.as_str(), b.as_str()]);
        assert!(db::read(&d, |c| out_of(c, first)).unwrap().is_empty());

        // The undo of an add: c goes back to the second mission, which had
        // no items for a moment.
        let r = add_item(&d, first, &c, 103).unwrap();
        assert!(
            today(&d, "2026-11-08")
                .unwrap()
                .iter()
                .all(|m| m.id != second)
        );
        restore(&d, &r.before, 104).unwrap();
        let m = today(&d, "2026-11-08").unwrap();
        assert_eq!(keys(&m[0]), [a.as_str(), b.as_str()]);
        assert_eq!((m[1].id, keys(&m[1])), (second, vec![c.as_str()]));

        // The undo of an add of an item that the user took out: it is out again.
        remove_item(&d, first, &b, 105).unwrap();
        let r = add_item(&d, first, &b, 106).unwrap();
        restore(&d, &r.before, 107).unwrap();
        assert_eq!(db::read(&d, |c| out_of(c, first)).unwrap(), vec![b.clone()]);

        // The last open item cannot go out.
        assert!(remove_item(&d, first, &a, 108).is_err());
        assert!(remove_item(&d, second, &a, 109).is_err(), "a is not in it");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_mission_with_no_items_or_no_rank_for_long_goes_away() {
        let d = temp("stale");
        let (a, b, _) = three(&d);
        save(
            &d,
            "2026-11-01",
            &[p(None, "Old", &[&a], &[]), p(None, "B", &[&b], &[])],
            100,
        );
        // b moves to a new mission; "B" has no items left. "Old" is not ranked.
        let later = 100 + (STALE_DAYS + 1) * 86_400;
        save(&d, "2026-11-20", &[p(None, "New", &[&b], &[])], later);
        let n: i64 = db::read(&d, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM mission", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(n, 1, "only the new mission stays");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
