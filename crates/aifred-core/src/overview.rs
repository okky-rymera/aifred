//! Everything on the user's plate in one answer: the open local tasks, the
//! saved GitHub list, and the saved ClickUp list, each item with a short
//! summary (summaries.rs). The overview tool of the tasks tool server sends
//! it. One tool call replaces five list calls, and the AI reads summaries
//! and not full descriptions.

use std::path::Path;

use serde_json::{Value, json};

use crate::clickup::ClickupItem;
use crate::db;
use crate::github::GithubItem;
use crate::summaries::for_item;
use crate::tasks::{ChecklistItem, json_column};

/// The overview. `clickup_account` is the account of the ClickUp token
/// (clickup::account_key), or None when ClickUp is not connected.
pub fn build(dir: &Path, clickup_account: Option<&str>) -> anyhow::Result<Value> {
    db::read(dir, |c| {
        let mut q = c.prepare(
            "SELECT id, title, notes, summary, created, due, start, priority, tags, checklist FROM task WHERE done = 0 ORDER BY created DESC",
        )?;
        let tasks: Vec<Value> = q
            .query_map([], |r| {
                let notes: String = r.get(2)?;
                let mut t = json!({ "id": r.get::<_, String>(0)?, "title": r.get::<_, String>(1)?, "created": r.get::<_, i64>(4)? });
                if let Some(s) = for_item(r.get(3)?, &notes) {
                    t["summary"] = s.into();
                }
                // Only the fields that are set, so the answer stays short.
                for (key, i) in [("due", 5), ("start", 6), ("priority", 7)] {
                    if let Some(v) = r.get::<_, Option<String>>(i)? {
                        t[key] = v.into();
                    }
                }
                // The same read as the task list (tasks.rs): a bad value is an error.
                let tags: Vec<String> = json_column(r, 8)?;
                if !tags.is_empty() {
                    t["tags"] = tags.into();
                }
                let list: Vec<ChecklistItem> = json_column(r, 9)?;
                if !list.is_empty() {
                    t["checklist"] = format!("{} of {} done", list.iter().filter(|i| i.done).count(), list.len()).into();
                }
                Ok(t)
            })?
            .collect::<Result<_, _>>()?;

        let github: Vec<Value> = rows(c, "github", None)?
            .into_iter()
            .map(|(data, summary)| {
                let i: GithubItem = serde_json::from_str(&data)?;
                let mut v = json!({
                    "id": i.id, "title": i.title, "url": i.url, "reason": i.reason, "updated": i.updated,
                });
                if !i.links.is_empty() {
                    v["linked"] = i.links.iter().map(|l| l.id.clone()).collect::<Vec<_>>().into();
                }
                if let Some(s) = summary {
                    v["summary"] = s.into();
                }
                Ok(v)
            })
            .collect::<anyhow::Result<_>>()?;

        let mut out = json!({
            "local_tasks": tasks,
            "github": { "fetched_at": fetched_at(c, "github", None)?, "items": github },
        });

        if let Some(account) = clickup_account {
            let mut items: Vec<(ClickupItem, Option<String>)> = rows(c, "clickup", Some(account))?
                .into_iter()
                .map(|(data, summary)| Ok((serde_json::from_str(&data)?, summary)))
                .collect::<anyhow::Result<_>>()?;
            items.sort_by_key(|(i, _)| std::cmp::Reverse(i.updated));
            let tasks: Vec<Value> = items
                .into_iter()
                .map(|(i, summary)| {
                    let mut v = json!({
                        "id": i.id, "title": i.title, "url": i.url, "status": i.status,
                        "list": format!("{} / {}", i.workspace, i.list),
                    });
                    for (key, value) in [("custom_id", i.custom_id.map(Value::from)), ("priority", i.priority.map(Value::from)), ("due", i.due.map(Value::from)), ("summary", summary.map(Value::from))] {
                        if let Some(value) = value {
                            v[key] = value;
                        }
                    }
                    v
                })
                .collect();
            out["clickup"] = json!({ "fetched_at": fetched_at(c, "clickup", Some(account))?, "tasks": tasks });
        }
        Ok(out)
    })
}

/// The data and the summary to show of each item in the saved list of `source`.
fn rows(c: &rusqlite::Connection, source: &str, account: Option<&str>) -> anyhow::Result<Vec<(String, Option<String>)>> {
    let mut q = c.prepare(
        "SELECT data, summary, details FROM work_item
         WHERE source = ?1 AND in_list = 1 AND (?2 IS NULL OR account = ?2) ORDER BY pos",
    )?;
    let rows = q.query_map(rusqlite::params![source, account], |r| {
        let details: String = r.get(2)?;
        Ok((r.get::<_, String>(0)?, for_item(r.get(1)?, &details)))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// When the saved list of `source` came from GitHub or ClickUp (Unix
/// seconds), or 0 when there is none. When `account` is Some, a list of
/// another account counts as none: after the user connects a new ClickUp
/// account, the old download time must not show next to an empty list.
fn fetched_at(c: &rusqlite::Connection, source: &str, account: Option<&str>) -> anyhow::Result<i64> {
    use rusqlite::OptionalExtension;
    Ok(c
        .query_row(
            "SELECT fetched_at FROM work_fetch WHERE source = ?1 AND (?2 IS NULL OR account = ?2)",
            rusqlite::params![source, account],
            |r| r.get(0),
        )
        .optional()?
        .unwrap_or(0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::clickup_list::ClickupStore;
    use crate::github_list::GithubStore;
    use crate::tasks::TaskStore;

    #[test]
    fn a_bad_tag_list_is_an_error_like_in_the_task_list() {
        let d = std::env::temp_dir().join(format!("aifred-overview-bad-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let tasks = TaskStore::new(&d);
        tasks.add("Call Sam", "", "you").unwrap();
        db::write(&d, |tx| Ok(tx.execute("UPDATE task SET tags = 'not json'", [])?)).unwrap();
        assert!(tasks.load().is_err());
        assert!(build(&d, None).is_err(), "the overview does not hide the bad value");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_overview_has_all_open_items_with_their_summaries() {
        let d = std::env::temp_dir().join(format!("aifred-overview-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        let tasks = TaskStore::new(&d);
        let sam = tasks.add("Call Sam", "Before 5pm.", "you").unwrap();
        tasks
            .update(&sam.id, crate::tasks::TaskFields {
                due: Some("2026-12-01".into()),
                priority: Some("urgent".into()),
                tags: Some(vec!["work".into()]),
                checklist: Some(vec![ChecklistItem { text: "Find number".into(), done: true }.into(), ChecklistItem { text: "Call".into(), done: false }.into()]),
                ..Default::default()
            })
            .unwrap();
        let done = tasks.add("Old", "", "you").unwrap();
        tasks.set_done(&done.id, true).unwrap();

        let mut gh: GithubItem = serde_json::from_value(json!({
            "id": "acme/shop#7", "repo": "acme/shop", "number": 7, "title": "Fix cart",
            "url": "u", "reason": "assigned", "updated": "2026-11-01T00:00:00Z"
        }))
        .unwrap();
        gh.body = Some("The cart loses items on reload.".into());
        GithubStore::new(&d).save_fetch(vec![gh], &[], 1000).unwrap();

        let mut cu = crate::clickup_list::tests::item("86a", 5);
        cu.priority = Some("high".into());
        cu.description = Some("Write the notes for the release.".into());
        ClickupStore::new(&d).save("acct", crate::clickup::MyTasks { items: vec![cu], cut: vec![] }, 2000).unwrap();

        let o = build(&d, Some("acct")).unwrap();
        assert_eq!(o["local_tasks"].as_array().unwrap().len(), 1, "done tasks are left out");
        assert_eq!(o["local_tasks"][0]["summary"], "Before 5pm.");
        let t = &o["local_tasks"][0];
        assert_eq!((&t["due"], &t["priority"], &t["tags"], &t["checklist"]), (&json!("2026-12-01"), &json!("urgent"), &json!(["work"]), &json!("1 of 2 done")));
        assert!(t.get("start").is_none(), "a field that is not set is left out");
        assert_eq!(o["github"]["fetched_at"], 1000);
        assert_eq!(o["github"]["items"][0]["summary"], "The cart loses items on reload.", "the download saved the body");
        assert_eq!(o["clickup"]["tasks"][0]["priority"], "high");
        assert_eq!(o["clickup"]["tasks"][0]["summary"], "Write the notes for the release.");
        assert_eq!(o["clickup"]["fetched_at"], 2000);
        assert!(build(&d, None).unwrap().get("clickup").is_none(), "no ClickUp part when it is not connected");
        // A new ClickUp account with no download yet: no time, no tasks.
        let other = build(&d, Some("new-acct")).unwrap();
        assert_eq!(other["clickup"]["fetched_at"], 0, "the old account's time is not shown");
        assert_eq!(other["clickup"]["tasks"].as_array().unwrap().len(), 0);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
