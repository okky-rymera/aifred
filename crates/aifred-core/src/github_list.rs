//! The GitHub part of the to-do list. The sync goes one way only: AIfred reads
//! GitHub and never changes it. The list is saved in the data folder as
//! github-cache.json, so AIfred does not ask GitHub each time the window loads.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::files;
use crate::github::{GithubItem, Link};

#[derive(Debug, Default, Serialize, Deserialize)]
struct Cache {
    /// Unix seconds. 0 means never.
    fetched_at: u64,
    items: Vec<GithubItem>,
}

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

    fn cache_path(&self) -> PathBuf {
        self.dir.join("github-cache.json")
    }

    /// The saved list, if it is younger than `max_age_secs`.
    /// None also when there is no saved list yet.
    /// Use u64::MAX to get the saved list at any age.
    pub fn cached(&self, max_age_secs: u64, now: u64) -> anyhow::Result<Option<GithubList>> {
        let cache: Cache = files::load(&self.cache_path())?;
        if cache.fetched_at == 0 || now.saturating_sub(cache.fetched_at) >= max_age_secs {
            return Ok(None);
        }
        Ok(Some(view(cache)))
    }

    /// Saves a new list from GitHub.
    ///
    /// GitHub did not send the links of the items in `no_links`. They get the
    /// links from the saved list, so their rows do not change shape. The read
    /// and the save hold the file lock, so a second fetch at the same time
    /// (the timer and the reload button) cannot mix old and new links.
    ///
    /// A saved list that cannot be read counts as no saved links: the new list
    /// from GitHub is more important. The save then repairs the file.
    pub fn save_fetch(&self, mut items: Vec<GithubItem>, no_links: &[String], now: u64) -> anyhow::Result<GithubList> {
        let path = self.cache_path();
        let cache = files::locked(&path, || {
            if !no_links.is_empty() {
                match files::load::<Cache>(&path) {
                    Ok(saved) => keep_saved_links(&mut items, no_links, &saved),
                    Err(e) => eprintln!("aifred: cannot use the saved GitHub links: {e:#}"),
                }
            }
            let cache = Cache { fetched_at: now, items };
            files::save(&path, &cache)?;
            Ok(cache)
        })?;
        Ok(view(cache))
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

fn keep_saved_links(items: &mut [GithubItem], ids: &[String], saved: &Cache) {
    for item in items.iter_mut().filter(|i| ids.contains(&i.id)) {
        if let Some(s) = saved.items.iter().find(|s| s.id == item.id) {
            item.links = s.links.clone();
            item.more_links = s.more_links;
        }
    }
}

fn view(cache: Cache) -> GithubList {
    GithubList { items: join_linked(cache.items), fetched_at: cache.fetched_at, warning: None }
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
    fn a_broken_saved_list_does_not_stop_a_new_fetch() {
        let (s, d) = store("broken");
        std::fs::write(d.join("github-cache.json"), b"{ half a fi").unwrap();
        let l = s.save_fetch(vec![item(1)], &["acme/shop#1".into()], 1000).unwrap();
        assert_eq!(ids(&l.items), vec![1]);
        assert_eq!(ids(&s.cached(u64::MAX, 1000).unwrap().unwrap().items), vec![1], "the file is repaired");
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
