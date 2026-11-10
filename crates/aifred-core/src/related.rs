//! The items linked to one item, for the search results of the chat tools.
//!
//! A search finds items by words and by meaning. This module adds one step
//! along the links that AIfred has already, so the AI also sees the work
//! around each result:
//!
//! - "parent" and "subtask": the subtask tree (`item_link`, links.rs).
//! - "same mission": the other items of the item's mission (missions.rs).
//! - "closes" and "closed by": a GitHub pull request and the issues that it
//!   closes (the `links` of the saved GitHub item, github_list.rs).
//!
//! It makes no AI call and adds no table. See docs/research-graph-memory.md.

use std::collections::HashMap;
use std::path::Path;

use rusqlite::Connection;
use serde::Serialize;

use crate::{db, items};

/// The most linked items for one result. A big tree or mission must not
/// fill the tool answer.
pub const MAX: usize = 8;

/// A linked item before the lookup: its key, why it is linked, and the
/// title and place to use when AIfred has not saved the item.
type Found = (String, String, Option<(String, String)>);

/// One item that is linked to a search result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Related {
    /// The item key, for example "github:acme/shop#12".
    pub key: String,
    pub title: String,
    /// Where the item is from, for example "GitHub acme/shop". Can be "".
    pub place: String,
    /// How it is linked: "parent", "subtask", "same mission: <title>",
    /// "closes", or "closed by".
    pub why: String,
}

/// The linked items of each key in `keys`, in the same order. The linked
/// items are extra data for a search result: an error gives an empty list
/// (and a line in the log), so it cannot fail the search.
pub fn of_keys(dir: &Path, keys: &[String]) -> Vec<Vec<Related>> {
    let found = db::read(dir, |c| {
        let mut l = Linker::new(c);
        Ok(keys.iter().map(|k| l.of(k)).collect())
    });
    found.unwrap_or_else(|e| {
        eprintln!("aifred: cannot read the linked items: {e:#}");
        vec![vec![]; keys.len()]
    })
}

/// Finds the linked items of the results of one search. It reads the
/// links to GitHub items once, at the first GitHub key, and not again for
/// each result.
pub(crate) struct Linker<'c> {
    c: &'c Connection,
    /// For each GitHub item id: the saved items that link to it, and if
    /// each one is a pull request.
    back: Option<HashMap<String, Vec<(String, bool)>>>,
}

impl<'c> Linker<'c> {
    pub(crate) fn new(c: &'c Connection) -> Self {
        Linker { c, back: None }
    }

    /// The linked items of `key` (of_in). An error gives an empty list.
    pub(crate) fn of(&mut self, key: &str) -> Vec<Related> {
        self.of_in(key).unwrap_or_else(|e| {
            eprintln!("aifred: cannot read the linked items of {key}: {e:#}");
            vec![]
        })
    }

    /// The linked items of `key`, at most MAX: the parent first, then the
    /// subtasks, the mission, and the GitHub links. An item that AIfred does
    /// not know (for example a deleted task) or has a bad key is left out.
    fn of_in(&mut self, key: &str) -> anyhow::Result<Vec<Related>> {
        let c = self.c;
        let mut found: Vec<Found> = Vec::new();
        if let Some(p) = crate::links::parent_in(c, key)? {
            found.push((p, "parent".into(), None));
        }
        for (k, _) in crate::links::children_in(c, key)? {
            found.push((k, "subtask".into(), None));
        }
        found.extend(mission_in(c, key)?.into_iter().map(|(k, w)| (k, w, None)));
        found.extend(self.github_in(key)?);

        let mut out: Vec<Related> = Vec::new();
        for (k, why, fallback) in found {
            if out.len() == MAX {
                break;
            }
            // One row for each item: the first reason wins.
            if k == key || out.iter().any(|r| r.key == k) {
                continue;
            }
            let saved = items::title_place(c, &k).unwrap_or_else(|e| {
                eprintln!("aifred: skip the linked item {k}: {e:#}");
                None
            });
            if let Some((title, place)) = saved.or(fallback) {
                out.push(Related {
                    key: k,
                    title,
                    place,
                    why,
                });
            }
        }
        Ok(out)
    }

    /// The GitHub items that `key` links to, and the ones that link to it. A
    /// pull request "closes" an issue; an issue is "closed by" a pull request.
    /// A link has its own title and repository, for a linked item that is not
    /// in the user's list (most issues that a review closes are not).
    fn github_in(&mut self, key: &str) -> anyhow::Result<Vec<Found>> {
        let Some(id) = key.strip_prefix("github:") else {
            return Ok(vec![]);
        };
        let why = |is_pr: bool| if is_pr { "closed by" } else { "closes" }.to_string();
        let mut out = Vec::new();
        // The links in the item itself. `pull_request` is about the linked item.
        let mut own = self.c.prepare_cached(
            "SELECT json_extract(l.value, '$.id'), json_extract(l.value, '$.pull_request'),
                    json_extract(l.value, '$.title'), json_extract(l.value, '$.repo')
             FROM work_item w, json_each(w.data, '$.links') l
             WHERE w.source = 'github' AND w.item_id = ?1 AND json_valid(w.data)",
        )?;
        for row in own.query_map([id], |r| {
            Ok((
                r.get::<_, Option<String>>(0)?,
                r.get::<_, Option<bool>>(1)?,
                r.get::<_, Option<String>>(2)?,
                r.get::<_, Option<String>>(3)?,
            ))
        })? {
            if let (Some(other), is_pr, title, repo) = row? {
                let fallback = title.map(|t| (t, format!("GitHub {}", repo.unwrap_or_default())));
                out.push((
                    format!("github:{other}"),
                    why(is_pr.unwrap_or(false)),
                    fallback,
                ));
            }
        }
        // Other items that link to this one.
        for (other, is_pr) in self.back()?.get(id).into_iter().flatten() {
            out.push((format!("github:{other}"), why(*is_pr), None));
        }
        Ok(out)
    }

    /// All links between saved GitHub items, by the id of the linked item.
    /// One read for the whole search. The url of an item tells if it is a
    /// pull request.
    fn back(&mut self) -> anyhow::Result<&HashMap<String, Vec<(String, bool)>>> {
        if self.back.is_none() {
            let mut q = self.c.prepare(
                "SELECT json_extract(l.value, '$.id'), w.item_id, instr(w.url, '/pull/') > 0
                 FROM work_item w, json_each(w.data, '$.links') l
                 WHERE w.source = 'github' AND json_valid(w.data)",
            )?;
            let mut map: HashMap<String, Vec<(String, bool)>> = HashMap::new();
            for row in q.query_map([], |r| {
                Ok((
                    r.get::<_, Option<String>>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, bool>(2)?,
                ))
            })? {
                if let (Some(to), from, is_pr) = row? {
                    map.entry(to).or_default().push((from, is_pr));
                }
            }
            self.back = Some(map);
        }
        Ok(self.back.as_ref().expect("set above"))
    }
}

/// The other items of `key`'s open mission.
fn mission_in(c: &Connection, key: &str) -> anyhow::Result<Vec<(String, String)>> {
    let mut q = c.prepare_cached(
        "SELECT o.key, m.title FROM mission_item i
         JOIN mission m ON m.id = i.mission AND m.status = 'open'
         JOIN mission_item o ON o.mission = i.mission AND o.key <> i.key
         WHERE i.key = ?1 ORDER BY o.pos",
    )?;
    let rows = q.query_map([key], |r| {
        let (k, title): (String, String) = (r.get(0)?, r.get(1)?);
        Ok((k, format!("same mission: {title}")))
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn put(d: &Path, id: &str, title: &str, url: &str, links: &str) {
        db::write(d, |tx| {
            tx.execute(
                "INSERT INTO work_item (source, item_id, title, place, url, details, data, in_list, pos, first_seen, last_seen)
                 VALUES ('github', ?1, ?2, 'acme/shop', ?3, '', json_object('id', ?1, 'links', json(?4)), 1, 0, 0, 0)",
                [id, title, url, links],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn whys(r: &[Related]) -> Vec<(&str, &str)> {
        r.iter().map(|x| (x.key.as_str(), x.why.as_str())).collect()
    }

    #[test]
    fn a_pull_request_and_its_issue_find_each_other() {
        let d = db::tests::temp("related-github");
        put(
            &d,
            "acme/shop#1",
            "Fix the cart",
            "https://github.com/acme/shop/issues/1",
            "[]",
        );
        put(
            &d,
            "acme/shop#2",
            "Cart fix",
            "https://github.com/acme/shop/pull/2",
            r#"[{"id":"acme/shop#1","pull_request":false}]"#,
        );
        let r = of_keys(
            &d,
            &["github:acme/shop#2".into(), "github:acme/shop#1".into()],
        );
        assert_eq!(whys(&r[0]), vec![("github:acme/shop#1", "closes")]);
        assert_eq!(whys(&r[1]), vec![("github:acme/shop#2", "closed by")]);
        assert_eq!(r[0][0].title, "Fix the cart");
    }

    #[test]
    fn a_linked_issue_that_is_not_saved_uses_the_title_in_the_link() {
        let d = db::tests::temp("related-unsaved");
        put(
            &d,
            "acme/shop#2",
            "Cart fix",
            "https://github.com/acme/shop/pull/2",
            r#"[{"id":"acme/shop#1","repo":"acme/shop","title":"Fix the cart","pull_request":false}]"#,
        );
        let r = of_keys(&d, &["github:acme/shop#2".into()]);
        assert_eq!(whys(&r[0]), vec![("github:acme/shop#1", "closes")]);
        assert_eq!(
            (r[0][0].title.as_str(), r[0][0].place.as_str()),
            ("Fix the cart", "GitHub acme/shop")
        );
    }

    #[test]
    fn a_link_in_both_items_shows_once() {
        let d = db::tests::temp("related-twice");
        put(
            &d,
            "acme/shop#1",
            "Fix the cart",
            "https://github.com/acme/shop/issues/1",
            r#"[{"id":"acme/shop#2","pull_request":true}]"#,
        );
        put(
            &d,
            "acme/shop#2",
            "Cart fix",
            "https://github.com/acme/shop/pull/2",
            r#"[{"id":"acme/shop#1","pull_request":false}]"#,
        );
        let r = of_keys(&d, &["github:acme/shop#1".into()]);
        assert_eq!(whys(&r[0]), vec![("github:acme/shop#2", "closed by")]);
    }

    #[test]
    fn parent_subtask_and_mission_are_found_and_unknown_items_are_left_out() {
        let d = db::tests::temp("related-tree");
        put(
            &d,
            "acme/shop#1",
            "Top",
            "https://github.com/acme/shop/issues/1",
            "[]",
        );
        put(
            &d,
            "acme/shop#2",
            "Middle",
            "https://github.com/acme/shop/issues/2",
            "[]",
        );
        put(
            &d,
            "acme/shop#3",
            "Under",
            "https://github.com/acme/shop/issues/3",
            "[]",
        );
        put(
            &d,
            "acme/shop#4",
            "Mate",
            "https://github.com/acme/shop/issues/4",
            "[]",
        );
        db::write(&d, |tx| {
            tx.execute_batch(
                "INSERT INTO item_link (key, main, ts) VALUES ('github:acme/shop#2', 'github:acme/shop#1', 0);
                 INSERT INTO item_link (key, main, ts) VALUES ('github:acme/shop#3', 'github:acme/shop#2', 0);
                 INSERT INTO mission (id, title, created, updated) VALUES (1, 'Ship the cart', 0, 0);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:acme/shop#2', 1, 0);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:acme/shop#4', 1, 1);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('task:gone', 1, 2);",
            )?;
            Ok(())
        })
        .unwrap();
        let r = of_keys(&d, &["github:acme/shop#2".into()]);
        assert_eq!(
            whys(&r[0]),
            vec![
                ("github:acme/shop#1", "parent"),
                ("github:acme/shop#3", "subtask"),
                ("github:acme/shop#4", "same mission: Ship the cart"),
            ]
        );
    }

    #[test]
    fn a_bad_key_is_left_out_and_the_rest_stays() {
        let d = db::tests::temp("related-bad-key");
        put(
            &d,
            "acme/shop#1",
            "A",
            "https://github.com/acme/shop/issues/1",
            "[]",
        );
        put(
            &d,
            "acme/shop#2",
            "B",
            "https://github.com/acme/shop/issues/2",
            "[]",
        );
        db::write(&d, |tx| {
            tx.execute_batch(
                "INSERT INTO mission (id, title, created, updated) VALUES (1, 'M', 0, 0);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:acme/shop#1', 1, 0);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('jira:5', 1, 1);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:', 1, 2);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:acme/shop#2', 1, 3);",
            )?;
            Ok(())
        })
        .unwrap();
        let r = of_keys(&d, &["github:acme/shop#1".into()]);
        assert_eq!(whys(&r[0]), vec![("github:acme/shop#2", "same mission: M")]);
    }

    #[test]
    fn a_done_mission_links_nothing() {
        let d = db::tests::temp("related-done");
        put(
            &d,
            "acme/shop#1",
            "A",
            "https://github.com/acme/shop/issues/1",
            "[]",
        );
        put(
            &d,
            "acme/shop#2",
            "B",
            "https://github.com/acme/shop/issues/2",
            "[]",
        );
        db::write(&d, |tx| {
            tx.execute_batch(
                "INSERT INTO mission (id, title, status, created, updated) VALUES (1, 'Old', 'done', 0, 0);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:acme/shop#1', 1, 0);
                 INSERT INTO mission_item (key, mission, pos) VALUES ('github:acme/shop#2', 1, 1);",
            )?;
            Ok(())
        })
        .unwrap();
        assert!(of_keys(&d, &["github:acme/shop#1".into()])[0].is_empty());
    }
}
