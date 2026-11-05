//! The ClickUp part of the to-do list. The list is saved in the database
//! (db.rs), so AIfred does not ask ClickUp each time the window loads. A task
//! that leaves the list stays in the database for search, marked as not in the list.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::clickup::{self, ClickupItem, MyTasks};
use crate::db::{self, ListItem};

const SOURCE: &str = "clickup";

/// Change this when the saved list must not be used after an update.
/// 1: tasks in a "done" status are left out. Older lists can have them.
const VERSION: u32 = 1;

/// What the window shows.
#[derive(Debug, Serialize)]
pub struct ClickupList {
    /// The open tasks assigned to the user, the newest change first.
    pub items: Vec<ClickupItem>,
    /// When the list came from ClickUp, in Unix seconds. 0 means never.
    pub fetched_at: u64,
    /// Set when ClickUp could not be asked and this is the older saved list,
    /// or when a workspace has more tasks than AIfred reads.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub struct ClickupStore {
    dir: PathBuf,
}

impl ClickupStore {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    /// The saved list, if `account` loaded it and it is younger than
    /// `max_age_secs`. None also when there is no saved list. Use u64::MAX to
    /// get the saved list at any age.
    pub fn cached(
        &self,
        account: &str,
        max_age_secs: u64,
        now: u64,
    ) -> anyhow::Result<Option<ClickupList>> {
        db::read(&self.dir, |c| {
            let fetch: Option<(String, u64, u32, String)> = c
                .query_row(
                    "SELECT account, fetched_at, version, cut FROM work_fetch WHERE source = ?1",
                    [SOURCE],
                    |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
                )
                .optional()?;
            let Some((saved_account, fetched_at, version, cut)) = fetch else {
                return Ok(None);
            };
            if version != VERSION
                || saved_account != account
                || fetched_at == 0
                || now.saturating_sub(fetched_at) >= max_age_secs
            {
                return Ok(None);
            }
            let cut: Vec<String> = serde_json::from_str(&cut)?;
            Ok(Some(view(
                db::list_data(c, SOURCE, Some(account))?,
                fetched_at,
                &cut,
            )))
        })
    }

    /// Saves a new list from ClickUp. The tasks of another account are
    /// deleted: they can belong to another ClickUp user.
    pub fn save(&self, account: &str, fetched: MyTasks, now: u64) -> anyhow::Result<ClickupList> {
        db::write(&self.dir, |tx| {
            tx.execute(
                "DELETE FROM work_item WHERE source = ?1 AND account != ?2",
                [SOURCE, account],
            )?;
            let rows = fetched
                .items
                .iter()
                .map(|i| {
                    Ok(ListItem {
                        id: i.id.clone(),
                        title: i.title.clone(),
                        place: format!("{} / {}", i.workspace, i.list),
                        url: i.url.clone(),
                        data: serde_json::to_string(i)?,
                        details: i.description.clone(),
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            db::save_list(tx, SOURCE, account, &rows, &fetched.cut, VERSION, now)
        })?;
        Ok(view(fetched.items, now, &fetched.cut))
    }

    /// Takes one task out of the saved list, after the user marked it done.
    /// The task stays for search. The time of the list does not change.
    /// A missing task is not an error.
    pub fn remove(&self, id: &str) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute(
                "UPDATE work_item SET in_list = 0 WHERE source = ?1 AND item_id = ?2",
                [SOURCE, id],
            )?;
            Ok(())
        })
    }

    /// Saved tasks of `account` with words of `query` in the title, the
    /// workspace and list, or the saved description, best match first.
    /// Tasks that left the list only when `include_old` is true.
    pub fn search(
        &self,
        account: &str,
        query: &str,
        include_old: bool,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<db::WorkHit>> {
        db::search_work(
            &self.dir,
            SOURCE,
            Some(account),
            &[],
            query,
            include_old,
            limit,
        )
    }

    /// Saves the description of task `id` for search.
    pub fn save_details(&self, id: &str, details: &str) -> anyhow::Result<()> {
        db::save_details(&self.dir, SOURCE, id, details)
    }

    /// Deletes all saved ClickUp tasks. Use it when the token changes: the old
    /// tasks can belong to another ClickUp account.
    pub fn clear(&self) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = ?1", [SOURCE])?;
            tx.execute("DELETE FROM work_fetch WHERE source = ?1", [SOURCE])?;
            Ok(())
        })
    }
}

/// For the chat tools: downloads a new ClickUp list when the saved one is
/// older than the tool age in the settings. Returns a note for the AI when
/// that fails: the tools then use the older saved list. Does nothing when
/// ClickUp is not connected.
pub async fn refresh_if_old(dir: &Path) -> Option<String> {
    let token = clickup::token(dir)?;
    let account = clickup::account_key(&token);
    let store = ClickupStore::new(dir);
    if let Ok(Some(_)) = store.cached(
        &account,
        crate::settings::tool_max_age_secs(dir),
        crate::files::now(),
    ) {
        return None;
    }
    let fetched = match clickup::Client::new(&token).my_tasks().await {
        Ok(f) => f,
        Err(e) => {
            return Some(format!(
                "Cannot reach ClickUp, so the ClickUp tasks come from the saved list. {e:#}"
            ));
        }
    };
    match store.save(&account, fetched, crate::files::now()) {
        Ok(_) => None,
        Err(e) => Some(format!(
            "Cannot save the new ClickUp list, so the ClickUp tasks come from the saved list. {e:#}"
        )),
    }
}

fn view(mut items: Vec<ClickupItem>, fetched_at: u64, cut: &[String]) -> ClickupList {
    items.sort_by_key(|i| std::cmp::Reverse(i.updated));
    ClickupList {
        items,
        fetched_at,
        warning: clickup::cut_warning(cut),
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::clickup::tests::temp;

    pub(crate) fn item(id: &str, updated: u64) -> ClickupItem {
        ClickupItem {
            id: id.into(),
            custom_id: None,
            title: format!("Task {id}"),
            url: format!("https://app.clickup.com/t/{id}"),
            status: "to do".into(),
            status_color: String::new(),
            list: "Sprint".into(),
            workspace: "Rymera".into(),
            priority: None,
            due: None,
            updated,
            description: None,
        }
    }

    fn tasks(items: Vec<ClickupItem>) -> MyTasks {
        MyTasks { items, cut: vec![] }
    }

    #[test]
    fn a_cut_list_keeps_its_warning_when_it_loads_again() {
        let d = temp("list-cut");
        let s = ClickupStore::new(&d);
        s.save(
            "acct",
            MyTasks {
                items: vec![item("a", 1)],
                cut: vec!["Big".into()],
            },
            1000,
        )
        .unwrap();
        let w = s
            .cached("acct", u64::MAX, 1000)
            .unwrap()
            .unwrap()
            .warning
            .unwrap();
        assert!(w.contains("Big"), "{w}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    fn ids(l: &ClickupList) -> Vec<&str> {
        l.items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn saved_list_is_used_until_it_is_too_old_and_newest_comes_first() {
        let d = temp("list-age");
        let s = ClickupStore::new(&d);
        assert!(
            s.cached("acct", 900, 1000).unwrap().is_none(),
            "nothing saved yet"
        );
        s.save("acct", tasks(vec![item("a", 1), item("b", 2)]), 1000)
            .unwrap();
        let l = s.cached("acct", 900, 1899).unwrap().expect("still fresh");
        assert_eq!(ids(&l), vec!["b", "a"]);
        assert!(s.cached("acct", 900, 1900).unwrap().is_none(), "too old");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn remove_takes_out_one_task_and_keeps_the_time() {
        let d = temp("list-remove");
        let s = ClickupStore::new(&d);
        s.save("acct", tasks(vec![item("a", 1), item("b", 2)]), 1000)
            .unwrap();
        s.remove("a").unwrap();
        s.remove("nope").unwrap();
        let l = s.cached("acct", u64::MAX, 5000).unwrap().unwrap();
        assert_eq!(ids(&l), vec!["b"]);
        assert_eq!(l.fetched_at, 1000);
        s.clear().unwrap();
        assert!(s.cached("acct", u64::MAX, 5000).unwrap().is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_list_from_another_token_is_not_used() {
        let d = temp("list-account");
        let s = ClickupStore::new(&d);
        s.save("acct", tasks(vec![item("a", 1)]), 1000).unwrap();
        assert!(
            s.cached("other", u64::MAX, 1000).unwrap().is_none(),
            "another account"
        );
        assert!(s.cached("acct", u64::MAX, 1000).unwrap().is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_list_saved_by_an_older_version_is_not_used() {
        let d = temp("list-version");
        let s = ClickupStore::new(&d);
        s.save("acct", tasks(vec![item("a", 1)]), 1000).unwrap();
        // Saved before VERSION 1: it can hold tasks in a "done" status.
        db::write(&d, |tx| {
            Ok(tx.execute("UPDATE work_fetch SET version = 0", [])?)
        })
        .unwrap();
        assert!(s.cached("acct", u64::MAX, 1000).unwrap().is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_account_deletes_the_tasks_of_the_old_one() {
        let d = temp("list-new-account");
        let s = ClickupStore::new(&d);
        s.save("old", tasks(vec![item("a", 1), item("b", 1)]), 1000)
            .unwrap();
        s.save("new", tasks(vec![item("c", 1)]), 2000).unwrap();
        let n: i64 = db::read(&d, |c| {
            Ok(c.query_row("SELECT count(*) FROM work_item", [], |r| r.get(0))?)
        })
        .unwrap();
        assert_eq!(n, 1, "only the task of the new account is left");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn search_uses_the_list_name_and_the_saved_description() {
        let d = temp("list-search");
        let s = ClickupStore::new(&d);
        s.save("acct", tasks(vec![item("a", 1), item("b", 2)]), 1000)
            .unwrap();
        s.save_details("b", "Send the invoice to the client")
            .unwrap();
        let ids = |v: Vec<db::WorkHit>| {
            v.into_iter()
                .map(|h| h.item["id"].as_str().unwrap().to_string())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            ids(s.search("acct", "invoices", false, None).unwrap()),
            vec!["b"]
        );
        assert_eq!(
            ids(s.search("acct", "sprint", false, None).unwrap()).len(),
            2,
            "the list name is searched"
        );
        assert!(
            s.search("other", "invoice", false, None)
                .unwrap()
                .is_empty(),
            "another account sees nothing"
        );
        s.remove("b").unwrap();
        assert!(s.search("acct", "invoice", false, None).unwrap().is_empty());
        assert_eq!(s.search("acct", "invoice", true, None).unwrap().len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_done_task_leaves_the_list_but_stays_for_search() {
        let d = temp("list-kept");
        let s = ClickupStore::new(&d);
        s.save("acct", tasks(vec![item("a", 1), item("b", 2)]), 1000)
            .unwrap();
        s.save("acct", tasks(vec![item("b", 2)]), 2000).unwrap();
        assert_eq!(
            ids(&s.cached("acct", u64::MAX, 2000).unwrap().unwrap()),
            vec!["b"]
        );
        let kept: Vec<(String, bool)> = db::read(&d, |c| {
            let mut q = c.prepare("SELECT item_id, in_list FROM work_item ORDER BY item_id")?;
            Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
                .collect::<Result<_, _>>()?)
        })
        .unwrap();
        assert_eq!(kept, vec![("a".into(), false), ("b".into(), true)]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
