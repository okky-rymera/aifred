//! One piece of work in many places. A ClickUp task, its GitHub issue, and
//! an email about it are one piece of work. The user links them, and the
//! home list shows them as one row (plan step 6 in docs/plan-assistant.md).
//!
//! - A group has one main item. The other items of the group have a row in
//!   `item_link` that names the main item. A main item is never itself
//!   under another item, so a group is never more than one level deep.
//! - AIfred suggests links. The user accepts or rejects each suggestion.
//!   A rejected pair is never suggested again.
//! - A suggestion needs no AI call. It comes from two rules:
//!   1. One item names the other: a GitHub link or "acme/shop#7", a ClickUp
//!      link, a ClickUp id like "DEV-42", or "CU-86a1b2" (the ClickUp GitHub
//!      integration puts this in branch names and titles).
//!   2. The titles have mostly the same words.
//! - Only items from different sources get suggestions. GitHub already
//!   joins an issue and its pull requests (github_list.rs).
//!
//! GitHub links between an issue and its pull request stay in github_list.rs.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;
use crate::items::split;

/// At most this many items in one group.
pub const MAX_GROUP: usize = 10;
/// At most this many suggestions.
const MAX_SUGGESTIONS: usize = 10;
/// Two titles match when this part of their words is the same, or more.
const TITLE_MATCH: f64 = 0.6;
/// ... and when they have at least this many words in common.
const TITLE_MIN_COMMON: usize = 2;

/// One item that shows under another item.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Link {
    /// The item key of the item under the main item.
    pub key: String,
    /// The item key of the main item.
    pub main: String,
}

/// Two open items that AIfred thinks are one piece of work. Each one can
/// be in a group already: a link then joins the two groups.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Suggestion {
    /// The item that AIfred suggests as the main item.
    pub main: String,
    /// The item that goes under it.
    pub key: String,
    /// Why, for example "The GitHub item names DEV-42".
    pub why: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Links {
    pub links: Vec<Link>,
    pub suggestions: Vec<Suggestion>,
}

/// The links and the suggestions.
pub fn get(dir: &Path) -> anyhow::Result<Links> {
    db::read(dir, get_in)
}

pub fn get_in(c: &Connection) -> anyhow::Result<Links> {
    let links = all(c)?;
    let suggestions = suggest(c, &links)?;
    Ok(Links { links, suggestions })
}

fn all(c: &Connection) -> anyhow::Result<Vec<Link>> {
    let mut q = c.prepare("SELECT key, main FROM item_link ORDER BY main, key")?;
    let rows = q.query_map([], |r| {
        Ok(Link {
            key: r.get(0)?,
            main: r.get(1)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The main item of `key`'s group. An item with no link is its own main item.
fn head_in(c: &Connection, key: &str) -> anyhow::Result<String> {
    Ok(
        c.query_row("SELECT main FROM item_link WHERE key = ?1", [key], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or_else(|| key.to_string()),
    )
}

/// All items of `key`'s group, the main item first. An item with no link
/// is a group of one.
pub fn group(dir: &Path, key: &str) -> anyhow::Result<Vec<String>> {
    db::read(dir, |c| group_in(c, key))
}

/// group(), with a connection that is open.
pub(crate) fn group_in(c: &Connection, key: &str) -> anyhow::Result<Vec<String>> {
    members_in(c, &head_in(c, key)?)
}

/// All items of the group whose main item is `head`, the main item first.
fn members_in(c: &Connection, head: &str) -> anyhow::Result<Vec<String>> {
    let mut q = c.prepare("SELECT key FROM item_link WHERE main = ?1 ORDER BY key")?;
    let mut out = vec![head.to_string()];
    for k in q.query_map([head], |r| r.get::<_, String>(0))? {
        out.push(k?);
    }
    Ok(out)
}

/// Puts `key` and its group in the group of `main`. The main item of
/// `main`'s group stays the main item. When `key` already is the main item
/// of `main`'s group, `main` becomes the main item (this is how an undo of
/// an unlink puts a main item back).
pub fn link(dir: &Path, main: &str, key: &str, now: u64) -> anyhow::Result<Links> {
    // The suggestions are found after the change, not in it: the search
    // must not keep other saves waiting.
    db::write(dir, |tx| link_in(tx, main, key, now))?;
    get(dir)
}

pub(crate) fn link_in(tx: &Transaction, main: &str, key: &str, now: u64) -> anyhow::Result<()> {
    split(main)?;
    split(key)?;
    if main == key {
        bail!("an item cannot be linked to itself");
    }
    for k in [main, key] {
        if crate::items::title_place(tx, k)?.is_none() {
            bail!("AIfred does not know the item {k}");
        }
    }
    // A link to an item that is gone must not count toward MAX_GROUP.
    drop_gone_in(tx, now)?;
    let (hm, hk) = (head_in(tx, main)?, head_in(tx, key)?);
    let target = if hm == hk {
        main.to_string()
    } else {
        hm.clone()
    };
    let mut members: BTreeSet<String> = members_in(tx, &hm)?.into_iter().collect();
    members.extend(members_in(tx, &hk)?);
    if members.len() > MAX_GROUP {
        bail!("one piece of work can have at most {MAX_GROUP} items");
    }
    for m in &members {
        tx.execute("DELETE FROM item_link WHERE key = ?1", [m])?;
    }
    for m in members.iter().filter(|m| **m != target) {
        tx.execute(
            "INSERT INTO item_link (key, main, ts) VALUES (?1, ?2, ?3)",
            rusqlite::params![m, target, now as i64],
        )?;
    }
    Ok(())
}

/// Takes `key` out of its group. When `key` is the main item, the next
/// item of the group becomes the main item of the others.
pub fn unlink(dir: &Path, key: &str, now: u64) -> anyhow::Result<Links> {
    split(key)?;
    db::write(dir, |tx| unlink_in(tx, key, now))?;
    get(dir)
}

/// unlink(), in a change that is open. A deleted task uses it too (tasks.rs),
/// so no group keeps an item that is gone.
pub fn unlink_in(tx: &Transaction, key: &str, now: u64) -> anyhow::Result<()> {
    let head = head_in(tx, key)?;
    if head != key {
        tx.execute("DELETE FROM item_link WHERE key = ?1", [key])?;
    } else {
        let rest: Vec<String> = members_in(tx, key)?.into_iter().skip(1).collect();
        tx.execute("DELETE FROM item_link WHERE main = ?1", [key])?;
        if let Some((new, others)) = rest.split_first() {
            for o in others {
                tx.execute(
                    "INSERT INTO item_link (key, main, ts) VALUES (?1, ?2, ?3)",
                    rusqlite::params![o, new, now as i64],
                )?;
            }
        }
    }
    Ok(())
}

/// Takes each item that AIfred does not know now out of its group. Use it
/// after a delete of work items, for example when the user disconnects
/// ClickUp or Gmail (clickup_list.rs, gmail.rs).
pub fn drop_gone_in(tx: &Transaction, now: u64) -> anyhow::Result<()> {
    let mut keys = BTreeSet::new();
    for l in all(tx)? {
        keys.insert(l.key);
        keys.insert(l.main);
    }
    for k in keys {
        if crate::items::title_place(tx, &k)?.is_none() {
            unlink_in(tx, &k, now)?;
        }
    }
    Ok(())
}

/// Says that `a` and `b` are not one piece of work (`no` true), so AIfred
/// does not suggest them again. It also does not suggest other items of
/// their two groups, while the groups stay as they are. `no` false takes that back (Undo).
pub fn reject(dir: &Path, a: &str, b: &str, no: bool, now: u64) -> anyhow::Result<Links> {
    split(a)?;
    split(b)?;
    let (a, b) = if a < b { (a, b) } else { (b, a) };
    db::write(dir, |tx| {
        if no {
            tx.execute(
                "INSERT INTO link_no (a, b, ts) VALUES (?1, ?2, ?3) ON CONFLICT DO NOTHING",
                rusqlite::params![a, b, now as i64],
            )?;
        } else {
            tx.execute("DELETE FROM link_no WHERE a = ?1 AND b = ?2", [a, b])?;
        }
        Ok(())
    })?;
    get(dir)
}

/// The groups with two or more open items, for the AI (overview.rs). The
/// main item is first. When the main item is closed, the first open item
/// takes its place, as in the home list.
pub fn for_ai(c: &Connection) -> anyhow::Result<Vec<Vec<String>>> {
    let mut groups: Vec<(String, Vec<String>)> = Vec::new();
    for l in all(c)? {
        match groups.iter_mut().find(|(m, _)| *m == l.main) {
            Some((_, g)) => g.push(l.key),
            None => groups.push((l.main.clone(), vec![l.main, l.key])),
        }
    }
    let mut out = Vec::new();
    for (_, g) in groups {
        let mut open = Vec::new();
        for k in g {
            if crate::items::is_open(c, &k)? {
                open.push(k);
            }
        }
        if open.len() > 1 {
            out.push(open);
        }
    }
    Ok(out)
}

// ---------- Suggestions ----------

/// What the suggestion rules need of one open item.
struct Open {
    key: String,
    source: String,
    title: String,
    /// The title, the description, and nothing else: the text that can name other items.
    text: String,
    /// The names of this item in lower case: "acme/shop#7", a ClickUp id, a ClickUp custom id.
    names: Vec<String>,
    /// How other items name it in a reason, for example "DEV-42".
    label: String,
}

fn open_items(c: &Connection) -> anyhow::Result<Vec<Open>> {
    let mut out = Vec::new();
    let mut q = c.prepare("SELECT id, title, notes FROM task WHERE done = 0")?;
    for r in q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
        ))
    })? {
        let (id, title, notes) = r?;
        out.push(Open {
            key: format!("task:{id}"),
            source: "task".into(),
            text: format!("{title}\n{notes}"),
            title,
            names: Vec::new(),
            label: String::new(),
        });
    }
    let mut q = c.prepare(
        "SELECT source, item_id, title, details, data FROM work_item
         WHERE in_list = 1 AND source IN ('github', 'clickup', 'gmail')",
    )?;
    for r in q.query_map([], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, String>(2)?,
            r.get::<_, String>(3)?,
            r.get::<_, String>(4)?,
        ))
    })? {
        let (source, id, title, details, data) = r?;
        let mut names = Vec::new();
        let mut label = String::new();
        match source.as_str() {
            "github" => {
                names.push(id.to_lowercase());
                label = id.clone();
            }
            "clickup" => {
                names.push(id.to_lowercase());
                label = id.clone();
                let custom = serde_json::from_str::<serde_json::Value>(&data)
                    .ok()
                    .and_then(|v| v["custom_id"].as_str().map(String::from))
                    .filter(|s| !s.is_empty());
                if let Some(cid) = custom {
                    names.push(cid.to_lowercase());
                    label = cid;
                }
            }
            _ => {}
        }
        out.push(Open {
            key: format!("{source}:{id}"),
            text: format!("{title}\n{details}"),
            source,
            title,
            names,
            label,
        });
    }
    Ok(out)
}

/// The order of the sources for the main item: the source with the most
/// plan data first (a due day, a priority).
fn rank(source: &str) -> u8 {
    match source {
        "task" => 0,
        "clickup" => 1,
        "github" => 2,
        _ => 3,
    }
}

fn source_name(source: &str) -> &'static str {
    match source {
        "task" => "Your task",
        "clickup" => "The ClickUp task",
        "github" => "The GitHub item",
        _ => "The email",
    }
}

fn suggest(c: &Connection, links: &[Link]) -> anyhow::Result<Vec<Suggestion>> {
    let items = open_items(c)?;
    let head: HashMap<&str, &str> = links
        .iter()
        .map(|l| (l.key.as_str(), l.main.as_str()))
        .collect();
    let head_of = |k: &str| -> String { head.get(k).copied().unwrap_or(k).to_string() };
    // The number of items in each group, by its main item.
    let mut size: HashMap<&str, usize> = HashMap::new();
    for l in links {
        *size.entry(l.main.as_str()).or_insert(1) += 1;
    }
    let size_of = |h: &str| size.get(h).copied().unwrap_or(1);
    let ordered = |a: String, b: String| if a < b { (a, b) } else { (b, a) };
    // A rejected pair blocks the two groups of its items, not only the two
    // items. Else another pair of the same two groups comes back at once.
    let mut no: HashSet<(String, String)> = HashSet::new();
    let mut q = c.prepare("SELECT a, b FROM link_no")?;
    for r in q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))? {
        let (a, b) = r?;
        no.insert(ordered(head_of(&a), head_of(&b)));
    }
    let by_name: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .flat_map(|(n, i)| i.names.iter().map(move |s| (s.as_str(), n)))
        .collect();

    // The best reason for each pair of groups: (score, a, b, why).
    let mut best: HashMap<(String, String), (f64, usize, usize, String)> = HashMap::new();
    let mut offer = |a: usize, b: usize, score: f64, why: String| {
        let (ia, ib) = (&items[a], &items[b]);
        if ia.source == ib.source {
            return;
        }
        let (ha, hb) = (head_of(&ia.key), head_of(&ib.key));
        if ha == hb {
            return;
        }
        // A link that would make a group too large fails, so do not suggest it.
        if size_of(&ha) + size_of(&hb) > MAX_GROUP {
            return;
        }
        let pair = ordered(ha, hb);
        if no.contains(&pair) {
            return;
        }
        let e = best.entry(pair).or_insert((f64::MIN, a, b, String::new()));
        if score > e.0 {
            *e = (score, a, b, why);
        }
    };

    // Rule 1: one item names the other.
    for (a, ia) in items.iter().enumerate() {
        for name in names_in(&ia.text) {
            if let Some(&b) = by_name.get(name.as_str())
                && b != a
            {
                let why = format!("{} names {}", source_name(&ia.source), items[b].label);
                offer(a, b, 2.0, why);
            }
        }
    }

    // Rule 2: the titles have mostly the same words. An index from each word
    // to its items finds the pairs with words in common, so the search does
    // not compare each item with all other items.
    let words: Vec<BTreeSet<String>> = items.iter().map(|i| title_words(&i.title)).collect();
    let mut with_word: HashMap<&str, Vec<usize>> = HashMap::new();
    for (n, w) in words.iter().enumerate() {
        for x in w {
            with_word.entry(x.as_str()).or_default().push(n);
        }
    }
    for a in 0..items.len() {
        // The number of words in common with each later item of another source.
        let mut count: HashMap<usize, usize> = HashMap::new();
        for x in &words[a] {
            for &b in &with_word[x.as_str()] {
                if b > a && items[b].source != items[a].source {
                    *count.entry(b).or_default() += 1;
                }
            }
        }
        let mut found: Vec<(usize, usize)> = count
            .into_iter()
            .filter(|&(_, n)| n >= TITLE_MIN_COMMON)
            .collect();
        found.sort_unstable();
        for (b, n) in found {
            let all = words[a].len() + words[b].len() - n;
            let score = n as f64 / all as f64;
            if score >= TITLE_MATCH {
                let list: Vec<&str> = words[a]
                    .intersection(&words[b])
                    .take(4)
                    .map(|s| s.as_str())
                    .collect();
                offer(
                    a,
                    b,
                    score,
                    format!("Same words in the titles: {}", list.join(", ")),
                );
            }
        }
    }

    let mut found: Vec<(f64, Suggestion)> = best
        .into_values()
        .map(|(score, a, b, why)| {
            // The two items of the reason, so the card shows what the reason is
            // about. Both are open. A link of the two joins their groups, under
            // the main item of the group of `main` (link_in). So `main` is the
            // item whose main item comes first in the order of rank().
            let (ia, ib) = (&items[a], &items[b]);
            let src = |k: &str| split(k).map(|(s, _)| rank(s)).unwrap_or(u8::MAX);
            let (main, key) = if src(&head_of(&ib.key)) < src(&head_of(&ia.key)) {
                (ib, ia)
            } else {
                (ia, ib)
            };
            (
                score,
                Suggestion {
                    main: main.key.clone(),
                    key: key.key.clone(),
                    why,
                },
            )
        })
        .collect();
    found.sort_by(|(x, a), (y, b)| {
        y.total_cmp(x)
            .then_with(|| a.main.cmp(&b.main))
            .then_with(|| a.key.cmp(&b.key))
    });
    Ok(found
        .into_iter()
        .take(MAX_SUGGESTIONS)
        .map(|(_, s)| s)
        .collect())
}

/// Words that say little about which work an item is.
const COMMON_WORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "into", "onto", "that", "this", "when", "not", "are",
    "was", "has", "have", "can", "cannot", "should", "add", "fix", "fixes", "fixed", "update",
    "updates", "make", "use", "new", "issue", "task", "bug", "feature", "feat", "chore", "pull",
    "request", "please", "re", "fwd", "about", "after", "before", "out", "all", "some", "our",
    "your", "you",
];

/// The words of a title in lower case, 3 letters or more, with no common words.
fn title_words(title: &str) -> BTreeSet<String> {
    title
        .split(|ch: char| !ch.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !COMMON_WORDS.contains(&w.as_str()))
        // A number alone ("2026", "142") is not a word of the work.
        .filter(|w| !w.chars().all(|ch| ch.is_ascii_digit()))
        .collect()
}

/// The names of other items in a text, in lower case. See the module text.
fn names_in(text: &str) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let lower = text.to_lowercase();
    // GitHub links: github.com/<owner>/<repo>/(issues|pull)/<n>.
    for (at, _) in lower.match_indices("github.com/") {
        let rest = &lower[at + "github.com/".len()..];
        let parts: Vec<&str> = rest
            .split(|ch: char| ch.is_whitespace() || ch == ')' || ch == '>' || ch == '"')
            .next()
            .unwrap_or("")
            .split('/')
            .collect();
        if parts.len() >= 4 && matches!(parts[2], "issues" | "pull") {
            let n: String = parts[3].chars().take_while(char::is_ascii_digit).collect();
            if !n.is_empty() && !parts[0].is_empty() && !parts[1].is_empty() {
                out.insert(format!("{}/{}#{n}", parts[0], parts[1]));
            }
        }
    }
    // "owner/repo#7".
    let repo_char = |ch: char| ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '/');
    for (at, _) in lower.match_indices('#') {
        let n: String = lower[at + 1..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        if n.is_empty() {
            continue;
        }
        let before = &lower[..at];
        let start = before
            .char_indices()
            .rev()
            .take_while(|(_, ch)| repo_char(*ch))
            .last()
            .map(|(i, _)| i)
            .unwrap_or(at);
        let repo = &before[start..];
        let mut p = repo.split('/');
        if let (Some(o), Some(r), None) = (p.next(), p.next(), p.next())
            && !o.is_empty()
            && !r.is_empty()
        {
            out.insert(format!("{repo}#{n}"));
        }
    }
    // ClickUp links: app.clickup.com/t/<id> or app.clickup.com/t/<team>/<custom id>.
    for (at, _) in lower.match_indices("clickup.com/t/") {
        let rest = &lower[at + "clickup.com/t/".len()..];
        let path: Vec<&str> = rest
            .split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-' || ch == '_' || ch == '/'))
            .next()
            .unwrap_or("")
            .split('/')
            .filter(|s| !s.is_empty())
            .collect();
        if let Some(last) = path.last() {
            out.insert((*last).to_string());
        }
    }
    // Words: "DEV-42" (a ClickUp custom id) and "CU-86a1b2" (a ClickUp id).
    for w in lower.split(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '-')) {
        let w = w.trim_matches('-');
        if let Some(id) = w.strip_prefix("cu-")
            && !id.is_empty()
            && id.chars().all(|ch| ch.is_ascii_alphanumeric())
        {
            out.insert(id.to_string());
        }
        if let Some((p, n)) = w.split_once('-')
            && p.chars().next().is_some_and(|ch| ch.is_ascii_alphabetic())
            && p.chars().all(|ch| ch.is_ascii_alphanumeric())
            && !n.is_empty()
            && n.chars().all(|ch| ch.is_ascii_digit())
        {
            out.insert(w.to_string());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tasks::TaskStore;

    fn dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-links-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn put(d: &Path, source: &str, id: &str, title: &str, details: &str, data: &str) {
        db::write(d, |tx| {
            tx.execute(
                "INSERT INTO work_item (source, item_id, title, place, url, details, data, in_list, pos, first_seen, last_seen)
                 VALUES (?1, ?2, ?3, '', '', ?4, ?5, 1, 0, 0, 0)",
                [source, id, title, details, data],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn finds_the_names_of_other_items_in_a_text() {
        let n = names_in(
            "See https://github.com/Acme/Shop/pull/12, also acme/docs#7 and #9.\n\
             Task: https://app.clickup.com/t/86a1b2 and https://app.clickup.com/t/9012/DEV-42.\n\
             Branch CU-86xyz_fix. Ticket OPS-7. Not: covid-19x, -5, x-.",
        );
        let want: BTreeSet<String> = ["acme/shop#12", "acme/docs#7", "86a1b2", "dev-42", "ops-7"]
            .into_iter()
            .map(String::from)
            .collect();
        assert!(want.is_subset(&n), "{n:?}");
        assert!(!n.contains("9"), "a number alone names nothing: {n:?}");
        assert!(!n.iter().any(|s| s.contains("covid")), "{n:?}");
    }

    #[test]
    fn title_words_skip_short_and_common_words() {
        let w = title_words("Fix the PayPal refund for 2026 orders");
        let want: BTreeSet<String> = ["paypal", "refund", "orders"]
            .into_iter()
            .map(String::from)
            .collect();
        assert_eq!(w, want);
    }

    #[test]
    fn suggests_items_that_name_each_other_or_have_the_same_title() {
        let d = dir("suggest");
        put(
            &d,
            "clickup",
            "86a",
            "Checkout totals",
            "",
            r#"{"custom_id":"DEV-42"}"#,
        );
        put(
            &d,
            "github",
            "acme/shop#12",
            "DEV-42: round the tax",
            "",
            "{}",
        );
        put(&d, "github", "acme/shop#13", "Other work", "", "{}");
        put(&d, "gmail", "m1", "Re: PayPal refund for Ana", "", "{}");
        let s = TaskStore::new(&d);
        let t = s.add("PayPal refund Ana", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        let l = get(&d).unwrap();
        assert!(l.links.is_empty());
        assert_eq!(l.suggestions.len(), 2, "{:?}", l.suggestions);
        // A name is a stronger reason than the same words, so it comes first.
        assert_eq!(l.suggestions[0].main, "clickup:86a");
        assert_eq!(l.suggestions[0].key, "github:acme/shop#12");
        assert_eq!(l.suggestions[0].why, "The GitHub item names DEV-42");
        // A local task is the main item before an email.
        assert_eq!(l.suggestions[1].main, tk);
        assert_eq!(l.suggestions[1].key, "gmail:m1");
        assert!(
            l.suggestions[1].why.contains("paypal"),
            "{:?}",
            l.suggestions[1]
        );

        // Accept the first one, reject the second one.
        let l = link(&d, "clickup:86a", "github:acme/shop#12", 1).unwrap();
        assert_eq!(
            l.links,
            vec![Link {
                key: "github:acme/shop#12".into(),
                main: "clickup:86a".into()
            }]
        );
        let l = reject(&d, "gmail:m1", &tk, true, 2).unwrap();
        assert!(l.suggestions.is_empty(), "{:?}", l.suggestions);
        let l = reject(&d, &tk, "gmail:m1", false, 3).unwrap();
        assert_eq!(l.suggestions.len(), 1, "the undo brings it back");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_suggestion_names_the_items_of_its_reason_and_fits_in_one_group() {
        let d = dir("members");
        put(
            &d,
            "clickup",
            "c1",
            "Checkout totals",
            "",
            r#"{"custom_id":"DEV-42"}"#,
        );
        put(&d, "gmail", "m1", "Hello", "", "{}");
        put(&d, "github", "a/b#1", "DEV-42: round the tax", "", "{}");
        let s = TaskStore::new(&d);
        let t = s.add("Refund Ana", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        link(&d, "gmail:m1", "clickup:c1", 1).unwrap();
        link(&d, &tk, "github:a/b#1", 1).unwrap();
        let l = get(&d).unwrap();
        assert_eq!(l.suggestions.len(), 1, "{:?}", l.suggestions);
        // The items of the reason, not the main items of their groups. The
        // GitHub item is `main`: its main item is a task, and the main item
        // of the ClickUp task is an email. A task comes before an email.
        assert_eq!(l.suggestions[0].main, "github:a/b#1");
        assert_eq!(l.suggestions[0].key, "clickup:c1");
        // The link puts the email's group under the task, as the card says.
        let l = link(&d, "github:a/b#1", "clickup:c1", 2).unwrap();
        assert!(l.links.iter().all(|x| x.main == tk), "{:?}", l.links);
        // Back to the two groups.
        for k in ["clickup:c1", "gmail:m1", "github:a/b#1"] {
            unlink(&d, k, 2).unwrap();
        }
        link(&d, "gmail:m1", "clickup:c1", 2).unwrap();
        link(&d, &tk, "github:a/b#1", 2).unwrap();
        // Two groups that are too large together get no suggestion.
        for n in 0..8 {
            let x = s.add(&format!("Other {n}"), "", "you").unwrap();
            link(&d, &tk, &format!("task:{}", x.id), 2).unwrap();
        }
        assert!(get(&d).unwrap().suggestions.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn not_the_same_blocks_all_pairs_of_the_two_groups() {
        let d = dir("reject-groups");
        put(
            &d,
            "clickup",
            "c1",
            "Checkout totals",
            "",
            r#"{"custom_id":"DEV-42"}"#,
        );
        put(&d, "github", "x/y#1", "Checkout tax rounding", "", "{}");
        // The main item is not an item of the reasons below.
        link(&d, "github:x/y#1", "clickup:c1", 1).unwrap();
        let s = TaskStore::new(&d);
        // The task names the ClickUp task, and has the title words of the GitHub item.
        s.add("Checkout tax rounding", "See DEV-42", "you").unwrap();
        let l = get(&d).unwrap();
        assert_eq!(l.suggestions.len(), 1, "{:?}", l.suggestions);
        let sg = &l.suggestions[0];
        let l = reject(&d, &sg.main, &sg.key, true, 2).unwrap();
        assert!(l.suggestions.is_empty(), "{:?}", l.suggestions);
        // The undo brings the pair of groups back.
        let l = reject(&d, &sg.main, &sg.key, false, 3).unwrap();
        assert_eq!(l.suggestions.len(), 1, "{:?}", l.suggestions);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn links_to_deleted_work_items_go_away() {
        let d = dir("gone");
        put(&d, "clickup", "c1", "T", "", "{}");
        put(&d, "github", "a/b#1", "T", "", "{}");
        put(&d, "gmail", "m1", "T", "", "{}");
        link(&d, "clickup:c1", "github:a/b#1", 1).unwrap();
        link(&d, "clickup:c1", "gmail:m1", 1).unwrap();
        // The user disconnects ClickUp: its items are deleted.
        db::write(&d, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = 'clickup'", [])?;
            drop_gone_in(tx, 2)
        })
        .unwrap();
        assert_eq!(
            get(&d).unwrap().links,
            vec![Link {
                key: "gmail:m1".into(),
                main: "github:a/b#1".into()
            }]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_link_does_not_count_items_that_are_gone() {
        let d = dir("gone-max");
        put(&d, "github", "a/b#0", "T", "", "{}");
        for n in 1..MAX_GROUP {
            put(&d, "clickup", &format!("c{n}"), "T", "", "{}");
            link(&d, "github:a/b#0", &format!("clickup:c{n}"), 1).unwrap();
        }
        // An old database: ClickUp items were deleted before drop_gone_in existed.
        db::write(&d, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = 'clickup'", [])?;
            Ok(())
        })
        .unwrap();
        put(&d, "gmail", "m1", "T", "", "{}");
        put(&d, "gmail", "m2", "T", "", "{}");
        link(&d, "github:a/b#0", "gmail:m1", 2).unwrap();
        let l = link(&d, "github:a/b#0", "gmail:m2", 2).unwrap();
        assert_eq!(l.links.len(), 2, "{:?}", l.links);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_deleted_task_leaves_its_group() {
        let d = dir("deleted");
        put(&d, "clickup", "c1", "T", "", "{}");
        put(&d, "github", "a/b#1", "T", "", "{}");
        let s = TaskStore::new(&d);
        let t = s.add("T", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        link(&d, &tk, "clickup:c1", 1).unwrap();
        link(&d, &tk, "github:a/b#1", 1).unwrap();
        s.remove(&t.id).unwrap();
        assert_eq!(
            get(&d).unwrap().links,
            vec![Link {
                key: "github:a/b#1".into(),
                main: "clickup:c1".into()
            }]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn items_of_one_source_get_no_suggestion() {
        let d = dir("same-source");
        put(
            &d,
            "github",
            "acme/shop#1",
            "PayPal refund broken",
            "acme/shop#2",
            "{}",
        );
        put(
            &d,
            "github",
            "acme/shop#2",
            "PayPal refund broken",
            "",
            "{}",
        );
        assert!(get(&d).unwrap().suggestions.is_empty());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn groups_merge_and_split() {
        let d = dir("groups");
        for (s, id) in [
            ("clickup", "c1"),
            ("github", "a/b#1"),
            ("github", "a/b#2"),
            ("gmail", "m1"),
        ] {
            put(&d, s, id, "T", "", "{}");
        }
        link(&d, "clickup:c1", "github:a/b#1", 1).unwrap();
        link(&d, "gmail:m1", "github:a/b#2", 1).unwrap();
        // Linking an item under a group puts the whole group under the main item.
        let l = link(&d, "github:a/b#1", "gmail:m1", 2).unwrap();
        assert!(
            l.links.iter().all(|x| x.main == "clickup:c1"),
            "{:?}",
            l.links
        );
        assert_eq!(l.links.len(), 3);
        // Unlink the main item: the next item is the new main item.
        let l = unlink(&d, "clickup:c1", 3).unwrap();
        assert!(
            l.links.iter().all(|x| x.main == "github:a/b#1"),
            "{:?}",
            l.links
        );
        assert_eq!(l.links.len(), 2);
        // The undo: the old main item is the main item again.
        let l = link(&d, "clickup:c1", "github:a/b#1", 4).unwrap();
        assert!(
            l.links.iter().all(|x| x.main == "clickup:c1"),
            "{:?}",
            l.links
        );
        assert_eq!(l.links.len(), 3);
        // Linking two items of one group again makes `main` the main item.
        let l = link(&d, "gmail:m1", "clickup:c1", 5).unwrap();
        assert!(
            l.links.iter().all(|x| x.main == "gmail:m1"),
            "{:?}",
            l.links
        );
        // Unlink an item under the main item.
        let l = unlink(&d, "github:a/b#2", 6).unwrap();
        assert_eq!(l.links.len(), 2);
        assert!(link(&d, "gmail:m1", "gmail:m1", 7).is_err());
        assert!(
            link(&d, "gmail:m1", "github:none#1", 7).is_err(),
            "an unknown item"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_ai_gets_the_groups_with_two_open_items() {
        let d = dir("ai");
        put(&d, "clickup", "c1", "T", "", "{}");
        put(&d, "github", "a/b#1", "T", "", "{}");
        let s = TaskStore::new(&d);
        let t = s.add("T", "", "you").unwrap();
        let tk = format!("task:{}", t.id);
        link(&d, &tk, "clickup:c1", 1).unwrap();
        link(&d, &tk, "github:a/b#1", 1).unwrap();
        let g = db::read(&d, for_ai).unwrap();
        assert_eq!(
            g,
            vec![vec![
                tk.clone(),
                "clickup:c1".to_string(),
                "github:a/b#1".to_string()
            ]]
        );
        // The main item is done: the open items stay one group.
        s.set_done(&t.id, true).unwrap();
        let g = db::read(&d, for_ai).unwrap();
        assert_eq!(
            g,
            vec![vec!["clickup:c1".to_string(), "github:a/b#1".to_string()]]
        );
        std::fs::remove_dir_all(&d).unwrap();
    }
}
