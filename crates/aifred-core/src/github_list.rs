//! The GitHub part of the to-do list, saved in the data folder:
//! - github-cache.json: the last list from GitHub, so aifred does not ask
//!   GitHub each time the window loads.
//! - github-done.json: items the user marked done in aifred.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::files;
use crate::github::GithubItem;

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    /// Unix seconds. 0 means never.
    fetched_at: u64,
    items: Vec<GithubItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DoneItem {
    #[serde(flatten)]
    pub item: GithubItem,
    /// Unix seconds.
    pub done_at: u64,
    /// When a complete fetch first did not list the item (Unix seconds).
    /// None while GitHub still lists it as open.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub gone_since: Option<u64>,
}

/// What the window shows.
#[derive(Debug, Serialize)]
pub struct GithubList {
    /// Open items that are not marked done.
    pub items: Vec<GithubItem>,
    pub done: Vec<DoneItem>,
    /// When the list came from GitHub, in Unix seconds.
    pub fetched_at: u64,
    /// Set when GitHub could not be asked and this is the older saved list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

/// Keep a done item this long after it leaves GitHub's open list.
const KEEP_DONE_SECS: u64 = 7 * 86400;

pub struct GithubStore {
    dir: PathBuf,
}

impl GithubStore {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    fn cache_path(&self) -> PathBuf {
        self.dir.join("github-cache.json")
    }

    fn done_path(&self) -> PathBuf {
        self.dir.join("github-done.json")
    }

    /// The saved list, if it is younger than `max_age_secs`.
    /// None also when there is no saved list yet.
    /// Use u64::MAX to get the saved list at any age.
    pub fn cached(&self, max_age_secs: u64, now: u64) -> anyhow::Result<Option<GithubList>> {
        let cache: Cache = files::load(&self.cache_path())?;
        if cache.fetched_at == 0 || now.saturating_sub(cache.fetched_at) >= max_age_secs {
            return Ok(None);
        }
        self.view(cache).map(Some)
    }

    /// Saves a new list from GitHub. Forgets a done item KEEP_DONE_SECS
    /// after GitHub stops listing it as open.
    ///
    /// `complete` is false when a search hit its limit. Then a missing item
    /// can still be open, so the done items are not touched.
    pub fn save_fetch(&self, items: Vec<GithubItem>, complete: bool, now: u64) -> anyhow::Result<GithubList> {
        if complete {
            files::update(&self.done_path(), |done: &mut Vec<DoneItem>| {
                for d in done.iter_mut() {
                    if items.iter().any(|i| i.id == d.item.id) {
                        d.gone_since = None;
                    } else {
                        d.gone_since.get_or_insert(now);
                    }
                }
                done.retain(|d| d.gone_since.is_none_or(|g| now.saturating_sub(g) < KEEP_DONE_SECS));
                Ok(())
            })?;
        }
        let cache = Cache { fetched_at: now, items };
        files::update(&self.cache_path(), |c: &mut Cache| {
            c.fetched_at = cache.fetched_at;
            c.items = cache.items.clone();
            Ok(())
        })?;
        self.view(cache)
    }

    fn view(&self, cache: Cache) -> anyhow::Result<GithubList> {
        let mut done: Vec<DoneItem> = files::load(&self.done_path())?;
        done.sort_by_key(|d| std::cmp::Reverse(d.done_at));
        let items = cache.items.into_iter().filter(|i| !done.iter().any(|d| d.item.id == i.id)).collect();
        Ok(GithubList { items, done, fetched_at: cache.fetched_at, warning: None })
    }

    /// Marks an item from the saved list as done. Returns the item.
    pub fn mark_done(&self, id: &str, now: u64) -> anyhow::Result<GithubItem> {
        let cache: Cache = files::load(&self.cache_path())?;
        let item = cache
            .items
            .into_iter()
            .find(|i| i.id == id)
            .ok_or_else(|| anyhow::anyhow!("{id} is not in the GitHub list. Reload the list."))?;
        files::update(&self.done_path(), |done: &mut Vec<DoneItem>| {
            if !done.iter().any(|d| d.item.id == id) {
                done.push(DoneItem { item: item.clone(), done_at: now, gone_since: None });
            }
            Ok(())
        })?;
        Ok(item)
    }

    /// A done item, for the close step. Only done items can be closed.
    pub fn done_item(&self, id: &str) -> anyhow::Result<GithubItem> {
        let done: Vec<DoneItem> = files::load(&self.done_path())?;
        done.into_iter()
            .find(|d| d.item.id == id)
            .map(|d| d.item)
            .ok_or_else(|| anyhow::anyhow!("{id} is not marked done"))
    }

    /// The item is closed on GitHub: take it out of the saved open list.
    pub fn forget_open(&self, id: &str) -> anyhow::Result<()> {
        files::update(&self.cache_path(), |c: &mut Cache| {
            c.items.retain(|i| i.id != id);
            Ok(())
        })
    }

    pub fn undo(&self, id: &str) -> anyhow::Result<()> {
        files::update(&self.done_path(), |done: &mut Vec<DoneItem>| {
            let before = done.len();
            done.retain(|d| d.item.id != id);
            if done.len() == before {
                anyhow::bail!("{id} is not marked done");
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store(name: &str) -> (GithubStore, PathBuf) {
        let d = std::env::temp_dir().join(format!("aifred-ghlist-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        (GithubStore::new(&d), d)
    }

    fn item(n: u64) -> GithubItem {
        GithubItem {
            id: format!("acme/shop#{n}"),
            repo: "acme/shop".into(),
            number: n,
            title: format!("Item {n}"),
            url: format!("https://github.com/acme/shop/issues/{n}"),
            reason: "assigned".into(),
            updated: "2026-10-01T10:00:00Z".into(),
        }
    }

    #[test]
    fn saved_list_is_used_until_it_is_too_old() {
        let (s, d) = store("age");
        assert!(s.cached(900, 1000).unwrap().is_none(), "nothing saved yet");
        s.save_fetch(vec![item(1)], true, 1000).unwrap();
        let l = s.cached(900, 1000 + 899).unwrap().expect("still fresh");
        assert_eq!(l.items.len(), 1);
        assert_eq!(l.fetched_at, 1000);
        assert!(s.cached(900, 1000 + 900).unwrap().is_none(), "too old");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn done_items_leave_the_open_list_and_undo_brings_them_back() {
        let (s, d) = store("done");
        s.save_fetch(vec![item(1), item(2)], true, 1000).unwrap();
        assert_eq!(s.mark_done("acme/shop#1", 1100).unwrap().number, 1);
        let l = s.cached(u64::MAX, 1200).unwrap().unwrap();
        assert_eq!(l.items.iter().map(|i| i.number).collect::<Vec<_>>(), vec![2]);
        assert_eq!(l.done.len(), 1);
        assert_eq!(l.done[0].done_at, 1100);

        // A new fetch that still lists item 1 keeps it hidden.
        let l = s.save_fetch(vec![item(1), item(2)], true, 1300).unwrap();
        assert_eq!(l.items.len(), 1);

        s.undo("acme/shop#1").unwrap();
        assert_eq!(s.cached(u64::MAX, 1400).unwrap().unwrap().items.len(), 2);
        assert!(s.undo("acme/shop#1").is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn only_known_items_can_be_marked_done_or_closed() {
        let (s, d) = store("known");
        s.save_fetch(vec![item(1)], true, 1000).unwrap();
        assert!(s.mark_done("evil/repo#9", 1100).is_err());
        assert!(s.done_item("acme/shop#1").is_err(), "not done yet, so it cannot be closed");
        s.mark_done("acme/shop#1", 1100).unwrap();
        assert_eq!(s.done_item("acme/shop#1").unwrap().number, 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn closed_item_leaves_the_saved_list() {
        let (s, d) = store("closed");
        s.save_fetch(vec![item(1), item(2)], true, 1000).unwrap();
        s.forget_open("acme/shop#1").unwrap();
        assert_eq!(s.cached(u64::MAX, 1100).unwrap().unwrap().items.len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn old_done_items_are_forgotten_when_github_drops_them() {
        let (s, d) = store("prune");
        s.save_fetch(vec![item(1), item(2)], true, 1000).unwrap();
        s.mark_done("acme/shop#1", 1000).unwrap();
        s.mark_done("acme/shop#2", 1000).unwrap();
        // Item 1 is closed on GitHub, item 2 is still open.
        let l = s.save_fetch(vec![item(2)], true, 2000).unwrap();
        assert_eq!(l.done.len(), 2, "kept for a week after it leaves GitHub's open list");
        let l = s.save_fetch(vec![item(2)], true, 2000 + KEEP_DONE_SECS - 1).unwrap();
        assert_eq!(l.done.len(), 2);
        let l = s.save_fetch(vec![item(2)], true, 2000 + KEEP_DONE_SECS).unwrap();
        assert_eq!(l.done.iter().map(|d| d.item.number).collect::<Vec<_>>(), vec![2]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_week_starts_when_the_item_closes_not_when_it_was_ticked() {
        let (s, d) = store("week");
        s.save_fetch(vec![item(1)], true, 1000).unwrap();
        s.mark_done("acme/shop#1", 1000).unwrap();
        // Still open on GitHub for 30 days, then closed.
        s.save_fetch(vec![item(1)], true, 1000 + 30 * 86400).unwrap();
        let l = s.save_fetch(vec![], true, 1000 + 31 * 86400).unwrap();
        assert_eq!(l.done.len(), 1, "not forgotten at once");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_that_comes_back_starts_the_week_again() {
        let (s, d) = store("back");
        s.save_fetch(vec![item(1)], true, 1000).unwrap();
        s.mark_done("acme/shop#1", 1000).unwrap();
        s.save_fetch(vec![], true, 2000).unwrap();
        s.save_fetch(vec![item(1)], true, 3000).unwrap();
        let l = s.save_fetch(vec![], true, 2000 + KEEP_DONE_SECS).unwrap();
        assert_eq!(l.done.len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_fetch_that_hit_the_search_limit_never_forgets_done_items() {
        let (s, d) = store("limit");
        s.save_fetch(vec![item(1)], true, 1000).unwrap();
        s.mark_done("acme/shop#1", 1000).unwrap();
        // Item 1 fell past the search limit, so it is missing but still open.
        s.save_fetch(vec![item(2)], false, 2000).unwrap();
        let l = s.save_fetch(vec![item(2)], false, 2000 + 30 * 86400).unwrap();
        assert_eq!(l.done.len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
