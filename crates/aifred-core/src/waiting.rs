//! The "waiting for" list: what the user waits for from another person.
//! For example "the signed contract from Ana, since Mon 2 Nov".
//!
//! An item can come from an email that the user sent (gmail.rs), or the
//! user (or the AI in the chat) adds it by hand.
//!
//! - An item from an email closes by itself when the person answers in the
//!   same thread: one of the people that the user wrote to. A message of
//!   another person in the thread does not close it. The next Gmail
//!   download sees the answer.
//! - An item can have a follow-up day. On that day, at the start of the
//!   work day, AIfred sends a notification when the item is still open.
//!   This is "tell me if Ana has not answered by Friday". It costs no AI call.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use anyhow::bail;
use rusqlite::{Connection, OptionalExtension, Transaction};
use serde::{Deserialize, Serialize};

use crate::db;

/// At most this many open items.
pub const MAX_OPEN: usize = 50;
/// The longest text of `what` and `who`.
pub const MAX_CHARS: usize = 200;
/// A closed item stays in the list this long, so the user sees that the
/// person answered.
const SHOW_CLOSED_SECS: u64 = 3 * 86_400;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Waiting {
    pub id: i64,
    /// What the user waits for.
    pub what: String,
    /// Who the user waits for.
    pub who: String,
    /// Since when, "YYYY-MM-DD".
    pub since: String,
    /// The day to check, "YYYY-MM-DD", or None.
    pub follow_up: Option<String>,
    /// True after the notification of the follow-up day.
    pub told: bool,
    /// The Gmail thread, when the item came from an email.
    pub thread: Option<String>,
    /// The thread in Gmail.
    pub url: Option<String>,
    pub done: bool,
    /// Why the item closed, for example "Ana Lee answered on Thu 5 Nov".
    pub done_why: String,
    pub created: u64,
    pub updated: u64,
}

const COLUMNS: &str =
    "id, what, who, since, follow_up, told, thread, url, done, done_why, created, updated";

fn row(r: &rusqlite::Row) -> rusqlite::Result<Waiting> {
    Ok(Waiting {
        id: r.get(0)?,
        what: r.get(1)?,
        who: r.get(2)?,
        since: r.get(3)?,
        follow_up: r.get(4)?,
        told: r.get(5)?,
        thread: r.get(6)?,
        url: r.get(7)?,
        done: r.get(8)?,
        done_why: r.get(9)?,
        created: r.get(10)?,
        updated: r.get(11)?,
    })
}

fn get(c: &Connection, id: i64) -> anyhow::Result<Waiting> {
    let w = c
        .query_row(
            &format!("SELECT {COLUMNS} FROM waiting WHERE id = ?1"),
            [id],
            row,
        )
        .optional()?;
    match w {
        Some(w) => Ok(w),
        None => bail!("no \"waiting for\" item with id {id}"),
    }
}

/// The open items, the oldest first, then the items that closed in the
/// last days. `all`: all closed items too.
pub fn list(dir: &Path, all: bool, now: u64) -> anyhow::Result<Vec<Waiting>> {
    db::read(dir, |c| list_in(c, all, now))
}

pub(crate) fn list_in(c: &Connection, all: bool, now: u64) -> anyhow::Result<Vec<Waiting>> {
    let since = if all {
        0
    } else {
        now.saturating_sub(SHOW_CLOSED_SECS)
    };
    let mut q = c.prepare(&format!(
        "SELECT {COLUMNS} FROM waiting WHERE done = 0 OR updated >= ?1
         ORDER BY done, since, id"
    ))?;
    let rows = q.query_map([since as i64], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// The open items, the oldest first.
pub(crate) fn open_in(c: &Connection) -> anyhow::Result<Vec<Waiting>> {
    let mut q = c.prepare(&format!(
        "SELECT {COLUMNS} FROM waiting WHERE done = 0 ORDER BY since, id"
    ))?;
    let rows = q.query_map([], row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// A new item.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct NewWaiting {
    /// Empty: the subject of the email.
    #[serde(default)]
    pub what: String,
    /// Empty: the people of the email.
    #[serde(default)]
    pub who: String,
    /// "YYYY-MM-DD". None: the day of the email, else today.
    #[serde(default)]
    pub since: Option<String>,
    #[serde(default)]
    pub follow_up: Option<String>,
    /// A thread of the saved "sent" list (gmail.rs).
    #[serde(default)]
    pub thread: Option<String>,
}

/// A change. Only the fields that are Some change.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct WaitingChange {
    #[serde(default)]
    pub what: Option<String>,
    #[serde(default)]
    pub who: Option<String>,
    /// Some(""): no follow-up day.
    #[serde(default)]
    pub follow_up: Option<String>,
    /// true closes the item, false opens it again.
    #[serde(default)]
    pub done: Option<bool>,
}

/// A text of one line, at most MAX_CHARS characters.
fn clean(text: &str, name: &str) -> anyhow::Result<String> {
    let t = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if t.is_empty() {
        bail!("{name} is empty");
    }
    if t.chars().count() > MAX_CHARS {
        bail!("{name} is longer than {MAX_CHARS} characters");
    }
    Ok(t)
}

/// A day "YYYY-MM-DD" that exists.
fn check_day(day: &str) -> anyhow::Result<String> {
    let d = day.trim();
    crate::plan::check_day(d)?;
    if chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").is_err() {
        bail!("the day \"{d}\" does not exist");
    }
    Ok(d.to_string())
}

/// The local day of a time, "YYYY-MM-DD".
fn day_of(t: i64) -> String {
    crate::gmail::local_time(t)
        .map(|d| d.format("%F").to_string())
        .unwrap_or_else(crate::plan::today)
}

/// At most MAX_CHARS characters. A longer text ends with "…".
fn shorten(text: &str) -> String {
    if text.chars().count() <= MAX_CHARS {
        return text.to_string();
    }
    let mut t: String = text.chars().take(MAX_CHARS - 1).collect();
    t.push('…');
    t
}

pub fn add(dir: &Path, n: NewWaiting, now: u64) -> anyhow::Result<Waiting> {
    db::write(dir, |tx| {
        check_room(tx)?;
        let mut what = n.what.clone();
        let mut who = n.who.clone();
        let mut since = n.since.clone();
        let (mut sent_at, mut url, mut wait_for) = (None, None, String::new());
        let thread = n.thread.as_deref().map(str::trim).filter(|t| !t.is_empty());
        if let Some(t) = thread {
            let Some(m) = crate::gmail::sent_mail(dir, tx, t)? else {
                bail!(
                    "\"{t}\" is not an email that waits for an answer. Use a thread from no_reply_yet in the overview"
                );
            };
            check_thread_free(tx, t, None)?;
            if what.trim().is_empty() {
                what = shorten(&format!("An answer to \"{}\"", m.subject));
            }
            if who.trim().is_empty() {
                who = shorten(&m.people.join(", "));
            }
            if since.is_none() {
                since = Some(day_of(m.since));
            }
            sent_at = Some(m.since);
            url = Some(m.url);
            let emails = if m.emails.is_empty() {
                vec![m.email]
            } else {
                m.emails
            };
            // ",a@x.com,b@y.com,": apply_answers finds ",a@x.com," in it.
            let emails: Vec<String> = emails.into_iter().filter(|e| !e.is_empty()).collect();
            if !emails.is_empty() {
                wait_for = format!(",{},", emails.join(","));
            }
        }
        let what = clean(&what, "what")?;
        let who = clean(&who, "who")?;
        let since = match since {
            Some(d) => check_day(&d)?,
            None => crate::plan::today(),
        };
        let follow_up = n.follow_up.as_deref().map(check_day).transpose()?;
        tx.execute(
            "INSERT INTO waiting (what, who, since, follow_up, thread, sent_at, url, wait_for, created, updated)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)",
            rusqlite::params![what, who, since, follow_up, thread, sent_at, url, wait_for, now as i64],
        )?;
        get(tx, tx.last_insert_rowid())
    })
}

pub fn update(dir: &Path, id: i64, ch: WaitingChange, now: u64) -> anyhow::Result<Waiting> {
    db::write(dir, |tx| {
        let mut w = get(tx, id)?;
        if let Some(t) = &ch.what {
            w.what = clean(t, "what")?;
        }
        if let Some(t) = &ch.who {
            w.who = clean(t, "who")?;
        }
        if let Some(f) = &ch.follow_up {
            let f = f.trim();
            let new = if f.is_empty() {
                None
            } else {
                Some(check_day(f)?)
            };
            if new != w.follow_up {
                // A new day gets its own notification.
                w.told = false;
            }
            w.follow_up = new;
        }
        if let Some(done) = ch.done
            && done != w.done
        {
            if !done {
                check_room(tx)?;
                if let Some(t) = &w.thread {
                    check_thread_free(tx, t, Some(id))?;
                }
                // An item that opens again gets the notification of its
                // follow-up day again. An email item keeps sent_at: when an
                // answer closed it, apply_answers set sent_at to that
                // answer, so only a newer answer closes it again. When the
                // user closed it, an answer that came before still counts.
                w.told = false;
            }
            w.done = done;
            w.done_why = if done {
                "closed by you".into()
            } else {
                String::new()
            };
        }
        tx.execute(
            "UPDATE waiting SET what = ?2, who = ?3, follow_up = ?4, told = ?5, done = ?6, done_why = ?7, updated = ?8
             WHERE id = ?1",
            rusqlite::params![id, w.what, w.who, w.follow_up, w.told, w.done, w.done_why, now as i64],
        )?;
        get(tx, id)
    })
}

/// An error when the list has MAX_OPEN open items.
fn check_room(c: &Connection) -> anyhow::Result<()> {
    let n: i64 = c.query_row("SELECT COUNT(*) FROM waiting WHERE done = 0", [], |r| {
        r.get(0)
    })?;
    if n as usize >= MAX_OPEN {
        bail!("the \"waiting for\" list has {MAX_OPEN} open items. Close some first");
    }
    Ok(())
}

/// An error when an open item other than `except` has the thread. One
/// thread has at most one open item.
fn check_thread_free(c: &Connection, thread: &str, except: Option<i64>) -> anyhow::Result<()> {
    let has: Option<i64> = c
        .query_row(
            "SELECT id FROM waiting WHERE thread = ?1 AND done = 0 AND id != ?2",
            rusqlite::params![thread, except.unwrap_or(-1)],
            |r| r.get(0),
        )
        .optional()?;
    if let Some(id) = has {
        bail!("this email is already on the \"waiting for\" list (item {id})");
    }
    Ok(())
}

pub fn delete(dir: &Path, id: i64) -> anyhow::Result<()> {
    db::write(dir, |tx| {
        if tx.execute("DELETE FROM waiting WHERE id = ?1", [id])? == 0 {
            bail!("no \"waiting for\" item with id {id}");
        }
        Ok(())
    })
}

/// The open items whose follow-up day has come and that got no
/// notification yet. The day comes at `start_minute` (the start of the
/// work day, in minutes after midnight) of `today`.
pub fn due(
    dir: &Path,
    today: &str,
    minute_now: u32,
    start_minute: u32,
) -> anyhow::Result<Vec<Waiting>> {
    db::read(dir, |c| {
        let mut q = c.prepare(&format!(
            "SELECT {COLUMNS} FROM waiting WHERE done = 0 AND told = 0 AND follow_up IS NOT NULL
             AND (follow_up < ?1 OR (follow_up = ?1 AND ?2))
             ORDER BY follow_up, id"
        ))?;
        let rows = q.query_map(rusqlite::params![today, minute_now >= start_minute], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// due() for now: today in the user's time zone, and the start of the
/// work day from the settings.
pub fn due_now(dir: &Path) -> anyhow::Result<Vec<Waiting>> {
    use chrono::Timelike;
    let s = crate::settings::load_or_default(dir);
    let start = crate::settings::minutes_of(&s.work_start).unwrap_or(540);
    let now = chrono::Local::now();
    due(
        dir,
        &now.format("%F").to_string(),
        now.hour() * 60 + now.minute(),
        start,
    )
}

/// Marks the notification of the follow-up day as sent. Only when the item
/// is still open, not told, and still has `follow_up`: a change during the
/// check gets its own notification. Returns false when nothing changed:
/// then the notification must not show.
pub fn mark_told(dir: &Path, id: i64, follow_up: &str) -> anyhow::Result<bool> {
    db::write(dir, |tx| {
        let n = tx.execute(
            "UPDATE waiting SET told = 1
             WHERE id = ?1 AND follow_up = ?2 AND done = 0 AND told = 0",
            rusqlite::params![id, follow_up],
        )?;
        Ok(n > 0)
    })
}

/// The Gmail threads of the open items.
pub(crate) fn tracked_threads(dir: &Path) -> anyhow::Result<Vec<String>> {
    db::read(dir, |c| Ok(open_threads(c)?.into_iter().collect()))
}

pub(crate) fn open_threads(c: &Connection) -> anyhow::Result<HashSet<String>> {
    let mut q = c.prepare("SELECT thread FROM waiting WHERE done = 0 AND thread IS NOT NULL")?;
    let rows = q.query_map([], |r| r.get(0))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// For each thread on the list: until when its messages are known. A
/// "sent" email of the user at or before this time is not a suggestion.
/// An open item covers all times. A closed item covers its sent_at: a new
/// email of the user in the thread is a suggestion again.
pub(crate) fn covered_threads(c: &Connection) -> anyhow::Result<HashMap<String, i64>> {
    let mut q = c.prepare(
        "SELECT thread, MAX(CASE WHEN done = 0 THEN 9223372036854775807 ELSE COALESCE(sent_at, 0) END)
         FROM waiting WHERE thread IS NOT NULL GROUP BY thread",
    )?;
    let rows = q.query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Closes the open items whose person answered: a message in the thread
/// after the user's email, from one of the people that the user wrote to
/// (wait_for). An item with no wait_for closes with an answer of anyone.
/// sent_at becomes the time of the answer: when the user opens the item
/// again, only a newer answer closes it.
pub(crate) fn apply_answers(
    tx: &Transaction,
    answers: &[crate::gmail::Answer],
    now: u64,
) -> anyhow::Result<()> {
    let mut q = tx.prepare(
        "UPDATE waiting SET done = 1, done_why = ?3, updated = ?4, sent_at = ?2
         WHERE done = 0 AND thread = ?1 AND sent_at IS NOT NULL AND sent_at < ?2
           AND (wait_for = '' OR instr(wait_for, ?5) > 0)",
    )?;
    for a in answers {
        let who = if a.from.is_empty() {
            "Someone".to_string()
        } else {
            a.from.clone()
        };
        let why = format!("{who} answered on {}", crate::gmail::day_text(a.at));
        let email = format!(",{},", a.email);
        q.execute(rusqlite::params![a.thread, a.at, why, now as i64, email])?;
    }
    Ok(())
}

/// The notification of a follow-up day.
pub fn reminder(w: &Waiting) -> (String, String) {
    let since = chrono::NaiveDate::parse_from_str(&w.since, "%Y-%m-%d")
        .map(|d| d.format(crate::gmail::DAY_FORMAT).to_string())
        .unwrap_or_else(|_| w.since.clone());
    (
        format!("No answer from {} yet", w.who),
        format!("{}. You wait since {since}.", w.what),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gmail::tests::{mail, save_fake, temp};

    fn new(what: &str, who: &str) -> NewWaiting {
        NewWaiting {
            what: what.into(),
            who: who.into(),
            ..Default::default()
        }
    }

    #[test]
    fn shorten_keeps_a_text_short() {
        assert_eq!(shorten("Ana, Bo"), "Ana, Bo");
        let long = "a".repeat(MAX_CHARS + 50);
        let c = shorten(&long);
        assert_eq!(c.chars().count(), MAX_CHARS);
        assert!(c.ends_with('…'));
        assert!(clean(&c, "who").is_ok());
    }

    #[test]
    fn add_change_close_and_delete() {
        let d = temp("waiting");
        let w = add(
            &d,
            NewWaiting {
                since: Some("2026-11-02".into()),
                follow_up: Some("2026-11-06".into()),
                ..new("  The signed   contract ", "Ana")
            },
            100,
        )
        .unwrap();
        assert_eq!(w.what, "The signed contract");
        assert_eq!(
            (w.since.as_str(), w.follow_up.as_deref()),
            ("2026-11-02", Some("2026-11-06"))
        );
        assert!(add(&d, new("", "Ana"), 100).is_err(), "no what");
        assert!(
            add(
                &d,
                NewWaiting {
                    follow_up: Some("2026-02-30".into()),
                    ..new("x", "y")
                },
                100
            )
            .is_err()
        );
        assert!(
            add(
                &d,
                NewWaiting {
                    follow_up: Some("Friday".into()),
                    ..new("x", "y")
                },
                100
            )
            .is_err()
        );

        let w2 = update(
            &d,
            w.id,
            WaitingChange {
                follow_up: Some("".into()),
                ..Default::default()
            },
            200,
        )
        .unwrap();
        assert_eq!(w2.follow_up, None);
        let w3 = update(
            &d,
            w.id,
            WaitingChange {
                done: Some(true),
                ..Default::default()
            },
            300,
        )
        .unwrap();
        assert!(w3.done);
        assert_eq!(w3.done_why, "closed by you");
        assert_eq!(
            list(&d, false, 300).unwrap().len(),
            1,
            "a closed item stays some days"
        );
        assert!(
            list(&d, false, 300 + SHOW_CLOSED_SECS + 1)
                .unwrap()
                .is_empty()
        );
        assert_eq!(list(&d, true, 300 + SHOW_CLOSED_SECS + 1).unwrap().len(), 1);
        let w4 = update(
            &d,
            w.id,
            WaitingChange {
                done: Some(false),
                ..Default::default()
            },
            400,
        )
        .unwrap();
        assert!(!w4.done && w4.done_why.is_empty());
        delete(&d, w.id).unwrap();
        assert!(delete(&d, w.id).is_err());
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn follow_ups_come_at_the_start_of_the_day_once() {
        let d = temp("follow");
        let a = add(
            &d,
            NewWaiting {
                follow_up: Some("2026-11-06".into()),
                ..new("Contract", "Ana")
            },
            100,
        )
        .unwrap();
        add(
            &d,
            NewWaiting {
                follow_up: Some("2026-11-09".into()),
                ..new("Invoice", "Bo")
            },
            100,
        )
        .unwrap();
        add(&d, new("No day", "Cy"), 100).unwrap();
        assert!(
            due(&d, "2026-11-05", 600, 540).unwrap().is_empty(),
            "the day before"
        );
        assert!(
            due(&d, "2026-11-06", 500, 540).unwrap().is_empty(),
            "before the work day"
        );
        let now = due(&d, "2026-11-06", 540, 540).unwrap();
        assert_eq!(now.iter().map(|w| w.id).collect::<Vec<_>>(), [a.id]);
        assert_eq!(
            due(&d, "2026-11-07", 0, 540).unwrap().len(),
            1,
            "a missed day comes at once"
        );
        assert!(mark_told(&d, a.id, "2026-11-06").unwrap());
        assert!(!mark_told(&d, a.id, "2026-11-06").unwrap(), "only once");
        assert!(due(&d, "2026-11-07", 0, 540).unwrap().is_empty(), "once");
        // A new day gets a new notification.
        update(
            &d,
            a.id,
            WaitingChange {
                follow_up: Some("2026-11-08".into()),
                ..Default::default()
            },
            200,
        )
        .unwrap();
        assert_eq!(due(&d, "2026-11-08", 600, 540).unwrap().len(), 1);
        // A told mark for an old day does not count.
        assert!(!mark_told(&d, a.id, "2026-11-06").unwrap());
        assert_eq!(due(&d, "2026-11-08", 600, 540).unwrap().len(), 1);
        let (title, body) = reminder(&now[0]);
        assert_eq!(title, "No answer from Ana yet");
        assert!(body.starts_with("Contract. You wait since "), "{body}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_email_item_closes_when_the_person_answers() {
        let d = temp("answer");
        crate::google::fake_login(&d, "me@x.com");
        let sent = mail("s1", "sent", 1000);
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![sent.clone(), mail("s2", "sent", 900)],
            vec![],
            2000,
        )
        .unwrap();
        assert!(
            add(
                &d,
                NewWaiting {
                    thread: Some("nope".into()),
                    ..Default::default()
                },
                2000
            )
            .is_err()
        );
        let w = add(
            &d,
            NewWaiting {
                thread: Some("s1".into()),
                ..Default::default()
            },
            2000,
        )
        .unwrap();
        assert_eq!(w.what, "An answer to \"Subject s1\"");
        assert_eq!(w.who, "Ana Lee");
        assert_eq!(w.url.as_deref(), Some(sent.url.as_str()));
        assert!(
            add(
                &d,
                NewWaiting {
                    thread: Some("s1".into()),
                    ..Default::default()
                },
                2000
            )
            .is_err(),
            "twice"
        );
        // A tracked thread leaves the suggestions.
        let l = crate::gmail::cached(&d, u64::MAX, 2000).unwrap().unwrap();
        assert_eq!(
            l.sent.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["s2"]
        );
        assert_eq!(tracked_threads(&d).unwrap(), ["s1"]);

        // An older message of Ana does not count.
        let old = crate::gmail::Answer {
            thread: "s1".into(),
            at: 500,
            from: "Ana Lee".into(),
            email: "ana@y.com".into(),
        };
        save_fake(&d, "me@x.com", vec![], vec![], vec![old], 3000).unwrap();
        assert!(!list(&d, false, 3000).unwrap()[0].done);
        let new = crate::gmail::Answer {
            thread: "s1".into(),
            at: 2500,
            from: "Ana Lee".into(),
            email: "ana@y.com".into(),
        };
        save_fake(&d, "me@x.com", vec![], vec![], vec![new.clone()], 3000).unwrap();
        let w = &list(&d, false, 3000).unwrap()[0];
        assert!(w.done);
        assert!(
            w.done_why.starts_with("Ana Lee answered on "),
            "{}",
            w.done_why
        );
        assert!(tracked_threads(&d).unwrap().is_empty());

        // Opened again, the item waits for a new answer: the old one does
        // not close it again.
        let open_again = WaitingChange {
            done: Some(false),
            ..Default::default()
        };
        update(&d, w.id, open_again.clone(), 3100).unwrap();
        save_fake(&d, "me@x.com", vec![], vec![], vec![new.clone()], 3200).unwrap();
        assert!(!list(&d, false, 3200).unwrap()[0].done);
        let newer = crate::gmail::Answer { at: 3150, ..new };
        save_fake(&d, "me@x.com", vec![], vec![], vec![newer], 3300).unwrap();
        assert!(list(&d, false, 3300).unwrap()[0].done);

        // One thread has at most one open item.
        save_fake(&d, "me@x.com", vec![], vec![sent], vec![], 3400).unwrap();
        let b = add(
            &d,
            NewWaiting {
                thread: Some("s1".into()),
                ..Default::default()
            },
            3400,
        )
        .unwrap();
        let e = update(&d, w.id, open_again, 3500).unwrap_err();
        assert!(e.to_string().contains(&format!("item {}", b.id)), "{e}");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_answer_of_another_person_in_the_thread_does_not_close_the_item() {
        let d = temp("other-person");
        crate::google::fake_login(&d, "me@x.com");
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![mail("s1", "sent", 1000)],
            vec![],
            2000,
        )
        .unwrap();
        add(
            &d,
            NewWaiting {
                thread: Some("s1".into()),
                ..Default::default()
            },
            2000,
        )
        .unwrap();
        let answer = |from: &str, email: &str, at| crate::gmail::Answer {
            thread: "s1".into(),
            at,
            from: from.into(),
            email: email.into(),
        };
        // Bo is in the thread, but the user did not write to Bo.
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![],
            vec![answer("Bo", "bo@z.com", 1500)],
            2100,
        )
        .unwrap();
        assert!(!list(&d, false, 2100).unwrap()[0].done, "Bo is not Ana");
        // Ana answered before Bo: her answer counts.
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![],
            vec![
                answer("Bo", "bo@z.com", 1500),
                answer("Ana Lee", "ana@y.com", 1200),
            ],
            2200,
        )
        .unwrap();
        let w = &list(&d, false, 2200).unwrap()[0];
        assert!(
            w.done && w.done_why.starts_with("Ana Lee answered"),
            "{w:?}"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_item_closed_by_hand_still_closes_with_an_earlier_answer() {
        let d = temp("by-hand");
        crate::google::fake_login(&d, "me@x.com");
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![mail("s1", "sent", 1000)],
            vec![],
            2000,
        )
        .unwrap();
        let w = add(
            &d,
            NewWaiting {
                thread: Some("s1".into()),
                ..Default::default()
            },
            2000,
        )
        .unwrap();
        let close = |done| WaitingChange {
            done: Some(done),
            ..Default::default()
        };
        // "Got it" by mistake, then Undo, before the download of Ana's answer.
        update(&d, w.id, close(true), 2100).unwrap();
        update(&d, w.id, close(false), 2200).unwrap();
        let answer = crate::gmail::Answer {
            thread: "s1".into(),
            at: 1500,
            from: "Ana Lee".into(),
            email: "ana@y.com".into(),
        };
        save_fake(&d, "me@x.com", vec![], vec![], vec![answer], 2300).unwrap();
        assert!(
            list(&d, false, 2300).unwrap()[0].done,
            "the answer at 1500 counts"
        );
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_closed_email_item_is_not_a_suggestion_until_a_new_email() {
        let d = temp("closed-sugg");
        crate::google::fake_login(&d, "me@x.com");
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![mail("s1", "sent", 1000)],
            vec![],
            2000,
        )
        .unwrap();
        let w = add(
            &d,
            NewWaiting {
                thread: Some("s1".into()),
                ..Default::default()
            },
            2000,
        )
        .unwrap();
        update(
            &d,
            w.id,
            WaitingChange {
                done: Some(true),
                ..Default::default()
            },
            2100,
        )
        .unwrap();
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![mail("s1", "sent", 1000)],
            vec![],
            2200,
        )
        .unwrap();
        let l = crate::gmail::cached(&d, u64::MAX, 2200).unwrap().unwrap();
        assert!(l.sent.is_empty(), "closed by hand: not a suggestion");
        // A new email of the user in the thread is a suggestion again.
        save_fake(
            &d,
            "me@x.com",
            vec![],
            vec![mail("s1", "sent", 3000)],
            vec![],
            3100,
        )
        .unwrap();
        let l = crate::gmail::cached(&d, u64::MAX, 3100).unwrap().unwrap();
        assert_eq!(l.sent.len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_reopened_item_gets_its_follow_up_again() {
        let d = temp("reopen-told");
        let w = add(
            &d,
            NewWaiting {
                follow_up: Some("2026-11-06".into()),
                ..new("Contract", "Ana")
            },
            100,
        )
        .unwrap();
        assert!(mark_told(&d, w.id, "2026-11-06").unwrap());
        let change = |done| WaitingChange {
            done: Some(done),
            ..Default::default()
        };
        update(&d, w.id, change(true), 200).unwrap();
        update(&d, w.id, change(false), 300).unwrap();
        assert_eq!(due(&d, "2026-11-07", 600, 540).unwrap().len(), 1);
        std::fs::remove_dir_all(&d).unwrap();
    }
}
