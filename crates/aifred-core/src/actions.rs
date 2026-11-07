//! The log of AIfred's actions in other apps (plan step 8 in
//! docs/plan-assistant.md). Each change that AIfred makes outside the
//! computer gets one row, with what it wrote and the ids that Undo needs.
//! The actions are Gmail drafts (drafts.rs), suggestions in a Google Doc
//! (docs.rs), and comments in a Google Sheet (sheets.rs).

use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Row};
use serde::Serialize;
use serde_json::Value;

use crate::db;

/// The most actions that list() gives.
pub const MAX_LIST: usize = 100;

/// One action of AIfred in another app.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Action {
    pub id: i64,
    /// Unix seconds.
    pub at: u64,
    /// "gmail", "docs", or "sheets".
    pub app: String,
    /// "draft" (gmail), "suggestion" (docs), or "comment" (sheets).
    pub kind: String,
    /// The key of the item that the action is for.
    pub item: String,
    /// What AIfred did, in one line.
    pub title: String,
    /// The text that AIfred wrote, as the user saw it before the save.
    pub after: String,
    /// The place of the change in the other app.
    pub url: String,
    /// When Undo ran. None: not undone.
    pub undone_at: Option<u64>,
    /// Why Undo did not remove the change, for example "you sent the draft".
    pub note: String,
    /// The ids in the other app. Not for the window.
    #[serde(skip)]
    pub target: Value,
}

/// A new action, before it has an id.
pub struct New<'a> {
    pub app: &'a str,
    pub kind: &'a str,
    pub item: &'a str,
    pub title: &'a str,
    pub after: &'a str,
    pub url: &'a str,
    pub target: Value,
}

const COLUMNS: &str = "id, at, app, kind, item, title, after, url, undone_at, note, target";

fn from_row(r: &Row) -> rusqlite::Result<Action> {
    let target: String = r.get(10)?;
    Ok(Action {
        id: r.get(0)?,
        at: r.get::<_, i64>(1)? as u64,
        app: r.get(2)?,
        kind: r.get(3)?,
        item: r.get(4)?,
        title: r.get(5)?,
        after: r.get(6)?,
        url: r.get(7)?,
        undone_at: r.get::<_, Option<i64>>(8)?.map(|t| t as u64),
        note: r.get(9)?,
        target: serde_json::from_str(&target).unwrap_or(Value::Null),
    })
}

/// Saves an action, and gives it with its id.
pub fn record(dir: &Path, a: New, now: u64) -> anyhow::Result<Action> {
    db::write(dir, |tx| {
        tx.execute(
            "INSERT INTO action (at, app, kind, item, title, after, target, url) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                now as i64,
                a.app,
                a.kind,
                a.item,
                a.title,
                a.after,
                a.target.to_string(),
                a.url
            ],
        )?;
        let id = tx.last_insert_rowid();
        Ok(get_in(tx, id)?.expect("the new action"))
    })
}

fn get_in(c: &Connection, id: i64) -> anyhow::Result<Option<Action>> {
    Ok(c.query_row(
        &format!("SELECT {COLUMNS} FROM action WHERE id = ?1"),
        [id],
        from_row,
    )
    .optional()?)
}

pub fn get(dir: &Path, id: i64) -> anyhow::Result<Option<Action>> {
    db::read(dir, |c| get_in(c, id))
}

/// The newest actions first, at most MAX_LIST. With `item`: only the
/// actions of that item.
pub fn list(dir: &Path, item: Option<&str>) -> anyhow::Result<Vec<Action>> {
    // Two queries: with "?1 IS NULL OR item = ?1", SQLite cannot use the
    // index action_by_item.
    let order = format!("ORDER BY at DESC, id DESC LIMIT {MAX_LIST}");
    db::read(dir, |c| {
        let rows = match item {
            Some(item) => c
                .prepare(&format!(
                    "SELECT {COLUMNS} FROM action WHERE item = ?1 {order}"
                ))?
                .query_map([item], from_row)?
                .collect::<Result<Vec<_>, _>>()?,
            None => c
                .prepare(&format!("SELECT {COLUMNS} FROM action {order}"))?
                .query_map([], from_row)?
                .collect::<Result<Vec<_>, _>>()?,
        };
        Ok(rows)
    })
}

/// Marks an action as undone, with a note when the change was not removed.
/// It fails when the action was undone already: two Undo clicks do one Undo.
pub fn set_undone(dir: &Path, id: i64, note: &str, now: u64) -> anyhow::Result<Action> {
    db::write(dir, |tx| {
        let n = tx.execute(
            "UPDATE action SET undone_at = ?2, note = ?3 WHERE id = ?1 AND undone_at IS NULL",
            rusqlite::params![id, now as i64, note],
        )?;
        if n == 0 {
            bail!("this action is undone already");
        }
        Ok(get_in(tx, id)?.expect("the action"))
    })
}

/// Changes the target of an action that is not undone yet, for example to
/// keep what a partial Undo did for the next Undo.
pub fn set_target(dir: &Path, id: i64, target: &Value) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        let n = tx.execute(
            "UPDATE action SET target = ?2 WHERE id = ?1 AND undone_at IS NULL",
            rusqlite::params![id, target.to_string()],
        )?;
        if n == 0 {
            bail!("this action is undone already");
        }
        Ok(())
    })
}

/// The checks of an Undo before it calls the app: the action is not undone
/// yet, and it is of the app `app` and the kind `kind`.
pub fn check_undo(a: &Action, app: &str, kind: &str) -> anyhow::Result<()> {
    if a.undone_at.is_some() {
        bail!("this action is undone already");
    }
    if (a.app.as_str(), a.kind.as_str()) != (app, kind) {
        bail!("AIfred cannot undo this action");
    }
    Ok(())
}

/// The account of the action, when Google is now connected as another
/// account. In another account, the app does not find the change, and
/// Undo would wrongly say that it is gone. `now`: the connected account.
/// Each app finds `now` in its own way: Gmail from the saved addresses,
/// Google Docs and Google Sheets from the Google login.
pub fn other_account<'a>(a: &'a Action, now: &str) -> Option<&'a str> {
    a.target["account"]
        .as_str()
        .filter(|s| !s.is_empty() && *s != now)
}

/// Undoes an action in its app: deletes the Gmail draft, the Doc
/// suggestions, or the Sheet comments.
pub async fn undo(dir: &Path, id: i64, now: u64) -> anyhow::Result<Action> {
    // One Undo at a time. Two clicks at the same time must not both delete.
    let _one = UNDO.lock().await;
    let a = crate::files::off_thread(dir, move |d| get(d, id))
        .await?
        .ok_or_else(|| anyhow::anyhow!("there is no such action"))?;
    match a.app.as_str() {
        "gmail" => crate::drafts::undo(dir, a, now).await,
        "docs" => crate::docs::undo(dir, a, now).await,
        "sheets" => crate::sheets::undo(dir, a, now).await,
        app => bail!("AIfred cannot undo an action in \"{app}\" yet"),
    }
}

static UNDO: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gmail::tests::temp;
    use serde_json::json;

    fn new(item: &str) -> New<'_> {
        New {
            app: "gmail",
            kind: "draft",
            item,
            title: "Reply draft to ana@y.com",
            after: "Hi Ana",
            url: "https://mail.google.com/x",
            target: json!({"draft": "d1"}),
        }
    }

    #[test]
    fn actions_are_logged_newest_first_and_undone_once() {
        let d = temp("actions-log");
        let a = record(&d, new("gmail:t1"), 100).unwrap();
        let b = record(&d, new("gmail:t2"), 200).unwrap();
        assert_eq!(a.target["draft"], "d1");
        assert_eq!(a.undone_at, None);
        let all = list(&d, None).unwrap();
        assert_eq!(all.iter().map(|x| x.id).collect::<Vec<_>>(), [b.id, a.id]);
        assert_eq!(
            list(&d, Some("gmail:t1")).unwrap(),
            std::slice::from_ref(&a)
        );
        let u = set_undone(&d, a.id, "", 300).unwrap();
        assert_eq!(u.undone_at, Some(300));
        assert!(set_undone(&d, a.id, "", 400).is_err(), "only one Undo");
        assert!(set_undone(&d, 999, "", 400).is_err());
        assert_eq!(get(&d, a.id).unwrap().unwrap().undone_at, Some(300));
        std::fs::remove_dir_all(&d).unwrap();
    }
}
