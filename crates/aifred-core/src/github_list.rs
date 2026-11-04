//! The GitHub part of the to-do list. The sync goes one way only: AIfred reads
//! GitHub and never changes it. The list is saved in the database (db.rs),
//! so AIfred does not ask GitHub each time the window loads. An item that
//! leaves the list stays in the database for search, marked as not in the list.

use std::path::{Path, PathBuf};

use rusqlite::OptionalExtension;
use serde::Serialize;

use crate::db::{self, ListItem};
use crate::github::{GithubItem, Link};

const SOURCE: &str = "github";

/// What the window shows.
#[derive(Debug, Serialize)]
pub struct GithubList {
    /// The open items, with each issue and its pull requests joined in one row.
    pub items: Vec<GithubItem>,
    /// When the list came from GitHub, in Unix seconds.
    pub fetched_at: u64,
    /// Set when GitHub could not be asked and this is the older saved list.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub warning: Option<String>,
}

pub struct GithubStore {
    dir: PathBuf,
}

impl GithubStore {
    pub fn new(dir: &Path) -> Self {
        Self { dir: dir.into() }
    }

    /// The saved list, if it is younger than `max_age_secs`.
    /// None also when there is no saved list yet.
    /// Use u64::MAX to get the saved list at any age.
    pub fn cached(&self, max_age_secs: u64, now: u64) -> anyhow::Result<Option<GithubList>> {
        db::read(&self.dir, |c| {
            let fetched_at: Option<u64> =
                c.query_row("SELECT fetched_at FROM work_fetch WHERE source = ?1", [SOURCE], |r| r.get(0)).optional()?;
            match fetched_at {
                Some(f) if f != 0 && now.saturating_sub(f) < max_age_secs => {
                    Ok(Some(view(db::list_data(c, SOURCE, None)?, f)))
                }
                _ => Ok(None),
            }
        })
    }

    /// Saves a new list from GitHub.
    ///
    /// GitHub did not send the links of the items in `no_links`. They get the
    /// links from the saved item, so their rows do not change shape. The read
    /// and the save are one transaction, so a second fetch at the same time
    /// (the timer and the reload button) cannot mix old and new links.
    ///
    /// A saved item that cannot be read counts as no saved links: the new list
    /// from GitHub is more important. The save then repairs the item.
    pub fn save_fetch(&self, mut items: Vec<GithubItem>, no_links: &[String], now: u64) -> anyhow::Result<GithubList> {
        db::write(&self.dir, |tx| {
            if !no_links.is_empty() {
                let mut q = tx.prepare("SELECT data FROM work_item WHERE source = ?1 AND item_id = ?2")?;
                for item in items.iter_mut().filter(|i| no_links.contains(&i.id)) {
                    let Some(data) = q.query_row([SOURCE, &item.id], |r| r.get::<_, String>(0)).optional()? else {
                        continue;
                    };
                    match serde_json::from_str::<GithubItem>(&data) {
                        Ok(saved) => keep_saved_links(item, &saved),
                        Err(e) => eprintln!("aifred: cannot use the saved GitHub links of {}: {e}", item.id),
                    }
                }
            }
            let rows = items
                .iter()
                .map(|i| {
                    Ok(ListItem {
                        id: i.id.clone(),
                        title: i.title.clone(),
                        place: i.repo.clone(),
                        url: i.url.clone(),
                        data: serde_json::to_string(i)?,
                        details: i.body.clone(),
                    })
                })
                .collect::<anyhow::Result<Vec<_>>>()?;
            db::save_list(tx, SOURCE, "", &rows, &[], 0, now)
        })?;
        Ok(view(items, now))
    }

    /// For the chat tools: downloads a new list when the saved one is older
    /// than the tool age in the settings. Returns a note for the AI when that
    /// fails: the tools then use the older saved list.
    pub async fn refresh_if_old(&self) -> Option<String> {
        let now = crate::files::now();
        if let Ok(Some(_)) = self.cached(crate::settings::tool_max_age_secs(&self.dir), now) {
            return None;
        }
        let fetched = match crate::github::my_items(&crate::github::owners_from_env()).await {
            Ok(f) => f,
            Err(e) => return Some(format!("Cannot reach GitHub, so the GitHub items come from the saved list. {e:#}")),
        };
        match self.save_fetch(fetched.items, &fetched.no_links, crate::files::now()) {
            Ok(_) => None,
            Err(e) => Some(format!("Cannot save the new GitHub list, so the GitHub items come from the saved list. {e:#}")),
        }
    }

    /// Saved items with words of `query` in the title, the repository, or the
    /// saved description, best match first. Items that left the list
    /// (for example closed issues) only when `include_old` is true. A
    /// non-empty `owners` (lowercase) keeps only the repositories of those owners.
    pub fn search(
        &self,
        query: &str,
        owners: &[String],
        include_old: bool,
        limit: Option<u32>,
    ) -> anyhow::Result<Vec<db::WorkHit>> {
        db::search_work(&self.dir, SOURCE, None, owners, query, include_old, limit)
    }

    /// Saves the description of item `id` ("owner/repo#number") for search.
    pub fn save_details(&self, id: &str, details: &str) -> anyhow::Result<()> {
        db::save_details(&self.dir, SOURCE, id, details)
    }

    /// Removes the files of the old "done" feature. AIfred does not change
    /// GitHub, so it no longer hides GitHub items: an item leaves the list
    /// when it is closed on GitHub.
    pub fn remove_old_files(&self) {
        for name in ["github-done.json", "github-done.lock"] {
            match std::fs::remove_file(self.dir.join(name)) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    eprintln!("aifred: cannot remove {name}: {e}");
                }
                _ => {}
            }
        }
    }
}

fn keep_saved_links(item: &mut GithubItem, saved: &GithubItem) {
    item.links = saved.links.clone();
    item.more_links = saved.more_links;
}

fn view(items: Vec<GithubItem>, fetched_at: u64) -> GithubList {
    GithubList { items: join_linked(items), fetched_at, warning: None }
}

/// Shows an issue and the pull requests that close it as one row.
///
/// - Your own pull request goes inside your issues when every issue it closes
///   is in the list: it is not a row, and each issue links to it. A pull
///   request that closes two of your issues goes inside both.
/// - Your pull request that also closes an issue that is not in the list
///   stays a row, so its link to that issue stays visible. Your issue still
///   links to it. This is also true when GitHub did not send all its links.
/// - A review request always stays a row, because the review is a separate
///   job. The issue it closes still links to it.
/// - A link to an item that is also in the list gets that item's reason.
/// - An issue row with a pull request inside it shows the newer of the two
///   "updated" times, so new activity on the pull request is visible.
fn join_linked(items: Vec<GithubItem>) -> Vec<GithubItem> {
    use std::collections::{HashMap, HashSet};
    let issues: HashSet<String> = items.iter().filter(|i| !i.is_pull_request()).map(|i| i.id.clone()).collect();
    let reasons: HashMap<String, String> = items.iter().map(|i| (i.id.clone(), i.reason.clone())).collect();
    let mut joined: HashMap<String, Vec<Link>> = HashMap::new();
    // The newest "updated" time of the pull requests inside each issue.
    let mut newest: HashMap<String, String> = HashMap::new();
    let mut rows = Vec::with_capacity(items.len());
    for item in items {
        let mine: Vec<String> = if item.is_pull_request() {
            item.links.iter().filter(|l| issues.contains(&l.id)).map(|l| l.id.clone()).collect()
        } else {
            Vec::new()
        };
        if mine.is_empty() {
            rows.push(item);
            continue;
        }
        let as_link = Link::from(&item);
        let inside = item.reason != "review_requested" && !item.more_links && mine.len() == item.links.len();
        for id in mine {
            if inside {
                let t = newest.entry(id.clone()).or_default();
                // GitHub's times are all UTC ("2026-10-01T10:00:00Z"), so the text order is the time order.
                if item.updated > *t {
                    *t = item.updated.clone();
                }
            }
            joined.entry(id).or_default().push(as_link.clone());
        }
        if !inside {
            rows.push(item);
        }
    }
    for row in &mut rows {
        if let Some(t) = newest.remove(&row.id).filter(|t| *t > row.updated) {
            row.updated = t;
        }
        for l in joined.remove(&row.id).unwrap_or_default() {
            // GitHub can send the same link from both sides. Keep one.
            if !row.links.iter().any(|x| x.id == l.id) {
                row.links.push(l);
            }
        }
        for l in &mut row.links {
            l.reason = reasons.get(&l.id).cloned();
        }
    }
    rows
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
            links: Vec::new(),
            more_links: false,
            body: None,
        }
    }

    fn pr(n: u64, reason: &str, closes: &[u64]) -> GithubItem {
        GithubItem {
            url: format!("https://github.com/acme/shop/pull/{n}"),
            reason: reason.into(),
            links: closes.iter().map(|&i| link(&item(i))).collect(),
            ..item(n)
        }
    }

    fn link(i: &GithubItem) -> Link {
        Link::from(i)
    }

    fn ids(items: &[GithubItem]) -> Vec<u64> {
        items.iter().map(|i| i.number).collect()
    }

    fn link_ids(i: &GithubItem) -> Vec<(u64, Option<&str>)> {
        i.links.iter().map(|l| (l.number, l.reason.as_deref())).collect()
    }

    #[test]
    fn your_pr_goes_inside_your_issue() {
        // The case from the screenshot: issue 142 is assigned to you, and your PR 143 closes it.
        let l = join_linked(vec![item(142), pr(143, "your_pr", &[142])]);
        assert_eq!(ids(&l), vec![142], "one row, not two");
        assert_eq!(link_ids(&l[0]), vec![(143, Some("your_pr"))]);
        assert!(l[0].links[0].pull_request);
    }

    #[test]
    fn issue_row_shows_the_newer_time_of_the_pr_inside() {
        let mut p = pr(143, "your_pr", &[142]);
        p.updated = "2026-10-20T08:00:00Z".into();
        let l = join_linked(vec![item(142), p]);
        assert_eq!(l[0].updated, "2026-10-20T08:00:00Z", "the PR got comments after the issue changed");
        // An older PR does not change the time.
        let mut p = pr(143, "your_pr", &[142]);
        p.updated = "2026-09-01T08:00:00Z".into();
        let l = join_linked(vec![item(142), p]);
        assert_eq!(l[0].updated, "2026-10-01T10:00:00Z");
    }

    #[test]
    fn old_done_files_are_removed() {
        let (s, d) = store("old-done");
        std::fs::write(d.join("github-done.json"), b"[]").unwrap();
        s.remove_old_files();
        assert!(!d.join("github-done.json").exists());
        s.remove_old_files(); // No error when the file is not there.
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn pr_for_an_issue_that_is_not_yours_stays_a_row() {
        let l = join_linked(vec![pr(143, "your_pr", &[142])]);
        assert_eq!(ids(&l), vec![143]);
        assert_eq!(link_ids(&l[0]), vec![(142, None)], "it shows a link to the issue");
    }

    #[test]
    fn issue_with_two_prs_shows_both() {
        // PR 150 is not in your list. GitHub sent it as a link of the issue.
        let mut issue = item(142);
        issue.links = vec![link(&pr(150, "", &[]))];
        let l = join_linked(vec![issue, pr(143, "your_pr", &[142]), pr(144, "your_pr", &[142])]);
        assert_eq!(ids(&l), vec![142]);
        assert_eq!(link_ids(&l[0]), vec![(150, None), (143, Some("your_pr")), (144, Some("your_pr"))]);
    }

    #[test]
    fn review_request_for_your_issue_keeps_its_row() {
        let l = join_linked(vec![pr(150, "review_requested", &[142]), item(142)]);
        assert_eq!(ids(&l), vec![150, 142], "two rows: the review and the issue");
        assert_eq!(link_ids(&l[0]), vec![(142, Some("assigned"))]);
        assert_eq!(link_ids(&l[1]), vec![(150, Some("review_requested"))], "the issue still links to the review");
    }

    #[test]
    fn pr_for_two_of_your_issues_goes_inside_both() {
        let l = join_linked(vec![item(1), item(2), pr(3, "your_pr", &[1, 2])]);
        assert_eq!(ids(&l), vec![1, 2]);
        assert_eq!(link_ids(&l[0]), vec![(3, Some("your_pr"))]);
        assert_eq!(link_ids(&l[1]), vec![(3, Some("your_pr"))]);
    }

    #[test]
    fn pr_for_one_of_your_issues_and_one_other_stays_a_row() {
        // Inside issue 1 only, the link to issue 2 would show nowhere.
        let l = join_linked(vec![item(1), pr(3, "your_pr", &[1, 2])]);
        assert_eq!(ids(&l), vec![1, 3]);
        assert_eq!(link_ids(&l[0]), vec![(3, Some("your_pr"))]);
        assert_eq!(link_ids(&l[1]), vec![(1, Some("assigned")), (2, None)]);
    }

    #[test]
    fn a_link_that_github_sent_both_ways_shows_once() {
        let mut issue = item(142);
        issue.links = vec![link(&pr(143, "", &[]))];
        let l = join_linked(vec![issue, pr(143, "your_pr", &[142])]);
        assert_eq!(link_ids(&l[0]), vec![(143, Some("your_pr"))]);
    }

    #[test]
    fn failed_link_lookup_keeps_the_saved_links() {
        let (s, d) = store("keep-links");
        s.save_fetch(vec![item(142), pr(143, "your_pr", &[142])], &[], 1000).unwrap();
        // The next fetch did not get the links of 143.
        let items = vec![item(142), pr(143, "your_pr", &[])];
        let l = s.save_fetch(items, &["acme/shop#143".into()], 2000).unwrap();
        assert_eq!(ids(&l.items), vec![142], "the PR stays inside the issue");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn pr_with_more_links_than_github_sent_stays_a_row() {
        // GitHub sent only the link to 1. The PR closes other issues too.
        let mut p = pr(3, "your_pr", &[1]);
        p.more_links = true;
        let l = join_linked(vec![item(1), p]);
        assert_eq!(ids(&l), vec![1, 3], "the PR keeps its row");
        assert_eq!(link_ids(&l[0]), vec![(3, Some("your_pr"))], "the issue still links to it");
    }

    #[test]
    fn a_broken_saved_item_does_not_stop_a_new_fetch() {
        let (s, d) = store("broken");
        s.save_fetch(vec![item(1)], &[], 1000).unwrap();
        db::write(&d, |tx| Ok(tx.execute("UPDATE work_item SET data = '{ half'", [])?)).unwrap();
        let l = s.save_fetch(vec![item(1)], &["acme/shop#1".into()], 2000).unwrap();
        assert_eq!(ids(&l.items), vec![1]);
        assert_eq!(ids(&s.cached(u64::MAX, 2000).unwrap().unwrap().items), vec![1], "the item is repaired");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn search_finds_saved_items_and_old_ones_only_when_asked() {
        let (s, d) = store("search");
        let mut a = item(1);
        a.title = "Fix the checkout button".into();
        let mut b = item(2);
        b.title = "Update docs".into();
        s.save_fetch(vec![a, b], &[], 1000).unwrap();
        s.save_details("acme/shop#2", "The checkout page needs a new screenshot.").unwrap();
        let found = |q: &str, old: bool| -> Vec<u64> {
            s.search(q, &[], old, None).unwrap().iter().map(|h| h.item["number"].as_u64().unwrap()).collect()
        };
        assert_eq!(found("checkout", false), vec![1, 2], "the title counts more than the description");
        assert_eq!(found("acme", false).len(), 2, "the repository is searched");
        s.save_fetch(vec![item(2)], &[], 2000).unwrap();
        assert_eq!(found("checkout button", false), vec![2], "item 1 left the list");
        let hits = s.search("button", &[], true, None).unwrap();
        assert_eq!((hits[0].in_list, hits[0].last_seen), (false, 1000));
        let v = s.search("button", &[], true, None).unwrap().remove(0).into_json();
        assert_eq!((v["number"].as_u64(), v["in_list"].as_bool(), v["last_seen"].as_u64()), (Some(1), Some(false), Some(1000)));
        assert_eq!(found("screenshot", false), vec![2], "a new download keeps the description");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_owner_limit_does_not_use_up_result_slots() {
        // The best word matches are from another owner. With limit 2, a
        // filter after the search would give back nothing.
        let (s, d) = store("owners");
        let mut items: Vec<GithubItem> = (1..=3)
            .map(|n| GithubItem {
                id: format!("other/shop#{n}"),
                repo: "other/shop".into(),
                title: "Checkout checkout checkout".into(),
                ..item(n)
            })
            .collect();
        let mut ours = item(9);
        ours.title = "Fix the checkout page".into();
        items.push(ours);
        s.save_fetch(items, &[], 1000).unwrap();
        let hits = s.search("checkout", &["acme".into()], false, Some(2)).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].item["id"], "acme/shop#9");
        assert_eq!(s.search("checkout", &[], false, Some(10)).unwrap().len(), 4, "no limit: all owners");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_that_leaves_the_list_is_kept_for_search() {
        let (s, d) = store("kept");
        s.save_fetch(vec![item(1), item(2)], &[], 1000).unwrap();
        s.save_fetch(vec![item(2)], &[], 2000).unwrap();
        let rows: Vec<(String, bool, i64, i64)> = db::read(&d, |c| {
            let mut q = c.prepare("SELECT item_id, in_list, first_seen, last_seen FROM work_item ORDER BY item_id")?;
            Ok(q.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)))?.collect::<Result<_, _>>()?)
        })
        .unwrap();
        assert_eq!(rows, vec![("acme/shop#1".into(), false, 1000, 1000), ("acme/shop#2".into(), true, 1000, 2000)]);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn saved_list_is_used_until_it_is_too_old() {
        let (s, d) = store("age");
        assert!(s.cached(900, 1000).unwrap().is_none(), "nothing saved yet");
        s.save_fetch(vec![item(1)], &[], 1000).unwrap();
        let l = s.cached(900, 1000 + 899).unwrap().expect("still fresh");
        assert_eq!(l.items.len(), 1);
        assert_eq!(l.fetched_at, 1000);
        assert!(s.cached(900, 1000 + 900).unwrap().is_none(), "too old");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_new_fetch_replaces_the_saved_list() {
        let (s, d) = store("replace");
        s.save_fetch(vec![item(1), item(2)], &[], 1000).unwrap();
        let l = s.save_fetch(vec![item(2)], &[], 2000).unwrap();
        assert_eq!(ids(&l.items), vec![2], "an item that GitHub closed leaves the list");
        assert_eq!(ids(&s.cached(u64::MAX, 2100).unwrap().unwrap().items), vec![2]);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
