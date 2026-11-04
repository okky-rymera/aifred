//! The ClickUp part of the to-do list. The list is saved in the data folder
//! as clickup-cache.json, so AIfred does not ask ClickUp each time the window loads.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::clickup::{self, ClickupItem, MyTasks};
use crate::files;

/// Change this when the saved list must not be used after an update.
/// 1: tasks in a "done" status are left out. Older lists can have them.
const VERSION: u32 = 1;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    /// The VERSION that saved the list. A missing value is 0.
    #[serde(default)]
    version: u32,
    /// Unix seconds. 0 means never.
    fetched_at: u64,
    /// clickup::account_key of the token that loaded the list. A list from
    /// another token can belong to another ClickUp account, so it is not used.
    #[serde(default)]
    account: String,
    items: Vec<ClickupItem>,
    /// The workspaces whose lists are not complete (too many tasks).
    #[serde(default)]
    cut: Vec<String>,
}

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

    fn cache_path(&self) -> PathBuf {
        self.dir.join("clickup-cache.json")
    }

    /// The saved list, if `account` loaded it and it is younger than
    /// `max_age_secs`. None also when there is no saved list. Use u64::MAX to
    /// get the saved list at any age.
    pub fn cached(&self, account: &str, max_age_secs: u64, now: u64) -> anyhow::Result<Option<ClickupList>> {
        let cache: Cache = files::load(&self.cache_path())?;
        if cache.version != VERSION
            || cache.account != account
            || cache.fetched_at == 0
            || now.saturating_sub(cache.fetched_at) >= max_age_secs
        {
            return Ok(None);
        }
        Ok(Some(view(cache)))
    }

    /// Saves a new list from ClickUp. It replaces the old list, so it also
    /// repairs a broken file.
    pub fn save(&self, account: &str, fetched: MyTasks, now: u64) -> anyhow::Result<ClickupList> {
        let cache =
            Cache { version: VERSION, fetched_at: now, account: account.to_string(), items: fetched.items, cut: fetched.cut };
        files::replace(&self.cache_path(), &cache)?;
        Ok(view(cache))
    }

    /// Removes one task from the saved list, after the user marked it done.
    /// The time of the list does not change. A missing task is not an error.
    pub fn remove(&self, id: &str) -> anyhow::Result<()> {
        files::update(&self.cache_path(), |c: &mut Cache| {
            c.items.retain(|i| i.id != id);
            Ok(())
        })
    }

    /// Removes the saved list. Use it when the token changes: the old list
    /// can belong to another ClickUp account.
    pub fn clear(&self) -> anyhow::Result<()> {
        match std::fs::remove_file(self.cache_path()) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

fn view(mut cache: Cache) -> ClickupList {
    cache.items.sort_by_key(|i| std::cmp::Reverse(i.updated));
    ClickupList { items: cache.items, fetched_at: cache.fetched_at, warning: clickup::cut_warning(&cache.cut) }
}

#[cfg(test)]
mod tests {
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
        }
    }

    fn tasks(items: Vec<ClickupItem>) -> MyTasks {
        MyTasks { items, cut: vec![] }
    }

    #[test]
    fn a_cut_list_keeps_its_warning_when_it_loads_again() {
        let d = temp("list-cut");
        let s = ClickupStore::new(&d);
        s.save("acct", MyTasks { items: vec![item("a", 1)], cut: vec!["Big".into()] }, 1000).unwrap();
        let w = s.cached("acct", u64::MAX, 1000).unwrap().unwrap().warning.unwrap();
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
        assert!(s.cached("acct", 900, 1000).unwrap().is_none(), "nothing saved yet");
        s.save("acct", tasks(vec![item("a", 1), item("b", 2)]), 1000).unwrap();
        let l = s.cached("acct", 900, 1899).unwrap().expect("still fresh");
        assert_eq!(ids(&l), vec!["b", "a"]);
        assert!(s.cached("acct", 900, 1900).unwrap().is_none(), "too old");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn remove_takes_out_one_task_and_keeps_the_time() {
        let d = temp("list-remove");
        let s = ClickupStore::new(&d);
        s.save("acct", tasks(vec![item("a", 1), item("b", 2)]), 1000).unwrap();
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
        assert!(s.cached("other", u64::MAX, 1000).unwrap().is_none(), "another account");
        assert!(s.cached("acct", u64::MAX, 1000).unwrap().is_some());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_list_saved_by_an_older_version_is_not_used() {
        let d = temp("list-version");
        // Saved before VERSION 1: it can hold tasks in a "done" status.
        std::fs::write(d.join("clickup-cache.json"), r#"{"fetched_at":1000,"items":[]}"#).unwrap();
        assert!(ClickupStore::new(&d).cached("acct", u64::MAX, 1000).unwrap().is_none());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_repairs_a_broken_file() {
        let d = temp("list-broken");
        std::fs::write(d.join("clickup-cache.json"), "{ half").unwrap();
        let s = ClickupStore::new(&d);
        assert!(s.cached("acct", u64::MAX, 1).is_err());
        s.save("acct", tasks(vec![item("a", 1)]), 1000).unwrap();
        assert_eq!(ids(&s.cached("acct", u64::MAX, 1000).unwrap().unwrap()), vec!["a"]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
