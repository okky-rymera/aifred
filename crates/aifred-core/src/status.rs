//! AIfred's own status of each item. It is the same for local tasks,
//! ClickUp tasks, and GitHub items. A ClickUp status name can mean another
//! thing in each list, so AIfred does not show it as its own status.
//!
//! The statuses (ALL): To do, In progress, Waiting (open), and Done,
//! Unfinished, Cancelled (closed). Unfinished: the team worked on it but did
//! not finish it in its time. It can link to the item that continues it
//! (status_next), for example the copy of a rock in the next quarter. With
//! no link, the user accepts it as it is. Cancelled: the user decided not to
//! do it. Unfinished and Cancelled are not finished work: AIfred keeps no
//! work time for them, and no "task done" job starts.
//!
//! The status is only in AIfred. Only "mark done" changes ClickUp, as before.
//!
//! Which status wins, first match:
//! 1. The status that the user picked (item_status, or a weekly review
//!    "drop", the newer one of the two). For an item that left the list,
//!    only a closed pick counts, and not one from before a close that
//!    AIfred knows (2): the newer fact wins. So "mark done" in ClickUp
//!    wins over an older Cancelled pick.
//! 2. How the item closed in ClickUp or GitHub (close_reason), for an item
//!    that left the list. An item that is still open there (STILL_OPEN,
//!    for example an issue that is not the user's any more) reads To do,
//!    and AIfred reads it again now and then, because it can close later.
//! 3. The user's rule for the ClickUp status (status_rule).
//! 4. Waiting: the user gave the item away in a weekly review, or it has an
//!    open blocker (blockers.rs).
//! 5. In progress: the item has work time (worktime.rs). It stays In
//!    progress after the timer stops, until it closes.
//! 6. The default for the ClickUp status group (default_for), or To do.
//!
//! A pick that gives the item the status it has from 3 to 6 anyway is not
//! saved (set): the status goes on to follow the rule, the blocker, or the
//! timer. open_again() takes a pick away for the same reason.
//!
//! `work_item.status` keeps 1 and 3 for the items in the list, so the open
//! check of a downloaded item is one SQL test (items::is_open). refresh_in()
//! writes it again after each download and each rule change, and
//! refresh_one_in() after a change of one pick. 4 and 5 are added on read:
//! all_in() reads each table once, of_in() reads the rows of one item.
//!
//! A local task keeps a closed status in `task.done` and `task.closed_as`,
//! and an open status that the user picked in item_status.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;

pub const TODO: &str = "todo";
pub const PROGRESS: &str = "progress";
pub const WAITING: &str = "waiting";
pub const DONE: &str = "done";
pub const UNFINISHED: &str = "unfinished";
pub const CANCELLED: &str = "cancelled";

/// All statuses, in the order that the app shows them.
pub const ALL: [&str; 6] = [TODO, PROGRESS, WAITING, DONE, UNFINISHED, CANCELLED];

/// The closed statuses, for SQL. Change it together with is_closed().
pub(crate) const CLOSED_SQL: &str = "('done', 'unfinished', 'cancelled')";

/// Who or what set a status.
pub mod by {
    /// The user picked it.
    pub const YOU: &str = "you";
    /// How the item closed in ClickUp or GitHub.
    pub const SOURCE: &str = "source";
    /// The user's rule for the ClickUp status.
    pub const RULE: &str = "rule";
    /// The user gave the item away in a weekly review.
    pub const DELEGATE: &str = "delegate";
    /// The item waits for another item.
    pub const BLOCKER: &str = "blocker";
    /// The user started the work timer on the item.
    pub const TIMER: &str = "timer";
    /// The default: the ClickUp status group, or To do.
    pub const DEFAULT: &str = "default";
}

pub fn is_closed(status: &str) -> bool {
    matches!(status, DONE | UNFINISHED | CANCELLED)
}

/// The status, or an error that names the six statuses.
pub fn check(status: &str) -> anyhow::Result<&'static str> {
    let s = status.trim().to_lowercase();
    ALL.iter()
        .find(|a| **a == s)
        .copied()
        .ok_or_else(|| anyhow::anyhow!("a status is one of {}, not \"{status}\"", ALL.join(", ")))
}

/// An item that is linked to another one, with its title.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Linked {
    pub key: String,
    /// The title, when AIfred knows the item.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

/// The status of one item.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ItemStatus {
    pub status: String,
    /// Who or what set it (the names in `by`).
    pub by: String,
    /// The status name in ClickUp or GitHub, when there is one. For
    /// example "delayed", or "not planned" for a closed GitHub issue.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// Unfinished: the item that continues it. None: accepted as it is.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next: Option<Linked>,
    /// The Unfinished items that this item continues.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub continues: Vec<Linked>,
}

impl ItemStatus {
    pub(crate) fn new(
        status: impl Into<String>,
        by: impl Into<String>,
        source: Option<String>,
    ) -> Self {
        Self {
            status: status.into(),
            by: by.into(),
            source,
            next: None,
            continues: Vec::new(),
        }
    }
}

// ----- ClickUp rules -----

/// The scopes of a rule, the narrowest first. The app makes only "list"
/// and "space" rules. "folder" and "all" still work.
pub const SCOPES: [&str; 4] = ["list", "folder", "space", "all"];

/// The user's rule for a ClickUp status: "this status in this list (or in
/// all lists of this space) is this AIfred status".
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Rule {
    #[serde(default)]
    pub id: i64,
    /// The status name, in lower case.
    pub name: String,
    /// "list", "folder", "space", or "all".
    pub scope: String,
    /// The ClickUp id of the list, folder, or space. "" for "all".
    #[serde(default)]
    pub scope_id: String,
    /// The name of the list, folder, or space, for the Settings page.
    #[serde(default)]
    pub scope_name: String,
    /// The space of the list (or the space itself), for the Settings page.
    #[serde(default)]
    pub space_id: String,
    #[serde(default)]
    pub space_name: String,
    /// The ClickUp group of the status: "open", "unstarted", "custom",
    /// "done", or "closed". "" when not known.
    #[serde(default)]
    pub kind: String,
    pub status: String,
}

/// Where a ClickUp status is: the parts that a rule can match.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Place<'a> {
    pub list_id: &'a str,
    pub folder_id: &'a str,
    pub space_id: &'a str,
}

fn norm(name: &str) -> String {
    name.trim().to_lowercase()
}

/// The narrowest rule for the status `name` at `place`.
pub fn rule_for<'r>(rules: &'r [Rule], name: &str, place: &Place) -> Option<&'r Rule> {
    let name = norm(name);
    SCOPES.iter().find_map(|scope| {
        let id = match *scope {
            "list" => place.list_id,
            "folder" => place.folder_id,
            "space" => place.space_id,
            _ => "",
        };
        if *scope != "all" && id.is_empty() {
            return None;
        }
        rules
            .iter()
            .find(|r| r.name == name && r.scope == *scope && r.scope_id == id)
    })
}

/// The default for a ClickUp status group. ClickUp shows 4 groups:
/// Not started ("open" for the first status, "unstarted" for the others),
/// Active ("custom"), Done ("done"), and Closed ("closed").
pub fn default_for(kind: &str) -> &'static str {
    match kind {
        "custom" | "active" => PROGRESS,
        "done" | "closed" => DONE,
        _ => TODO,
    }
}

/// The status of a ClickUp status: the user's rule, else the group default.
pub fn map_clickup(
    rules: &[Rule],
    name: &str,
    kind: &str,
    place: &Place,
) -> (&'static str, &'static str) {
    if let Some(r) = rule_for(rules, name, place)
        && let Ok(s) = check(&r.status)
    {
        return (s, by::RULE);
    }
    (default_for(kind), by::DEFAULT)
}

pub fn rules(dir: &Path) -> anyhow::Result<Vec<Rule>> {
    db::read(dir, rules_in)
}

const RULE_COLUMNS: &str =
    "id, name, scope, scope_id, scope_name, space_id, space_name, kind, status";

fn rule_row(r: &rusqlite::Row) -> rusqlite::Result<Rule> {
    Ok(Rule {
        id: r.get(0)?,
        name: r.get(1)?,
        scope: r.get(2)?,
        scope_id: r.get(3)?,
        scope_name: r.get(4)?,
        space_id: r.get(5)?,
        space_name: r.get(6)?,
        kind: r.get(7)?,
        status: r.get(8)?,
    })
}

/// The rules in the order that the user added them.
pub(crate) fn rules_in(c: &Connection) -> anyhow::Result<Vec<Rule>> {
    let mut q = c.prepare(&format!(
        "SELECT {RULE_COLUMNS} FROM status_rule ORDER BY id"
    ))?;
    let rows = q.query_map([], rule_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Adds a rule, or changes the rule with the same name and scope. Returns
/// all rules and the rule from before (None: there was none), for the undo.
pub fn set_rule(dir: &Path, rule: &Rule, now: u64) -> anyhow::Result<(Vec<Rule>, Option<Rule>)> {
    let name = norm(&rule.name);
    if name.is_empty() {
        bail!("the status name is empty");
    }
    if !SCOPES.contains(&rule.scope.as_str()) {
        bail!(
            "a scope is one of {}, not \"{}\"",
            SCOPES.join(", "),
            rule.scope
        );
    }
    let scope_id = if rule.scope == "all" {
        ""
    } else {
        rule.scope_id.trim()
    };
    if rule.scope != "all" && scope_id.is_empty() {
        bail!("a rule for one {} needs its id", rule.scope);
    }
    let status = check(&rule.status)?;
    db::write(dir, |tx| {
        let before = tx
            .query_row(
                &format!(
                    "SELECT {RULE_COLUMNS} FROM status_rule
                     WHERE name = ?1 AND scope = ?2 AND scope_id = ?3"
                ),
                rusqlite::params![name, rule.scope, scope_id],
                rule_row,
            )
            .optional()?;
        // An undo puts back the old row with its id, so it keeps its place.
        let id = (rule.id > 0).then_some(rule.id);
        tx.execute(
            "INSERT INTO status_rule (id, name, scope, scope_id, scope_name, space_id, space_name, kind, status, ts)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
             ON CONFLICT (name, scope, scope_id) DO UPDATE SET
               scope_name = excluded.scope_name, space_id = excluded.space_id,
               space_name = excluded.space_name, kind = excluded.kind,
               status = excluded.status, ts = excluded.ts",
            rusqlite::params![
                id,
                name,
                rule.scope,
                scope_id,
                rule.scope_name.trim(),
                rule.space_id.trim(),
                rule.space_name.trim(),
                rule.kind.trim(),
                status,
                now as i64
            ],
        )?;
        refresh_in(tx)?;
        Ok((rules_in(tx)?, before))
    })
}

/// Deletes the rule with the same name and scope. A missing rule is not an error.
pub fn delete_rule(dir: &Path, rule: &Rule) -> anyhow::Result<Vec<Rule>> {
    let scope_id = if rule.scope == "all" {
        ""
    } else {
        rule.scope_id.trim()
    };
    db::write(dir, |tx| {
        tx.execute(
            "DELETE FROM status_rule WHERE name = ?1 AND scope = ?2 AND scope_id = ?3",
            rusqlite::params![norm(&rule.name), rule.scope, scope_id],
        )?;
        refresh_in(tx)?;
        rules_in(tx)
    })
}

/// How many of the user's saved ClickUp tasks each rule changes now, by
/// rule id. The Settings page shows it next to each rule.
pub fn rule_use(dir: &Path) -> anyhow::Result<HashMap<i64, u32>> {
    db::read(dir, |c| {
        let rules = rules_in(c)?;
        let mut q =
            c.prepare("SELECT data FROM work_item WHERE source = 'clickup' AND in_list = 1")?;
        let rows: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
        let mut out = HashMap::new();
        for data in rows {
            let i: ClickupStatus = serde_json::from_str(&data).unwrap_or_default();
            let place = Place {
                list_id: &i.list_id,
                folder_id: &i.folder_id,
                space_id: &i.space_id,
            };
            if let Some(r) = rule_for(&rules, &i.status, &place) {
                *out.entry(r.id).or_default() += 1;
            }
        }
        Ok(out)
    })
}

// ----- The next item of an Unfinished item -----

/// Links the Unfinished item `key` to the item that continues it. None
/// removes the link: the user accepts the item as it is. Returns the link
/// from before, for the undo.
pub fn set_next(
    dir: &Path,
    key: &str,
    next: Option<&str>,
    now: u64,
) -> anyhow::Result<Option<String>> {
    let key = key.trim();
    crate::items::split(key)?;
    let next = next.map(str::trim).filter(|n| !n.is_empty());
    db::write(dir, |tx| {
        if crate::items::title_place(tx, key)?.is_none() {
            bail!("AIfred does not know the item {key}");
        }
        if let Some(n) = next {
            crate::items::split(n)?;
            if n == key {
                bail!("an item cannot continue itself");
            }
            if crate::items::title_place(tx, n)?.is_none() {
                bail!("AIfred does not know the item {n}");
            }
        }
        let before: Option<String> = tx
            .query_row(
                "SELECT next_key FROM status_next WHERE key = ?1",
                [key],
                |r| r.get(0),
            )
            .optional()?;
        match next {
            Some(n) => tx.execute(
                "INSERT INTO status_next (key, next_key, ts) VALUES (?1, ?2, ?3)
                 ON CONFLICT (key) DO UPDATE SET next_key = excluded.next_key, ts = excluded.ts",
                rusqlite::params![key, n, now as i64],
            )?,
            None => tx.execute("DELETE FROM status_next WHERE key = ?1", [key])?,
        };
        Ok(before)
    })
}

/// The links of status_next: (Unfinished item, the item that continues it).
fn next_links(c: &Connection) -> anyhow::Result<Vec<(String, String)>> {
    let mut q = c.prepare("SELECT key, next_key FROM status_next ORDER BY ts, key")?;
    let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

fn linked(c: &Connection, key: &str) -> anyhow::Result<Linked> {
    Ok(Linked {
        key: key.to_string(),
        title: crate::items::title_place(c, key)?.map(|(t, _)| t),
    })
}

/// Adds the "continued in" links to the statuses: `next` to an Unfinished
/// item, and `continues` to the item that continues it. A link counts only
/// while the item is Unfinished.
fn add_links(c: &Connection, out: &mut HashMap<String, ItemStatus>) -> anyhow::Result<()> {
    for (key, next) in next_links(c)? {
        let status = match out.get(&key) {
            Some(s) => Some(s.status.clone()),
            // A closed item, for example a local task: it is not in `out`.
            None => of_base(c, &key)?.map(|s| s.status),
        };
        if status.as_deref() != Some(UNFINISHED) {
            continue;
        }
        if out.contains_key(&next) {
            let l = linked(c, &key)?;
            if let Some(t) = out.get_mut(&next) {
                t.continues.push(l);
            }
        }
        if out.contains_key(&key) {
            let n = linked(c, &next)?;
            if let Some(s) = out.get_mut(&key) {
                s.next = Some(n);
            }
        }
    }
    Ok(())
}

/// The links of one item, for of_in().
fn add_links_one(c: &Connection, key: &str, s: &mut ItemStatus) -> anyhow::Result<()> {
    for (k, next) in next_links(c)? {
        if k == key && s.status == UNFINISHED {
            s.next = Some(linked(c, &next)?);
        }
        if next == key
            && of_base(c, &k)?.is_some_and(|o| o.status == UNFINISHED)
            && !s.continues.iter().any(|l| l.key == k)
        {
            s.continues.push(linked(c, &k)?);
        }
    }
    Ok(())
}

// ----- The stored status -----

/// The parts of a saved ClickUp item that the status needs.
#[derive(Deserialize, Default)]
struct ClickupStatus {
    #[serde(default)]
    status: String,
    #[serde(default)]
    status_type: String,
    #[serde(default)]
    list_id: String,
    #[serde(default)]
    folder_id: String,
    #[serde(default)]
    space_id: String,
}

/// The status of a downloaded item from its source alone: the user's rule
/// for the ClickUp status, else the group default, else To do (steps 3
/// and 6 of "which status wins").
fn source_status(rules: &[Rule], source: &str, data: &str) -> (&'static str, &'static str) {
    if source != "clickup" {
        return (TODO, by::DEFAULT);
    }
    let c: ClickupStatus = serde_json::from_str(data).unwrap_or_default();
    let place = Place {
        list_id: &c.list_id,
        folder_id: &c.folder_id,
        space_id: &c.space_id,
    };
    map_clickup(rules, &c.status, &c.status_type, &place)
}

/// Writes `work_item.status` again for all items in the list: the user's
/// pick, else source_status(). An item that left the list does not need
/// it: of_base() reads its close reason and its pick. A download writes it
/// again for the items that come back (db::save_list).
pub(crate) fn refresh_in(tx: &Transaction) -> anyhow::Result<()> {
    let rules = rules_in(tx)?;
    let picked = picks_in(tx)?;
    let rows: Vec<(String, String, String, String, String)> = {
        let mut q = tx.prepare(
            "SELECT source, item_id, data, status, status_by FROM work_item WHERE in_list = 1",
        )?;
        q.query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?
    };
    let mut up = tx.prepare(
        "UPDATE work_item SET status = ?3, status_by = ?4 WHERE source = ?1 AND item_id = ?2",
    )?;
    for (source, id, data, old, old_by) in rows {
        let key = format!("{source}:{id}");
        let (status, who) = match picked.get(&key) {
            Some(p) => (p.0.as_str(), by::YOU),
            None => source_status(&rules, &source, &data),
        };
        if status != old || who != old_by {
            up.execute(rusqlite::params![source, id, status, who])?;
        }
    }
    Ok(())
}

/// refresh_in() for one item, after a change of its pick. A local task
/// and an item that is not in the list have nothing to write.
pub(crate) fn refresh_one_in(tx: &Transaction, key: &str) -> anyhow::Result<()> {
    let (source, id) = crate::items::split(key)?;
    if source == "task" {
        return Ok(());
    }
    let data: Option<String> = tx
        .query_row(
            "SELECT data FROM work_item WHERE source = ?1 AND item_id = ?2 AND in_list = 1",
            [source, id],
            |r| r.get(0),
        )
        .optional()?;
    let Some(data) = data else {
        return Ok(());
    };
    let (status, who) = match pick_of(tx, key, true)? {
        Some((s, _)) => (s, by::YOU),
        None => {
            let (s, b) = source_status(&rules_in(tx)?, source, &data);
            (s.to_string(), b)
        }
    };
    tx.execute(
        "UPDATE work_item SET status = ?3, status_by = ?4 WHERE source = ?1 AND item_id = ?2",
        rusqlite::params![source, id, status, who],
    )?;
    Ok(())
}

/// The statuses that the user picked, with the time: item_status, and a
/// weekly review "drop" that is still in force (Cancelled). The newer one wins.
fn picks_in(c: &Connection) -> anyhow::Result<HashMap<String, (String, u64)>> {
    let mut out: HashMap<String, (String, u64)> = HashMap::new();
    let mut q = c.prepare("SELECT key, status, ts FROM item_status")?;
    for r in q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, u64>(2)?,
        ))
    })? {
        let (k, s, ts) = r?;
        out.insert(k, (s, ts));
    }
    for (k, (pick, ts)) in crate::review::picks_in_force(c)? {
        if pick != crate::review::DROP {
            continue;
        }
        if out.get(&k).is_none_or(|(_, t)| *t < ts) {
            out.insert(k, (CANCELLED.to_string(), ts));
        }
    }
    Ok(out)
}

/// picks_in() for one item. With `own` false, only a weekly review drop
/// counts, not the item_status row: the status that the item has with no
/// pick of the user (set() compares with it).
fn pick_of(c: &Connection, key: &str, own: bool) -> anyhow::Result<Option<(String, u64)>> {
    let mut out: Option<(String, u64)> = if own {
        c.query_row(
            "SELECT status, ts FROM item_status WHERE key = ?1",
            [key],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .optional()?
    } else {
        None
    };
    if let Some((pick, ts)) = crate::review::pick_in_force(c, key)?
        && pick == crate::review::DROP
        && out.as_ref().is_none_or(|(_, t)| *t < ts)
    {
        out = Some((CANCELLED.to_string(), ts));
    }
    Ok(out)
}

/// Steps 4 to 6 of "which status wins", for an open item with no pick and
/// no rule: Waiting from the weekly review or an open blocker, In progress
/// from the work timer, else `default`.
fn derive(
    delegated: bool,
    blocked: bool,
    worked: bool,
    default: &'static str,
) -> (&'static str, &'static str) {
    if delegated {
        (WAITING, by::DELEGATE)
    } else if blocked {
        (WAITING, by::BLOCKER)
    } else if worked {
        (PROGRESS, by::TIMER)
    } else {
        (default, by::DEFAULT)
    }
}

/// derive() for one item, from its own rows only.
fn derive_one(
    c: &Connection,
    key: &str,
    default: &'static str,
) -> anyhow::Result<(&'static str, &'static str)> {
    let delegated =
        crate::review::pick_in_force(c, key)?.is_some_and(|(p, _)| p == crate::review::DELEGATE);
    let blocked = crate::blockers::is_blocked_in(c, key)?;
    let worked = c
        .query_row(
            "SELECT 1 FROM work_session WHERE key = ?1 LIMIT 1",
            [key],
            |_| Ok(()),
        )
        .optional()?
        .is_some();
    Ok(derive(delegated, blocked, worked, default))
}

/// The status of an open local task: an open pick of the user, else
/// `derived`. A closed pick of a task is in `task`: an old one does not count.
fn task_status(pick: Option<&str>, derived: (&'static str, &'static str)) -> ItemStatus {
    match pick {
        Some(s) if !is_closed(s) => ItemStatus::new(s, by::YOU, None),
        _ => ItemStatus::new(derived.0, derived.1, None),
    }
}

/// The status of a downloaded item in the list: `status` and `who` from
/// work_item (the pick or the rule), else `derived` from the group default.
fn list_item_status(
    source: &str,
    data: &str,
    status: String,
    who: String,
    derived: impl FnOnce(&'static str) -> anyhow::Result<(&'static str, &'static str)>,
) -> anyhow::Result<ItemStatus> {
    let src = (source == "clickup")
        .then(|| serde_json::from_str::<ClickupStatus>(data).ok())
        .flatten()
        .map(|c| c.status)
        .filter(|s| !s.is_empty());
    let (status, who) = if who == by::DEFAULT {
        let (s, b) = derived(check(&status).unwrap_or(TODO))?;
        (s.to_string(), b.to_string())
    } else {
        (status, who)
    };
    Ok(ItemStatus::new(status, who, src))
}

// ----- Read -----

/// The status of each open local task and each item of the last downloads
/// (also the ones that are closed in AIfred, like an Unfinished ClickUp task).
pub fn all(dir: &Path) -> anyhow::Result<HashMap<String, ItemStatus>> {
    db::read(dir, all_in)
}

pub(crate) fn all_in(c: &Connection) -> anyhow::Result<HashMap<String, ItemStatus>> {
    let mut out = all_in_base(c)?;
    add_links(c, &mut out)?;
    Ok(out)
}

/// all_in() with no "continued in" links. It reads each table once; of_base()
/// gives the same status for one item from its own rows.
fn all_in_base(c: &Connection) -> anyhow::Result<HashMap<String, ItemStatus>> {
    let picked = picks_in(c)?;
    let delegated: HashSet<String> = crate::review::picks_in_force(c)?
        .into_iter()
        .filter(|(_, (p, _))| p == crate::review::DELEGATE)
        .map(|(k, _)| k)
        .collect();
    let blocked = crate::blockers::open_blockers_in(c)?;
    let worked: HashSet<String> = {
        let mut q = c.prepare("SELECT DISTINCT key FROM work_session")?;
        q.query_map([], |r| r.get::<_, String>(0))?
            .collect::<Result<_, _>>()?
    };
    let derived = |key: &str, default: &'static str| {
        derive(
            delegated.contains(key),
            blocked.contains_key(key),
            worked.contains(key),
            default,
        )
    };
    let mut out = HashMap::new();

    let mut q = c.prepare("SELECT id FROM user_task WHERE done = 0")?;
    let ids: Vec<String> = q.query_map([], |r| r.get(0))?.collect::<Result<_, _>>()?;
    for id in ids {
        let key = format!("task:{id}");
        let pick = picked.get(&key).map(|(s, _)| s.as_str());
        out.insert(key.clone(), task_status(pick, derived(&key, TODO)));
    }

    let mut q = c.prepare(
        "SELECT source, item_id, data, status, status_by FROM work_item WHERE in_list = 1",
    )?;
    let rows: Vec<(String, String, String, String, String)> = q
        .query_map([], |r| {
            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?))
        })?
        .collect::<Result<_, _>>()?;
    for (source, id, data, status, who) in rows {
        let key = format!("{source}:{id}");
        let s = list_item_status(&source, &data, status, who, |d| Ok(derived(&key, d)))?;
        out.insert(key, s);
    }
    Ok(out)
}

/// The status of one item, also a closed one. None: AIfred does not know it.
pub fn of(dir: &Path, key: &str) -> anyhow::Result<Option<ItemStatus>> {
    db::read(dir, |c| of_in(c, key))
}

pub(crate) fn of_in(c: &Connection, key: &str) -> anyhow::Result<Option<ItemStatus>> {
    let Some(mut s) = of_base(c, key)? else {
        return Ok(None);
    };
    add_links_one(c, key, &mut s)?;
    Ok(Some(s))
}

/// of_in() with no "continued in" links.
fn of_base(c: &Connection, key: &str) -> anyhow::Result<Option<ItemStatus>> {
    of_with(c, key, true)
}

/// The status that an open item has with no pick of the user (a weekly
/// review drop still counts): the rule, the review, a blocker, the timer, or
/// the default. A closed local task gives its closed status.
fn unpicked(c: &Connection, key: &str) -> anyhow::Result<Option<ItemStatus>> {
    of_with(c, key, false)
}

/// of_base(), from the rows of this one item. `own` false: unpicked().
fn of_with(c: &Connection, key: &str, own: bool) -> anyhow::Result<Option<ItemStatus>> {
    let (source, id) = crate::items::split(key)?;
    if source == "task" {
        let row: Option<(bool, Option<String>)> = c
            .query_row(
                "SELECT done, closed_as FROM user_task WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        return Ok(match row {
            None => None,
            Some((true, closed)) => Some(ItemStatus::new(
                closed.unwrap_or_else(|| DONE.into()),
                by::YOU,
                None,
            )),
            Some((false, _)) => Some(open_task_status(c, key, own)?),
        });
    }
    type Row = (
        bool,
        String,
        String,
        String,
        Option<String>,
        Option<String>,
        Option<u64>,
    );
    let row: Option<Row> = c
        .query_row(
            "SELECT in_list, status, status_by, data, close_reason, close_label, closed_at
             FROM work_item WHERE source = ?1 AND item_id = ?2",
            [source, id],
            |r| {
                Ok((
                    r.get(0)?,
                    r.get(1)?,
                    r.get(2)?,
                    r.get(3)?,
                    r.get(4)?,
                    r.get(5)?,
                    r.get(6)?,
                ))
            },
        )
        .optional()?;
    Ok(match row {
        None => None,
        Some((true, status, who, data, _, _, _)) => {
            let (status, who) = if own {
                (status, who)
            } else if let Some((s, _)) = pick_of(c, key, false)? {
                // A weekly review drop closes the item with no item_status row.
                (s, by::YOU.to_string())
            } else {
                let (s, b) = source_status(&rules_in(c)?, source, &data);
                (s.to_string(), b.to_string())
            };
            Some(list_item_status(source, &data, status, who, |d| {
                derive_one(c, key, d)
            })?)
        }
        Some((false, _, _, _, reason, label, closed_at)) => {
            // A closed pick of the user still wins, unless AIfred knows how
            // the item closed in its source and the pick is older than that
            // close: the newer fact wins. An open pick never counts: the
            // item is closed in its source.
            let older = |ts: u64| reason.is_some() && closed_at.is_some_and(|at| ts <= at);
            let picked = pick_of(c, key, own)?.filter(|(s, ts)| is_closed(s) && !older(*ts));
            Some(match picked {
                Some((s, _)) => ItemStatus::new(s, by::YOU, label),
                None => match reason.as_deref() {
                    // Still open in its source: it is not the user's any more.
                    Some(STILL_OPEN) => ItemStatus::new(TODO, by::SOURCE, label),
                    Some(r) => ItemStatus::new(r, by::SOURCE, label),
                    None => ItemStatus::new(DONE, by::SOURCE, label),
                },
            })
        }
    })
}

/// The status of a local task that is open: of_with() for a task, as if
/// it were open. `own` false: with no pick of the user.
fn open_task_status(c: &Connection, key: &str, own: bool) -> anyhow::Result<ItemStatus> {
    let pick = pick_of(c, key, own)?;
    Ok(task_status(
        pick.as_ref().map(|(s, _)| s.as_str()),
        derive_one(c, key, TODO)?,
    ))
}

// ----- Change -----

/// What set() changed, for the undo (restore).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Before {
    pub key: String,
    /// The item_status row from before: the status and the time.
    pub picked: Option<(String, u64)>,
    /// A local task from before: done, closed_as, done_at.
    pub task: Option<(bool, Option<String>, Option<i64>)>,
    /// The real time that AIfred kept for the item before (worktime.rs).
    /// None: it had none.
    #[serde(default)]
    pub work: Option<crate::worktime::Kept>,
}

/// The user picks `status` for the item `key`. Returns what it was before,
/// for the undo. "Done" of a ClickUp task must go through "mark done"
/// (it changes ClickUp), so it is refused here. An email has no status.
///
/// A local task: Done finishes it as before (jobs and work time, tasks.rs).
/// Unfinished and Cancelled close it with no work time and no job. An open
/// status opens it again. A GitHub item: Done ends its work and keeps the
/// real time, like a finished task. Unfinished and Cancelled stop the work
/// and forget a real time from an earlier finish.
///
/// A pick that the item already has from its rule, a blocker, or the timer
/// is not saved: the status stays theirs, and it changes with them.
pub fn set(dir: &Path, key: &str, status: &str, now: u64) -> anyhow::Result<Before> {
    let key = key.trim();
    let status = check(status)?;
    let (source, id) = crate::items::split(key)?;
    match source {
        "gmail" => bail!("an email has no status: hide it instead"),
        "clickup" if status == DONE => {
            bail!("use mark_task_done for Done of a ClickUp task: it changes ClickUp")
        }
        _ => {}
    }
    db::write(dir, |tx| {
        if crate::items::title_place(tx, key)?.is_none() {
            bail!("AIfred does not know the item {key}");
        }
        let picked: Option<(String, u64)> = tx
            .query_row(
                "SELECT status, ts FROM item_status WHERE key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let task: Option<(bool, Option<String>, Option<i64>)> = if source == "task" {
            tx.query_row(
                "SELECT done, closed_as, done_at FROM user_task WHERE id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?
        } else {
            None
        };
        let before = Before {
            key: key.to_string(),
            picked,
            task: task.clone(),
            work: crate::worktime::kept_in(tx, key)?,
        };
        if let Some((done, closed_as, _)) = task {
            set_task_in(tx, id, status, done, closed_as.as_deref(), now)?;
        } else {
            set_item_in(tx, key, status, now)?;
        }
        Ok(before)
    })
}

/// Saves the pick `status` of `key`, or removes the pick when the item has
/// that status with no pick (unpicked): then the rule, the review, a
/// blocker, or the timer keeps it, and the status follows them again.
fn write_pick(tx: &Transaction, key: &str, status: &str, now: u64) -> anyhow::Result<()> {
    let same = unpicked(tx, key)?.is_some_and(|u| u.status == status);
    if same {
        tx.execute("DELETE FROM item_status WHERE key = ?1", [key])?;
    } else {
        tx.execute(
            "INSERT INTO item_status (key, status, ts) VALUES (?1, ?2, ?3)
             ON CONFLICT (key) DO UPDATE SET status = excluded.status, ts = excluded.ts",
            rusqlite::params![key, status, now as i64],
        )?;
    }
    refresh_one_in(tx, key)
}

/// set() for a GitHub or ClickUp item.
fn set_item_in(tx: &Transaction, key: &str, status: &str, now: u64) -> anyhow::Result<()> {
    match status {
        DONE => {
            // The pick first: finish_in() keeps no time for an item that
            // reads Unfinished or Cancelled.
            write_pick(tx, key, status, now)?;
            crate::worktime::finish_in(tx, key, now)?;
        }
        UNFINISHED | CANCELLED => {
            // Not finished work: the timer stops, and no real time is kept,
            // also not one from an earlier finish.
            crate::worktime::stop_in(tx, key, now)?;
            crate::worktime::unfinish_in(tx, key)?;
            write_pick(tx, key, status, now)?;
        }
        _ => write_pick(tx, key, status, now)?,
    }
    Ok(())
}

fn set_task_in(
    tx: &Transaction,
    id: &str,
    status: &str,
    done: bool,
    closed_as: Option<&str>,
    now: u64,
) -> anyhow::Result<()> {
    let key = format!("task:{id}");
    let finished = done && closed_as.is_none_or(|c| c == DONE);
    match status {
        DONE => {
            tx.execute("DELETE FROM item_status WHERE key = ?1", [&key])?;
            if !finished {
                if done {
                    // Unfinished or Cancelled before: open it first, so the
                    // finish runs the jobs and keeps the work time.
                    crate::tasks::set_done_in(tx, id, false)?;
                }
                crate::tasks::set_done_in(tx, id, true)?;
            }
        }
        UNFINISHED | CANCELLED => {
            tx.execute("DELETE FROM item_status WHERE key = ?1", [&key])?;
            if finished {
                // It was finished work: forget its real time.
                crate::worktime::unfinish_in(tx, &key)?;
            }
            crate::worktime::stop_in(tx, &key, now)?;
            tx.execute(
                "UPDATE task SET done = 1, done_at = COALESCE(CASE WHEN done = 1 THEN done_at END, ?1),
                   closed_as = ?2, updated = ?1
                 WHERE doer = 'you' AND id = ?3",
                rusqlite::params![now as i64, status, id],
            )?;
        }
        _ => {
            if done {
                crate::tasks::set_done_in(tx, id, false)?;
            }
            write_pick(tx, &key, status, now)?;
        }
    }
    Ok(())
}

/// The undo of set(): puts the status from before back, with the real time
/// that the item had kept before. A real time that set() kept (Done) goes
/// away again.
pub fn restore(dir: &Path, before: &Before, now: u64) -> anyhow::Result<()> {
    let (source, id) = crate::items::split(&before.key)?;
    db::write(dir, |tx| {
        if source == "task"
            && let Some((done, closed_as, done_at)) = &before.task
        {
            let now_done: Option<(bool, Option<String>)> = tx
                .query_row(
                    "SELECT done, closed_as FROM user_task WHERE id = ?1",
                    [id],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let Some((now_done, now_closed)) = now_done else {
                bail!("no task with id {id}");
            };
            let now_finished = now_done && now_closed.as_deref().is_none_or(|c| c == DONE);
            if now_finished {
                // The undo of a finish forgets the real time, as before (tasks.rs).
                crate::tasks::set_done_in(tx, id, false)?;
            }
            if *done {
                tx.execute(
                    "UPDATE task SET done = 1, done_at = ?1, closed_as = ?2, updated = ?3
                     WHERE doer = 'you' AND id = ?4",
                    rusqlite::params![done_at, closed_as, now as i64, id],
                )?;
            } else {
                tx.execute(
                    "UPDATE task SET done = 0, done_at = NULL, closed_as = NULL, updated = ?1
                     WHERE doer = 'you' AND id = ?2",
                    rusqlite::params![now as i64, id],
                )?;
            }
        }
        match &before.picked {
            Some((s, ts)) => tx.execute(
                "INSERT INTO item_status (key, status, ts) VALUES (?1, ?2, ?3)
                 ON CONFLICT (key) DO UPDATE SET status = excluded.status, ts = excluded.ts",
                rusqlite::params![before.key, s, *ts as i64],
            )?,
            None => tx.execute("DELETE FROM item_status WHERE key = ?1", [&before.key])?,
        };
        match &before.work {
            Some(w) => crate::worktime::keep_in(tx, w)?,
            None => crate::worktime::unfinish_in(tx, &before.key)?,
        }
        refresh_one_in(tx, &before.key)
    })
}

/// Opens a GitHub or ClickUp item that is closed only in AIfred again, with
/// the least change: the pick of the user goes away, so the rule, a
/// blocker, or the timer decides again. Only when the item is still closed
/// then (a rule, or a weekly review drop) does it get the pick To do.
/// Returns what it was before, for the undo (restore).
pub fn open_again(dir: &Path, key: &str, now: u64) -> anyhow::Result<Before> {
    let key = key.trim();
    let (source, _) = crate::items::split(key)?;
    if matches!(source, "task" | "gmail") {
        bail!("open_again is for a GitHub or ClickUp item: set the status of {key} instead");
    }
    db::write(dir, |tx| {
        if crate::items::title_place(tx, key)?.is_none() {
            bail!("AIfred does not know the item {key}");
        }
        let picked: Option<(String, u64)> = tx
            .query_row(
                "SELECT status, ts FROM item_status WHERE key = ?1",
                [key],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        let before = Before {
            key: key.to_string(),
            picked,
            task: None,
            work: crate::worktime::kept_in(tx, key)?,
        };
        let still_closed = unpicked(tx, key)?.is_some_and(|s| is_closed(&s.status));
        if still_closed {
            write_pick(tx, key, TODO, now)?;
        } else {
            tx.execute("DELETE FROM item_status WHERE key = ?1", [key])?;
            refresh_one_in(tx, key)?;
        }
        Ok(before)
    })
}

/// Forgets the rows of an item that is deleted: its pick, and its
/// "continued in" links in both directions (tasks.rs remove).
pub(crate) fn forget_in(tx: &Transaction, key: &str) -> anyhow::Result<()> {
    tx.execute("DELETE FROM item_status WHERE key = ?1", [key])?;
    tx.execute(
        "DELETE FROM status_next WHERE key = ?1 OR next_key = ?1",
        [key],
    )?;
    Ok(())
}

// ----- Why an item closed -----

/// How long after an item left the list AIfred still reads why. After
/// that, an item with no known reason stays Done.
const LOOKUP_SECS: u64 = 7 * 86_400;
/// The most failed reads of one item.
const MAX_TRIES: i64 = 5;
/// The most reads after one download.
const MAX_LOOKUPS: usize = 20;
/// An item that is still open in its source after it left the list (for
/// example, an issue that is not the user's any more) can close later.
/// AIfred reads it again this long after it left, at most once a day.
const STILL_OPEN_SECS: u64 = 30 * 86_400;
const RECHECK_SECS: u64 = 86_400;

/// The close reason of an item that is still open in its source after it
/// left the list. It is never a status: of_in() shows such an item as To do.
pub const STILL_OPEN: &str = "open";

/// The close reason to save for a source status that maps to `status`:
/// the closed status itself, else STILL_OPEN.
pub fn reason_for(status: &str) -> &'static str {
    match check(status) {
        Ok(s) if is_closed(s) => s,
        _ => STILL_OPEN,
    }
}

/// The ids of the items of `source` that left the list a short time ago,
/// and whose close reason AIfred does not know yet, each once. Also the
/// items that were still open in their source at the last read, again
/// after RECHECK_SECS, for STILL_OPEN_SECS after they left.
pub fn to_look_up(dir: &Path, source: &str, now: u64) -> anyhow::Result<Vec<String>> {
    db::read(dir, |c| {
        let mut q = c.prepare(
            "SELECT item_id FROM work_item
             WHERE source = ?1 AND in_list = 0 AND (
               (close_reason IS NULL AND close_tries < ?2 AND last_seen >= ?3)
               OR (close_reason = ?5 AND close_tries < ?2 AND close_checked <= ?6 AND last_seen >= ?7)
             )
             ORDER BY last_seen DESC LIMIT ?4",
        )?;
        let rows = q.query_map(
            rusqlite::params![
                source,
                MAX_TRIES,
                now.saturating_sub(LOOKUP_SECS) as i64,
                MAX_LOOKUPS as i64,
                STILL_OPEN,
                now.saturating_sub(RECHECK_SECS) as i64,
                now.saturating_sub(STILL_OPEN_SECS) as i64,
            ],
            |r| r.get(0),
        )?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// Saves why an item closed: `status` is Done, Unfinished, or Cancelled, or
/// STILL_OPEN, and `label` is the name in the source. Nothing changes when
/// the item is in the list again.
pub fn save_reason(
    dir: &Path,
    source: &str,
    id: &str,
    status: &str,
    label: &str,
    now: u64,
) -> anyhow::Result<()> {
    let status = if status == STILL_OPEN {
        STILL_OPEN
    } else {
        let s = check(status)?;
        if !is_closed(s) {
            bail!("a close reason is a closed status or \"{STILL_OPEN}\", not \"{s}\"");
        }
        s
    };
    db::write(dir, |tx| {
        tx.execute(
            "UPDATE work_item SET close_reason = ?3, close_label = NULLIF(?4, ''), close_checked = ?5, close_tries = 0
             WHERE source = ?1 AND item_id = ?2 AND in_list = 0",
            rusqlite::params![source, id, status, label.trim(), now as i64],
        )?;
        Ok(())
    })
}

/// Counts a failed read of why an item closed. After MAX_TRIES, AIfred stops.
pub fn failed_reason(dir: &Path, source: &str, id: &str) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        tx.execute(
            "UPDATE work_item SET close_tries = close_tries + 1 WHERE source = ?1 AND item_id = ?2",
            [source, id],
        )?;
        Ok(())
    })
}

/// Why a ClickUp task closed, from its status now: its closed status in
/// AIfred, or STILL_OPEN for a task that is still open in ClickUp (for
/// example, it has another assignee now).
pub fn clickup_reason(dir: &Path, s: &crate::clickup::TaskState) -> anyhow::Result<&'static str> {
    let rules = rules(dir)?;
    let place = Place {
        list_id: &s.list_id,
        folder_id: &s.folder_id,
        space_id: &s.space_id,
    };
    let (status, _) = map_clickup(&rules, &s.status, &s.status_type, &place);
    Ok(reason_for(status))
}

/// Reads why the ClickUp tasks that left the list closed. Errors only
/// count a try: the next download tries again.
pub async fn look_up_clickup(
    dir: &Path,
    client: &crate::clickup::Client,
    now: u64,
) -> anyhow::Result<()> {
    for id in to_look_up(dir, "clickup", now)? {
        match client.task_state(&id).await {
            Ok(s) => {
                let status = clickup_reason(dir, &s)?;
                save_reason(dir, "clickup", &id, status, &s.status, now)?;
            }
            Err(_) => failed_reason(dir, "clickup", &id)?,
        }
    }
    Ok(())
}

/// Reads why the GitHub items that left the list closed.
pub async fn look_up_github(dir: &Path, now: u64) -> anyhow::Result<()> {
    use crate::github::{Closed, close_state};
    for id in to_look_up(dir, "github", now)? {
        match close_state(&id).await {
            Ok(Closed::Completed) => save_reason(dir, "github", &id, DONE, "", now)?,
            Ok(Closed::NotDone(why)) => save_reason(dir, "github", &id, CANCELLED, &why, now)?,
            // Still open on GitHub: the user is not on it now. It can close later.
            Ok(Closed::Open) => {
                save_reason(dir, "github", &id, STILL_OPEN, "open, not yours now", now)?
            }
            Err(_) => failed_reason(dir, "github", &id)?,
        }
    }
    Ok(())
}

// ----- For the AI -----

/// The text name of a status, for the AI and the app.
pub fn label(status: &str) -> &'static str {
    match status {
        TODO => "To do",
        PROGRESS => "In progress",
        WAITING => "Waiting",
        DONE => "Done",
        UNFINISHED => "Unfinished",
        CANCELLED => "Cancelled",
        _ => "To do",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clickup::tests::temp;

    fn rule(name: &str, scope: &str, id: &str, status: &str) -> Rule {
        Rule {
            id: 0,
            name: name.into(),
            scope: scope.into(),
            scope_id: id.into(),
            scope_name: String::new(),
            space_id: String::new(),
            space_name: String::new(),
            kind: String::new(),
            status: status.into(),
        }
    }

    #[test]
    fn the_narrowest_rule_wins() {
        let rules = [
            rule("delayed", "all", "", WAITING),
            rule("delayed", "space", "s1", UNFINISHED),
            rule("delayed", "list", "l2", CANCELLED),
        ];
        let p = |l: &'static str, s: &'static str| Place {
            list_id: l,
            folder_id: "",
            space_id: s,
        };
        assert_eq!(
            rule_for(&rules, "Delayed", &p("l1", "s1")).unwrap().status,
            UNFINISHED
        );
        assert_eq!(
            rule_for(&rules, "delayed", &p("l2", "s1")).unwrap().status,
            CANCELLED
        );
        assert_eq!(
            rule_for(&rules, "delayed", &p("l3", "s9")).unwrap().status,
            WAITING
        );
        assert!(rule_for(&rules, "planned", &p("l1", "s1")).is_none());
        // A rule for a space also covers a new list in it (a new quarter).
        assert_eq!(
            rule_for(&rules, "delayed", &p("q1-2027", "s1"))
                .unwrap()
                .status,
            UNFINISHED
        );
    }

    #[test]
    fn the_rocks_statuses_map_as_planned() {
        let none: [Rule; 0] = [];
        let p = Place::default();
        assert_eq!(
            map_clickup(&none, "planned", "open", &p),
            (TODO, by::DEFAULT)
        );
        // Only the group counts, not the name: "delayed" is Not started.
        assert_eq!(
            map_clickup(&none, "delayed", "unstarted", &p),
            (TODO, by::DEFAULT)
        );
        assert_eq!(
            map_clickup(&none, "cancelled", "done", &p),
            (DONE, by::DEFAULT)
        );
        assert_eq!(
            map_clickup(&none, "in progress", "custom", &p),
            (PROGRESS, by::DEFAULT)
        );
        assert_eq!(
            map_clickup(&none, "partially complete", "done", &p),
            (DONE, by::DEFAULT)
        );
        assert_eq!(
            map_clickup(&none, "complete", "closed", &p),
            (DONE, by::DEFAULT)
        );
        // A rule wins over the group.
        let rules = [rule("delayed", "all", "", UNFINISHED)];
        assert_eq!(
            map_clickup(&rules, "delayed", "unstarted", &p),
            (UNFINISHED, by::RULE)
        );
    }

    fn save_clickup(d: &Path, items: &[(&str, &str, &str)]) {
        save_clickup_at(d, items, 100);
    }

    /// save_clickup() as a download at `now`.
    fn save_clickup_at(d: &Path, items: &[(&str, &str, &str)], now: u64) {
        let rows: Vec<String> = items
            .iter()
            .map(|(id, status, kind)| {
                serde_json::json!({
                    "id": id, "title": id, "url": "u", "status": status, "status_type": kind,
                    "list": "Rocks (Q4 2026)", "list_id": "l1", "space_id": "s1",
                    "workspace": "W", "updated": 1
                })
                .to_string()
            })
            .collect();
        db::write(d, |tx| {
            let list: Vec<db::ListItem> = items
                .iter()
                .zip(&rows)
                .map(|((id, _, _), data)| db::ListItem {
                    id: id.to_string(),
                    title: id.to_string(),
                    place: "W / Rocks".into(),
                    url: "u".into(),
                    data: data.clone(),
                    details: None,
                })
                .collect();
            db::save_list(tx, "clickup", "acc", &list, &[], 2, now)
        })
        .unwrap();
    }

    fn open(d: &Path, key: &str) -> bool {
        db::read(d, |c| crate::items::is_open(c, key)).unwrap()
    }

    #[test]
    fn a_delayed_rock_with_a_rule_is_unfinished_and_not_open() {
        let d = temp("status-rock");
        save_clickup(
            &d,
            &[
                ("r1", "delayed", "unstarted"),
                ("r2", "in progress", "custom"),
            ],
        );
        assert_eq!(
            of(&d, "clickup:r1").unwrap().unwrap().status,
            TODO,
            "no rule: the group decides"
        );
        let mut r = rule("delayed", "list", "l1", UNFINISHED);
        r.space_id = "s1".into();
        r.kind = "unstarted".into();
        let (rules, before) = set_rule(&d, &r, 4).unwrap();
        assert!(before.is_none());
        assert_eq!(
            (rules[0].space_id.as_str(), rules[0].kind.as_str()),
            ("s1", "unstarted")
        );
        assert_eq!(rule_use(&d).unwrap()[&rules[0].id], 1);
        let all = all(&d).unwrap();
        assert_eq!(all["clickup:r1"].status, UNFINISHED);
        assert_eq!(all["clickup:r1"].source.as_deref(), Some("delayed"));
        assert_eq!(all["clickup:r2"].status, PROGRESS);
        assert!(
            !open(&d, "clickup:r1"),
            "an Unfinished task is not open in AIfred"
        );
        assert!(open(&d, "clickup:r2"));

        // The list rule is narrower than a space rule, so it wins.
        set_rule(&d, &rule("Delayed", "space", "s1", WAITING), 5).unwrap();
        assert_eq!(of(&d, "clickup:r1").unwrap().unwrap().status, UNFINISHED);
        // Without the list rule, the space rule covers the list.
        delete_rule(&d, &r).unwrap();
        assert!(open(&d, "clickup:r1"));
        // A new download keeps the rule.
        save_clickup(&d, &[("r1", "delayed", "unstarted")]);
        assert_eq!(of(&d, "clickup:r1").unwrap().unwrap().status, WAITING);
        assert_eq!(of(&d, "clickup:r1").unwrap().unwrap().by, by::RULE);
        let rules = delete_rule(&d, &rule("delayed", "space", "s1", WAITING)).unwrap();
        assert!(rules.is_empty());
        assert_eq!(of(&d, "clickup:r1").unwrap().unwrap().status, TODO);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_pick_wins_and_its_undo_puts_back_the_old_status() {
        let d = temp("status-pick");
        save_clickup(&d, &[("c1", "to do", "open")]);
        assert!(
            set(&d, "clickup:c1", DONE, 1).is_err(),
            "Done goes through mark done"
        );
        let before = set(&d, "clickup:c1", CANCELLED, 10).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, CANCELLED);
        assert!(!open(&d, "clickup:c1"));
        // A download keeps the pick.
        save_clickup(&d, &[("c1", "in progress", "custom")]);
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, CANCELLED);
        restore(&d, &before, 11).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, PROGRESS);
        assert!(open(&d, "clickup:c1"));
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn timer_and_blockers_give_in_progress_and_waiting() {
        let d = temp("status-derived");
        save_clickup(
            &d,
            &[
                ("c1", "to do", "open"),
                ("c2", "to do", "open"),
                ("c3", "delayed", "unstarted"),
            ],
        );
        crate::worktime::start(&d, "clickup:c1", 10).unwrap();
        crate::worktime::stop(&d, 20).unwrap();
        assert_eq!(
            of(&d, "clickup:c1").unwrap().unwrap().status,
            PROGRESS,
            "it stays after the stop"
        );
        crate::blockers::add(&d, "clickup:c2", "clickup:c1", "you", "", 30).unwrap();
        assert_eq!(of(&d, "clickup:c2").unwrap().unwrap().status, WAITING);
        // A rule wins over the timer.
        set_rule(&d, &rule("delayed", "space", "s1", UNFINISHED), 35).unwrap();
        crate::worktime::start(&d, "clickup:c3", 40).unwrap();
        assert_eq!(of(&d, "clickup:c3").unwrap().unwrap().status, UNFINISHED);
        // The user's pick wins over the timer.
        set(&d, "clickup:c1", TODO, 50).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, TODO);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_unfinished_task_is_closed_but_not_finished_work() {
        let d = temp("status-task");
        let s = crate::tasks::TaskStore::new(&d);
        let t = s.add("Rock", "", "you").unwrap();
        let key = format!("task:{}", t.id);
        crate::worktime::start(&d, &key, 10).unwrap();
        let before = set(&d, &key, UNFINISHED, 100).unwrap();
        let st = of(&d, &key).unwrap().unwrap();
        assert_eq!(st.status, UNFINISHED);
        assert!(!open(&d, &key));
        assert_eq!(
            crate::worktime::running(&d).unwrap(),
            None,
            "the timer stops"
        );
        let done: i64 = db::read(&d, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM work_done", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(done, 0, "no real time is kept");
        let events: i64 = db::read(&d, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM job_event", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(events, 0, "no task_done job starts");
        // Done from Unfinished finishes it as usual.
        set(&d, &key, DONE, 200).unwrap();
        assert_eq!(of(&d, &key).unwrap().unwrap().status, DONE);
        // In progress opens it again.
        set(&d, &key, PROGRESS, 300).unwrap();
        assert!(open(&d, &key));
        assert_eq!(of(&d, &key).unwrap().unwrap().status, PROGRESS);
        // The undo of the first pick puts it back to open, with no pick.
        restore(&d, &before, 400).unwrap();
        assert!(open(&d, &key));
        assert_eq!(of(&d, &key).unwrap().unwrap().by, by::TIMER);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_unfinished_clickup_task_keeps_no_work_time() {
        let d = temp("status-work");
        save_clickup(&d, &[("c1", "to do", "open")]);
        crate::worktime::start(&d, "clickup:c1", 10).unwrap();
        crate::worktime::stop(&d, 600).unwrap();
        set(&d, "clickup:c1", UNFINISHED, 700).unwrap();
        // The timer "Done" button, or a close elsewhere, asks for the finish.
        assert_eq!(
            crate::worktime::finish(&d, "clickup:c1", 800).unwrap(),
            None
        );
        let done: i64 = db::read(&d, |c| {
            Ok(c.query_row("SELECT COUNT(*) FROM work_done", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(done, 0);
        // A rule that makes it Unfinished works the same.
        set_rule(&d, &rule("delayed", "all", "", UNFINISHED), 850).unwrap();
        save_clickup(&d, &[("c2", "delayed", "unstarted")]);
        crate::worktime::start(&d, "clickup:c2", 900).unwrap();
        assert_eq!(
            crate::worktime::finish(&d, "clickup:c2", 1500).unwrap(),
            None
        );
        assert_eq!(
            crate::worktime::running(&d).unwrap(),
            None,
            "the timer stops"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_closed_item_gets_its_reason_once() {
        let d = temp("status-reason");
        save_clickup(&d, &[("c1", "to do", "open"), ("c2", "to do", "open")]);
        save_clickup(&d, &[("c2", "to do", "open")]);
        assert_eq!(to_look_up(&d, "clickup", 200).unwrap(), ["c1"]);
        assert_eq!(
            of(&d, "clickup:c1").unwrap().unwrap().status,
            DONE,
            "no reason yet: Done"
        );
        let s = crate::clickup::TaskState {
            status: "won't do".into(),
            status_type: "closed".into(),
            list_id: "l1".into(),
            folder_id: String::new(),
            space_id: "s1".into(),
        };
        assert_eq!(clickup_reason(&d, &s).unwrap(), DONE, "the group: Closed");
        set_rule(&d, &rule("won't do", "space", "s1", CANCELLED), 150).unwrap();
        let reason = clickup_reason(&d, &s).unwrap();
        assert_eq!(reason, CANCELLED);
        save_reason(&d, "clickup", "c1", reason, &s.status, 160).unwrap();
        let st = of(&d, "clickup:c1").unwrap().unwrap();
        assert_eq!(
            (st.status.as_str(), st.source.as_deref()),
            (CANCELLED, Some("won't do"))
        );
        assert!(
            to_look_up(&d, "clickup", 200).unwrap().is_empty(),
            "read only once"
        );
        // Back in the list: the reason goes away.
        save_clickup(&d, &[("c1", "to do", "open")]);
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, TODO);
        // Too old: no read.
        save_clickup(&d, &[]);
        assert!(
            to_look_up(&d, "clickup", 100 + LOOKUP_SECS + 1)
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_unfinished_item_links_to_the_item_that_continues_it() {
        let d = temp("status-next");
        save_clickup(
            &d,
            &[("q1", "delayed", "unstarted"), ("q2", "planned", "open")],
        );
        set_rule(&d, &rule("delayed", "space", "s1", UNFINISHED), 5).unwrap();
        assert!(set_next(&d, "clickup:q1", Some("clickup:q1"), 6).is_err());
        assert!(set_next(&d, "clickup:q1", Some("clickup:nope"), 6).is_err());
        assert_eq!(
            set_next(&d, "clickup:q1", Some("clickup:q2"), 6).unwrap(),
            None
        );
        let all = all(&d).unwrap();
        let next = all["clickup:q1"].next.as_ref().unwrap();
        assert_eq!(
            (next.key.as_str(), next.title.as_deref()),
            ("clickup:q2", Some("q2"))
        );
        assert_eq!(all["clickup:q2"].continues[0].key, "clickup:q1");
        assert_eq!(of(&d, "clickup:q2").unwrap().unwrap().continues.len(), 1);

        // A closed local task keeps its link too.
        let t = crate::tasks::TaskStore::new(&d)
            .add("Old rock", "", "you")
            .unwrap();
        let key = format!("task:{}", t.id);
        set(&d, &key, UNFINISHED, 7).unwrap();
        set_next(&d, &key, Some("clickup:q2"), 8).unwrap();
        assert_eq!(
            of(&d, &key).unwrap().unwrap().next.unwrap().key,
            "clickup:q2"
        );
        assert_eq!(all_fn(&d)["clickup:q2"].continues.len(), 2);

        // The link counts only while the item is Unfinished.
        delete_rule(&d, &rule("delayed", "space", "s1", UNFINISHED)).unwrap();
        assert!(all_fn(&d)["clickup:q1"].next.is_none());
        // None removes it, and gives back the link from before (the undo).
        assert_eq!(
            set_next(&d, &key, None, 9).unwrap().as_deref(),
            Some("clickup:q2")
        );
        assert!(of(&d, &key).unwrap().unwrap().next.is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn all_fn(d: &Path) -> HashMap<String, ItemStatus> {
        all(d).unwrap()
    }

    fn save_github(d: &Path, ids: &[&str], now: u64) {
        let list: Vec<db::ListItem> = ids
            .iter()
            .map(|id| db::ListItem {
                id: id.to_string(),
                title: id.to_string(),
                place: "acme/shop".into(),
                url: "u".into(),
                data: serde_json::json!({ "id": id, "updated": "2026-11-01T00:00:00Z" })
                    .to_string(),
                details: None,
            })
            .collect();
        db::write(d, |tx| {
            db::save_list(tx, "github", "me", &list, &[], 1, now)
        })
        .unwrap();
    }

    fn kept_minutes(d: &Path, key: &str) -> Option<u32> {
        db::read(d, |c| {
            Ok(crate::worktime::kept_in(c, key)?.map(|k| k.minutes))
        })
        .unwrap()
    }

    /// Finding: Unfinished or Cancelled on a GitHub or ClickUp item kept the
    /// real time of an earlier finish, so the day check still counted it.
    /// And the undo gave the status back, but not the real time.
    #[test]
    fn cancelled_forgets_the_real_time_and_the_undo_puts_it_back() {
        let d = temp("status-cancel-time");
        save_github(&d, &["acme/shop#1"], 100);
        let key = "github:acme/shop#1";
        crate::worktime::start(&d, key, 1000).unwrap();
        crate::worktime::finish(&d, key, 1000 + 90 * 60).unwrap();
        assert_eq!(kept_minutes(&d, key), Some(90));
        let before = set(&d, key, CANCELLED, 7000).unwrap();
        assert_eq!(kept_minutes(&d, key), None, "not finished work");
        assert_eq!(before.work.as_ref().map(|w| w.minutes), Some(90));
        restore(&d, &before, 8000).unwrap();
        assert_eq!(kept_minutes(&d, key), Some(90), "the undo brings it back");
        assert_eq!(of(&d, key).unwrap().unwrap().by, by::TIMER);

        // The same for a finished local task.
        let s = crate::tasks::TaskStore::new(&d);
        let t = s.add("Report", "", "you").unwrap();
        let tkey = format!("task:{}", t.id);
        crate::worktime::start(&d, &tkey, 10_000).unwrap();
        crate::worktime::stop(&d, 10_000 + 120 * 60).unwrap();
        s.set_done(&t.id, true).unwrap();
        assert_eq!(kept_minutes(&d, &tkey), Some(120));
        let before = set(&d, &tkey, CANCELLED, 20_000).unwrap();
        assert_eq!(kept_minutes(&d, &tkey), None);
        restore(&d, &before, 21_000).unwrap();
        assert_eq!(kept_minutes(&d, &tkey), Some(120));
        assert_eq!(of(&d, &tkey).unwrap().unwrap().status, DONE);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding: Done on a GitHub item from the chat only wrote the status.
    /// The timer kept running and no real time was kept.
    #[test]
    fn done_on_a_github_item_ends_the_work_and_keeps_the_time() {
        let d = temp("status-gh-done");
        save_github(&d, &["acme/shop#2"], 100);
        let key = "github:acme/shop#2";
        crate::worktime::start(&d, key, 1000).unwrap();
        let before = set(&d, key, DONE, 1000 + 30 * 60).unwrap();
        assert_eq!(
            crate::worktime::running(&d).unwrap(),
            None,
            "the timer stops"
        );
        assert_eq!(kept_minutes(&d, key), Some(30));
        assert_eq!(of(&d, key).unwrap().unwrap().status, DONE);
        assert!(before.work.is_none());
        restore(&d, &before, 5000).unwrap();
        assert_eq!(
            kept_minutes(&d, key),
            None,
            "the undo of a finish forgets the time"
        );
        assert_eq!(of(&d, key).unwrap().unwrap().by, by::TIMER);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding: the undo of a chat status change, and "Open again", wrote a
    /// pick that then won over the rule, the blocker, and the timer for good.
    #[test]
    fn a_pick_that_the_item_already_has_is_not_saved() {
        let d = temp("status-unpin");
        save_clickup(&d, &[("c1", "to do", "open"), ("c2", "to do", "open")]);
        crate::worktime::start(&d, "clickup:c1", 10).unwrap();
        crate::worktime::stop(&d, 20).unwrap();
        // The chat sets Waiting, then the user asks for the undo: the chat
        // sets the old status, In progress, which the timer gives anyway.
        set(&d, "clickup:c1", WAITING, 30).unwrap();
        set(&d, "clickup:c1", PROGRESS, 40).unwrap();
        let st = of(&d, "clickup:c1").unwrap().unwrap();
        assert_eq!((st.status.as_str(), st.by.as_str()), (PROGRESS, by::TIMER));
        // So a new blocker still counts.
        crate::blockers::add(&d, "clickup:c1", "clickup:c2", "you", "", 50).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().by, by::BLOCKER);
        // The user picks To do on purpose: that is saved, and it wins.
        set(&d, "clickup:c1", TODO, 60).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().by, by::YOU);

        // "Open again" of an item that a pick closed: the pick goes away,
        // and the timer decides again.
        set(&d, "clickup:c2", CANCELLED, 70).unwrap();
        crate::worktime::start(&d, "clickup:c2", 80).unwrap();
        let before = open_again(&d, "clickup:c2", 90).unwrap();
        let st = of(&d, "clickup:c2").unwrap().unwrap();
        assert_eq!((st.status.as_str(), st.by.as_str()), (PROGRESS, by::TIMER));
        restore(&d, &before, 95).unwrap();
        assert_eq!(of(&d, "clickup:c2").unwrap().unwrap().status, CANCELLED);
        // "Open again" of an item that a rule closed needs the pick To do.
        save_clickup(&d, &[("c3", "delayed", "unstarted")]);
        set_rule(&d, &rule("delayed", "all", "", UNFINISHED), 100).unwrap();
        open_again(&d, "clickup:c3", 110).unwrap();
        let st = of(&d, "clickup:c3").unwrap().unwrap();
        assert_eq!((st.status.as_str(), st.by.as_str()), (TODO, by::YOU));
        assert!(open_again(&d, "task:t1", 120).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding: "mark done" in ClickUp only took the task out of the list.
    /// A Cancelled or Unfinished pick from before still won over the close,
    /// and the finish right after it dropped the real time.
    #[test]
    fn a_close_in_the_source_wins_over_an_older_pick() {
        let d = temp("status-close-newer");
        save_clickup(
            &d,
            &[
                ("c1", "to do", "open"),
                ("c2", "to do", "open"),
                ("c3", "to do", "open"),
            ],
        );
        set(&d, "clickup:c1", CANCELLED, 100).unwrap();
        crate::review::pick(&d, "clickup:c2", crate::review::DROP, "", "2026-11-10", 100).unwrap();
        crate::worktime::start(&d, "clickup:c1", 150).unwrap();
        // The user marks both done in ClickUp (the app, or the chat).
        let s = crate::clickup_list::ClickupStore::new(&d);
        s.remove("c1", 200).unwrap();
        s.remove("c2", 200).unwrap();
        for id in ["c1", "c2"] {
            let st = of(&d, &format!("clickup:{id}")).unwrap().unwrap();
            assert_eq!(
                (st.status.as_str(), st.by.as_str()),
                (DONE, by::SOURCE),
                "{id}: the close is newer than the pick"
            );
        }
        assert!(
            to_look_up(&d, "clickup", 201).unwrap().is_empty(),
            "AIfred's own close needs no read"
        );
        // The app ends the work right after: it is finished work now.
        assert_eq!(
            crate::worktime::finish(&d, "clickup:c1", 1000)
                .unwrap()
                .map(|w| w.minutes),
            Some(14)
        );
        // A pick after the close wins: the user knows better.
        set(&d, "clickup:c1", UNFINISHED, 2000).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, UNFINISHED);
        // Back in the list (the undo of mark done): the old pick counts again.
        save_clickup(&d, &[("c2", "to do", "open"), ("c3", "to do", "open")]);
        assert_eq!(of(&d, "clickup:c2").unwrap().unwrap().status, CANCELLED);
        // A close that AIfred read later wins the same way; one with no
        // known reason does not.
        set(&d, "clickup:c3", UNFINISHED, 3000).unwrap();
        save_clickup_at(&d, &[("c2", "to do", "open")], 3500);
        assert_eq!(of(&d, "clickup:c3").unwrap().unwrap().status, UNFINISHED);
        save_reason(&d, "clickup", "c3", DONE, "complete", 4000).unwrap();
        assert_eq!(of(&d, "clickup:c3").unwrap().unwrap().status, DONE);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// A weekly review drop closes an item in the list with no pick row.
    /// "Open again" and a pick of To do must still open it.
    #[test]
    fn a_dropped_item_opens_again() {
        let d = temp("status-drop-open");
        save_clickup(&d, &[("c1", "to do", "open"), ("c2", "to do", "open")]);
        for id in ["c1", "c2"] {
            crate::review::pick(
                &d,
                &format!("clickup:{id}"),
                crate::review::DROP,
                "",
                "2026-11-10",
                100,
            )
            .unwrap();
            let st = of(&d, &format!("clickup:{id}")).unwrap().unwrap();
            assert_eq!((st.status.as_str(), st.by.as_str()), (CANCELLED, by::YOU));
        }
        let before = open_again(&d, "clickup:c1", 200).unwrap();
        let st = of(&d, "clickup:c1").unwrap().unwrap();
        assert_eq!((st.status.as_str(), st.by.as_str()), (TODO, by::YOU));
        assert!(open(&d, "clickup:c1"));
        restore(&d, &before, 250).unwrap();
        assert_eq!(of(&d, "clickup:c1").unwrap().unwrap().status, CANCELLED);
        set(&d, "clickup:c2", TODO, 300).unwrap();
        let st = of(&d, "clickup:c2").unwrap().unwrap();
        assert_eq!((st.status.as_str(), st.by.as_str()), (TODO, by::YOU));
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding: an item that left the list while still open in its source
    /// was saved as Done for good, so a later close was never read.
    #[test]
    fn an_item_that_is_still_open_in_its_source_is_read_again() {
        let d = temp("status-still-open");
        assert_eq!(reason_for(TODO), STILL_OPEN);
        assert_eq!(reason_for(WAITING), STILL_OPEN);
        assert_eq!(reason_for(DONE), DONE);
        assert_eq!(reason_for(CANCELLED), CANCELLED);
        assert!(save_reason(&d, "github", "x", TODO, "", 1).is_err());
        save_github(&d, &["acme/shop#3"], 100);
        save_github(&d, &[], 200);
        let key = "github:acme/shop#3";
        assert_eq!(to_look_up(&d, "github", 200).unwrap(), ["acme/shop#3"]);
        save_reason(
            &d,
            "github",
            "acme/shop#3",
            STILL_OPEN,
            "open, not yours now",
            200,
        )
        .unwrap();
        let st = of(&d, key).unwrap().unwrap();
        assert_eq!(
            (st.status.as_str(), st.by.as_str(), st.source.as_deref()),
            (TODO, by::SOURCE, Some("open, not yours now"))
        );
        assert!(
            to_look_up(&d, "github", 200 + RECHECK_SECS - 1)
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            to_look_up(&d, "github", 200 + RECHECK_SECS).unwrap(),
            ["acme/shop#3"],
            "again after a day"
        );
        save_reason(
            &d,
            "github",
            "acme/shop#3",
            STILL_OPEN,
            "",
            200 + RECHECK_SECS,
        )
        .unwrap();
        assert!(
            to_look_up(&d, "github", 200 + STILL_OPEN_SECS + 1)
                .unwrap()
                .is_empty(),
            "not for ever"
        );
        // Five failed reads in a row: AIfred stops, as for a first read.
        let day = 200 + 2 * RECHECK_SECS;
        assert_eq!(to_look_up(&d, "github", day).unwrap(), ["acme/shop#3"]);
        for _ in 0..MAX_TRIES {
            failed_reason(&d, "github", "acme/shop#3").unwrap();
        }
        assert!(
            to_look_up(&d, "github", day + RECHECK_SECS)
                .unwrap()
                .is_empty()
        );
        // A week later it closes as "not planned".
        save_reason(
            &d,
            "github",
            "acme/shop#3",
            CANCELLED,
            "not planned",
            200 + 7 * 86_400,
        )
        .unwrap();
        assert_eq!(of(&d, key).unwrap().unwrap().status, CANCELLED);
        assert!(
            to_look_up(&d, "github", 200 + 8 * 86_400)
                .unwrap()
                .is_empty()
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding: a deleted task left its pick and its "continued in" rows.
    #[test]
    fn a_deleted_task_leaves_no_status_rows() {
        let d = temp("status-delete");
        let s = crate::tasks::TaskStore::new(&d);
        let a = s.add("Rock A", "", "you").unwrap();
        let b = s.add("Task B", "", "you").unwrap();
        let (ka, kb) = (format!("task:{}", a.id), format!("task:{}", b.id));
        set(&d, &ka, UNFINISHED, 10).unwrap();
        set_next(&d, &ka, Some(&kb), 11).unwrap();
        set(&d, &kb, WAITING, 12).unwrap();
        assert_eq!(of(&d, &ka).unwrap().unwrap().next.unwrap().key, kb);
        s.remove(&b.id).unwrap();
        assert!(of(&d, &ka).unwrap().unwrap().next.is_none());
        let rows: i64 = db::read(&d, |c| {
            Ok(c.query_row(
                "SELECT (SELECT COUNT(*) FROM status_next) + (SELECT COUNT(*) FROM item_status WHERE key = ?1)",
                [&kb],
                |r| r.get(0),
            )?)
        })
        .unwrap();
        assert_eq!(rows, 0);
        std::fs::remove_dir_all(&d).unwrap();
    }

    /// Finding: the closed list was copied into several SQL strings.
    #[test]
    fn closed_sql_agrees_with_is_closed() {
        let c = Connection::open_in_memory().unwrap();
        for s in ALL {
            let in_sql: bool = c
                .query_row(&format!("SELECT ?1 IN {CLOSED_SQL}"), [s], |r| r.get(0))
                .unwrap();
            assert_eq!(in_sql, is_closed(s), "{s}");
        }
    }

    /// of() reads the rows of one item, all() reads each table once. For
    /// every way a status can be set, the two must give the same answer.
    #[test]
    fn the_status_of_one_item_is_the_same_as_in_the_status_of_all() {
        let d = temp("status-parity");
        save_clickup(
            &d,
            &[
                ("rule", "delayed", "unstarted"),
                ("you", "to do", "open"),
                ("del", "to do", "open"),
                ("blk", "to do", "open"),
                ("tim", "to do", "open"),
                ("def", "in progress", "custom"),
                ("drop", "to do", "open"),
                ("next", "planned", "open"),
                ("gone", "to do", "open"),
            ],
        );
        set_rule(&d, &rule("delayed", "space", "s1", UNFINISHED), 5).unwrap();
        set(&d, "clickup:you", WAITING, 10).unwrap();
        crate::review::pick(
            &d,
            "clickup:del",
            crate::review::DELEGATE,
            "Sam",
            "2026-11-10",
            20,
        )
        .unwrap();
        crate::review::pick(
            &d,
            "clickup:drop",
            crate::review::DROP,
            "",
            "2026-11-10",
            20,
        )
        .unwrap();
        crate::blockers::add(&d, "clickup:blk", "clickup:tim", "you", "", 30).unwrap();
        crate::worktime::start(&d, "clickup:tim", 40).unwrap();
        set_next(&d, "clickup:rule", Some("clickup:next"), 50).unwrap();
        let s = crate::tasks::TaskStore::new(&d);
        let t1 = s.add("Timer task", "", "you").unwrap();
        crate::worktime::start(&d, &format!("task:{}", t1.id), 60).unwrap();
        let t2 = s.add("Picked task", "", "you").unwrap();
        set(&d, &format!("task:{}", t2.id), WAITING, 70).unwrap();
        let t3 = s.add("Unfinished task", "", "you").unwrap();
        set(&d, &format!("task:{}", t3.id), UNFINISHED, 80).unwrap();
        set_next(&d, &format!("task:{}", t3.id), Some("clickup:next"), 85).unwrap();
        // "gone" left the list and closed as "won't do".
        let mut keep: Vec<(&str, &str, &str)> = vec![
            ("rule", "delayed", "unstarted"),
            ("you", "to do", "open"),
            ("del", "to do", "open"),
            ("blk", "to do", "open"),
            ("tim", "to do", "open"),
            ("def", "in progress", "custom"),
            ("drop", "to do", "open"),
            ("next", "planned", "open"),
        ];
        save_clickup(&d, &keep);
        keep.clear();
        save_reason(&d, "clickup", "gone", CANCELLED, "won't do", 90).unwrap();

        let all = all_fn(&d);
        let by_of: Vec<String> = ["rule", "you", "del", "blk", "tim", "def", "drop", "next"]
            .iter()
            .map(|k| all[&format!("clickup:{k}")].by.clone())
            .collect();
        assert_eq!(
            by_of,
            [
                by::RULE,
                by::YOU,
                by::DELEGATE,
                by::BLOCKER,
                by::TIMER,
                by::DEFAULT,
                by::YOU,
                by::DEFAULT
            ],
            "the fixture covers every way a status is set"
        );
        assert_eq!(all[&format!("task:{}", t1.id)].by, by::TIMER);
        assert_eq!(all[&format!("task:{}", t2.id)].by, by::YOU);
        assert_eq!(all["clickup:next"].continues.len(), 2);
        for (key, status) in &all {
            assert_eq!(
                of(&d, key).unwrap().as_ref(),
                Some(status),
                "of() and all() differ for {key}"
            );
        }
        let gone = of(&d, "clickup:gone").unwrap().unwrap();
        assert_eq!(
            (gone.status.as_str(), gone.by.as_str()),
            (CANCELLED, by::SOURCE)
        );
        let t3 = of(&d, &format!("task:{}", t3.id)).unwrap().unwrap();
        assert_eq!(t3.status, UNFINISHED);
        assert_eq!(t3.next.unwrap().key, "clickup:next");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
