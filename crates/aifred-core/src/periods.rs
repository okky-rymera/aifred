//! Summaries of the chats by time: one for each day, and one for each
//! quarter that has ended. assistant.rs writes them. A day summary covers
//! the main conversation and the side chats (side.rs) of that day. The
//! messages stay in the database: the AI goes from a summary to the
//! messages of the day with the day_messages tool (aifred-tools memory.rs).

use std::path::Path;

use anyhow::{Context, bail};
use rusqlite::{Connection, OptionalExtension};
use serde::Serialize;

use crate::db;
use crate::files::now;

/// One summary: of a day ("YYYY-MM-DD") or of a quarter ("YYYY-Qn").
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PeriodSummary {
    pub period: String,
    pub summary: String,
    /// When it was written, in Unix seconds.
    pub made: u64,
}

/// One message of a day: from the main conversation or from a side chat.
#[derive(Debug, Clone, Serialize)]
pub struct DayMessage {
    pub ts: u64,
    /// "main", or the side chat thread: "plan:YYYY-MM-DD", "item:<key>", or "mission:<id>".
    pub chat: String,
    pub role: String,
    pub text: String,
    /// The id in its own table (turn or side_turn).
    #[serde(skip)]
    pub id: i64,
}

impl DayMessage {
    /// The label of the message in a summary prompt, for example
    /// "[plan chat for 2026-11-10, user]".
    pub fn label(&self) -> String {
        let chat = match self.chat.split_once(':') {
            None => "main chat".to_string(),
            Some(("plan", day)) => format!("plan chat for {day}"),
            Some(("item", key)) => format!("chat about item {key}"),
            Some(("mission", id)) => format!("chat about mission {id}"),
            Some(_) => self.chat.clone(),
        };
        format!("[{chat}, {}]", self.role)
    }
}

/// The part of a day that its summary does not cover yet.
#[derive(Debug)]
pub struct UnsummarizedDay {
    pub summary: Option<String>,
    /// The last main chat message and the last side chat message in `summary`. 0: none.
    pub turn_upto: i64,
    pub side_upto: i64,
    /// Oldest first.
    pub messages: Vec<DayMessage>,
}

/// An error when `day` is not "YYYY-MM-DD".
pub fn check_day(day: &str) -> anyhow::Result<chrono::NaiveDate> {
    let d = chrono::NaiveDate::parse_from_str(day, "%Y-%m-%d")
        .with_context(|| format!("\"{day}\" is not a day like 2026-11-10"))?;
    // "2026-11-1" parses, but the SQL compares the text with "2026-11-01".
    anyhow::ensure!(
        d.format("%F").to_string() == day,
        "\"{day}\" is not a day like 2026-11-10"
    );
    Ok(d)
}

/// The quarter of a day: "2026-11-10" gives "2026-Q4".
pub fn quarter_of(day: &str) -> anyhow::Result<String> {
    use chrono::Datelike;
    let d = check_day(day)?;
    Ok(format!("{}-Q{}", d.year(), d.month().div_ceil(3)))
}

/// The first and the last day of a quarter: "2026-Q4" gives
/// ("2026-10-01", "2026-12-31").
pub fn quarter_days(quarter: &str) -> anyhow::Result<(String, String)> {
    let bad = || format!("\"{quarter}\" is not a quarter like 2026-Q4");
    let (y, q) = quarter.split_once("-Q").with_context(bad)?;
    let year: i32 = y.parse().with_context(bad)?;
    let q: u32 = q.parse().with_context(bad)?;
    if !(1..=4).contains(&q) {
        bail!("{}", bad());
    }
    let first = chrono::NaiveDate::from_ymd_opt(year, (q - 1) * 3 + 1, 1).with_context(bad)?;
    let next = if q == 4 {
        chrono::NaiveDate::from_ymd_opt(year + 1, 1, 1)
    } else {
        chrono::NaiveDate::from_ymd_opt(year, q * 3 + 1, 1)
    }
    .with_context(bad)?;
    let last = next.pred_opt().with_context(bad)?;
    Ok((
        first.format("%F").to_string(),
        last.format("%F").to_string(),
    ))
}

/// The messages of a day in both tables, as one list. `after_turn` and
/// `after_side` leave out the messages up to those ids.
const DAY_SQL: &str = "
SELECT ts, 'main' AS chat, role, text, id, 0 AS side FROM turn
 WHERE date(ts, 'unixepoch', 'localtime') = ?1 AND id > ?2
UNION ALL
SELECT ts, thread, role, text, id, 1 FROM side_turn
 WHERE date(ts, 'unixepoch', 'localtime') = ?1 AND id > ?3
ORDER BY ts, side, id";

fn messages(
    c: &Connection,
    day: &str,
    after_turn: i64,
    after_side: i64,
) -> anyhow::Result<Vec<DayMessage>> {
    let mut q = c.prepare(DAY_SQL)?;
    let rows = q.query_map(rusqlite::params![day, after_turn, after_side], |r| {
        Ok(DayMessage {
            ts: r.get(0)?,
            chat: r.get(1)?,
            role: r.get(2)?,
            text: r.get(3)?,
            id: r.get(4)?,
        })
    })?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// All messages of a day, oldest first: the main conversation and the side chats.
pub fn messages_of_day(dir: &Path, day: &str) -> anyhow::Result<Vec<DayMessage>> {
    check_day(day)?;
    db::read(dir, |c| messages(c, day, 0, 0))
}

/// The summary of the day, and the messages after it.
pub fn unsummarized_day(dir: &Path, day: &str) -> anyhow::Result<UnsummarizedDay> {
    check_day(day)?;
    db::read(dir, |c| {
        let row: Option<(String, i64, i64)> = c
            .query_row(
                "SELECT summary, turn_upto, side_upto FROM period_summary WHERE period = ?1",
                [day],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let (summary, turn_upto, side_upto) = match row {
            Some((s, t, u)) => (Some(s), t, u),
            None => (None, 0, 0),
        };
        Ok(UnsummarizedDay {
            summary,
            turn_upto,
            side_upto,
            messages: messages(c, day, turn_upto, side_upto)?,
        })
    })
}

/// The days before `today` that need a summary, newest first, at most
/// `limit`. A day needs one when it has messages that its summary does not
/// cover. A day with no summary yet must have at least `min_chars` bytes of
/// messages: a search finds the few messages of a short day directly.
/// `days`: only the days from the first to the second, both included.
pub fn days_to_summarize(
    dir: &Path,
    today: &str,
    min_chars: usize,
    days: Option<(&str, &str)>,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    db::read(dir, |c| waiting_days(c, today, min_chars, days, limit))
}

fn waiting_days(
    c: &Connection,
    today: &str,
    min_chars: usize,
    days: Option<(&str, &str)>,
    limit: usize,
) -> anyhow::Result<Vec<String>> {
    let (from, to) = days.unwrap_or(("0000-00-00", "9999-99-99"));
    let mut q = c.prepare(
        "WITH m AS (
           SELECT date(ts, 'unixepoch', 'localtime') AS day, id AS turn_id, 0 AS side_id,
                  length(CAST(text AS BLOB)) AS n FROM turn
           UNION ALL
           SELECT date(ts, 'unixepoch', 'localtime'), 0, id, length(CAST(text AS BLOB)) FROM side_turn
         )
         SELECT m.day FROM m LEFT JOIN period_summary p ON p.period = m.day
         WHERE m.day < ?1 AND m.day >= ?2 AND m.day <= ?3
         GROUP BY m.day
         HAVING (MAX(m.turn_id) > COALESCE(MAX(p.turn_upto), 0)
              OR MAX(m.side_id) > COALESCE(MAX(p.side_upto), 0))
            AND (MAX(p.id) IS NOT NULL OR SUM(m.n) >= ?4)
         ORDER BY m.day DESC LIMIT ?5",
    )?;
    let rows = q.query_map(
        rusqlite::params![today, from, to, min_chars as i64, limit as i64],
        |r| r.get(0),
    )?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Saves the summary of a day. It covers the main chat messages up to
/// `turn_upto` and the side chat messages up to `side_upto`.
pub fn set_day_summary(
    dir: &Path,
    day: &str,
    summary: &str,
    turn_upto: i64,
    side_upto: i64,
) -> anyhow::Result<()> {
    check_day(day)?;
    db::write(dir, |tx| {
        tx.execute(
            "INSERT INTO period_summary (period, summary, turn_upto, side_upto, made) VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(period) DO UPDATE SET summary = excluded.summary, turn_upto = excluded.turn_upto,
               side_upto = excluded.side_upto, made = excluded.made",
            rusqlite::params![day, summary, turn_upto, side_upto, now() as i64],
        )?;
        Ok(())
    })
}

/// Saves the summary of a quarter.
pub fn set_quarter_summary(dir: &Path, quarter: &str, summary: &str) -> anyhow::Result<()> {
    quarter_days(quarter)?;
    db::write(dir, |tx| {
        tx.execute(
            "INSERT INTO period_summary (period, summary, made) VALUES (?1, ?2, ?3)
             ON CONFLICT(period) DO UPDATE SET summary = excluded.summary, made = excluded.made",
            rusqlite::params![quarter, summary, now() as i64],
        )?;
        Ok(())
    })
}

/// The quarter that needs a summary next, newest first: a quarter that
/// ended before `today`, with day summaries, where no day still waits for
/// its summary (the rule of days_to_summarize), and with no quarter summary
/// or one older than its newest day summary.
pub fn quarter_to_summarize(
    dir: &Path,
    today: &str,
    min_chars: usize,
) -> anyhow::Result<Option<String>> {
    db::read(dir, |c| {
        // The newest day summary of each quarter, newest quarter first.
        let mut q = c.prepare(
            "SELECT period, made FROM period_summary WHERE length(period) = 10 ORDER BY period DESC",
        )?;
        let days: Vec<(String, i64)> = q
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<Result<_, _>>()?;
        let mut quarters: Vec<(String, i64)> = Vec::new();
        for (day, made) in days {
            let quarter = quarter_of(&day)?;
            match quarters.iter_mut().find(|(q, _)| *q == quarter) {
                Some((_, newest)) => *newest = (*newest).max(made),
                None => quarters.push((quarter, made)),
            }
        }
        for (quarter, newest_day) in quarters {
            let (first, last) = quarter_days(&quarter)?;
            if last.as_str() >= today {
                continue;
            }
            let made: Option<i64> = c
                .query_row(
                    "SELECT made FROM period_summary WHERE period = ?1",
                    [&quarter],
                    |r| r.get(0),
                )
                .optional()?;
            if made.is_some_and(|m| m >= newest_day) {
                continue;
            }
            if !waiting_days(c, today, min_chars, Some((&first, &last)), 1)?.is_empty() {
                continue;
            }
            return Ok(Some(quarter));
        }
        Ok(None)
    })
}

fn row(r: &rusqlite::Row) -> rusqlite::Result<PeriodSummary> {
    Ok(PeriodSummary {
        period: r.get(0)?,
        summary: r.get(1)?,
        made: r.get(2)?,
    })
}

/// The summary of a day or a quarter, if it has one.
pub fn get(dir: &Path, period: &str) -> anyhow::Result<Option<PeriodSummary>> {
    db::read(dir, |c| {
        Ok(c.query_row(
            "SELECT period, summary, made FROM period_summary WHERE period = ?1",
            [period],
            row,
        )
        .optional()?)
    })
}

/// The day summaries from `from` to `to`, both included, oldest first.
pub fn day_summaries(dir: &Path, from: &str, to: &str) -> anyhow::Result<Vec<PeriodSummary>> {
    check_day(from)?;
    check_day(to)?;
    db::read(dir, |c| {
        let mut q = c.prepare(
            "SELECT period, summary, made FROM period_summary
             WHERE length(period) = 10 AND period >= ?1 AND period <= ?2 ORDER BY period",
        )?;
        let rows = q.query_map([from, to], row)?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// One day of a range: its summary when it has one, and how many messages it has.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DayOverview {
    pub day: String,
    /// None: the day has no summary (yet, or it is too short). day_messages has its messages.
    pub summary: Option<String>,
    pub messages: usize,
}

/// The days from `from` to `to` (both included) that have messages or a
/// summary, oldest first, with the summary and the number of messages of
/// each. A short day has messages and no summary, so the AI knows to read them.
pub fn days(dir: &Path, from: &str, to: &str) -> anyhow::Result<Vec<DayOverview>> {
    check_day(from)?;
    check_day(to)?;
    db::read(dir, |c| {
        let mut q = c.prepare(
            "WITH m AS (
               SELECT date(ts, 'unixepoch', 'localtime') AS day FROM turn
               UNION ALL
               SELECT date(ts, 'unixepoch', 'localtime') FROM side_turn
             ),
             n AS (SELECT day, COUNT(*) AS messages FROM m WHERE day >= ?1 AND day <= ?2 GROUP BY day),
             p AS (SELECT period AS day, summary FROM period_summary
                   WHERE length(period) = 10 AND period >= ?1 AND period <= ?2)
             SELECT COALESCE(n.day, p.day), p.summary, COALESCE(n.messages, 0)
             FROM n LEFT JOIN p ON p.day = n.day
             UNION
             SELECT p.day, p.summary, COALESCE(n.messages, 0)
             FROM p LEFT JOIN n ON n.day = p.day
             ORDER BY 1",
        )?;
        let rows = q.query_map([from, to], |r| {
            Ok(DayOverview {
                day: r.get(0)?,
                summary: r.get(1)?,
                messages: r.get::<_, i64>(2)? as usize,
            })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    })
}

/// The newest day summary, if there is one.
pub fn newest_day(dir: &Path) -> anyhow::Result<Option<PeriodSummary>> {
    db::read(dir, |c| {
        Ok(c.query_row(
            "SELECT period, summary, made FROM period_summary WHERE length(period) = 10 ORDER BY period DESC LIMIT 1",
            [],
            row,
        )
        .optional()?)
    })
}

/// The day and quarter summaries that match `query` by words or by
/// meaning, best first.
pub fn search(dir: &Path, query: &str, limit: Option<u32>) -> anyhow::Result<Vec<PeriodSummary>> {
    db::read(dir, |c| {
        let ids = db::hybrid(
            c,
            dir,
            query,
            "period_vec",
            "SELECT f.rowid FROM period_fts f WHERE period_fts MATCH ?1 ORDER BY bm25(period_fts)",
            "SELECT 1 FROM period_summary WHERE id = ?1",
            &[],
            limit,
        )?;
        let mut get =
            c.prepare("SELECT period, summary, made FROM period_summary WHERE id = ?1")?;
        Ok(ids
            .iter()
            .map(|id| get.query_row([id], row))
            .collect::<Result<_, _>>()?)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-periods-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// Unix seconds at noon of a local day.
    fn noon(day: &str) -> i64 {
        use chrono::TimeZone;
        let d = check_day(day).unwrap().and_hms_opt(12, 0, 0).unwrap();
        chrono::Local.from_local_datetime(&d).unwrap().timestamp()
    }

    /// A main chat message on `day`, in the one conversation of the test.
    fn main_msg(dir: &Path, day: &str, role: &str, text: &str) {
        db::write(dir, |tx| {
            tx.execute(
                "INSERT INTO conversation (id, started) SELECT 1, ?1 WHERE NOT EXISTS (SELECT 1 FROM conversation WHERE id = 1)",
                [noon(day)],
            )?;
            tx.execute(
                "INSERT INTO turn (conversation_id, ts, role, text, backend) VALUES (1, ?1, ?2, ?3, 'fake')",
                rusqlite::params![noon(day), role, text],
            )?;
            Ok(())
        })
        .unwrap();
    }

    fn side_msg(dir: &Path, day: &str, thread: &str, role: &str, text: &str) {
        db::write(dir, |tx| {
            tx.execute(
                "INSERT INTO side_turn (thread, ts, role, text) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![thread, noon(day), role, text],
            )?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn quarters_have_a_name_and_a_first_and_last_day() {
        assert_eq!(quarter_of("2026-11-10").unwrap(), "2026-Q4");
        assert_eq!(quarter_of("2026-01-01").unwrap(), "2026-Q1");
        assert_eq!(
            quarter_days("2026-Q4").unwrap(),
            ("2026-10-01".to_string(), "2026-12-31".to_string())
        );
        assert_eq!(
            quarter_days("2026-Q1").unwrap(),
            ("2026-01-01".to_string(), "2026-03-31".to_string())
        );
        assert!(quarter_days("2026-Q5").is_err());
        assert!(check_day("2026-13-01").is_err());
    }

    #[test]
    fn a_day_waits_until_its_messages_are_in_a_summary() {
        let d = temp("days");
        let long = "Plan the Bali trip. ".repeat(20); // 400 bytes
        main_msg(&d, "2026-11-08", "user", &long);
        main_msg(&d, "2026-11-08", "assistant", "Flights first.");
        side_msg(&d, "2026-11-09", "plan:2026-11-09", "user", &long);
        main_msg(&d, "2026-11-10", "user", "Hi"); // too short
        main_msg(&d, "2026-11-11", "user", &long); // today
        let waiting = |min: usize| days_to_summarize(&d, "2026-11-11", min, None, 10).unwrap();
        assert_eq!(
            waiting(100),
            vec!["2026-11-09", "2026-11-08"],
            "newest first, no short day, not today"
        );
        assert_eq!(waiting(1), vec!["2026-11-10", "2026-11-09", "2026-11-08"]);
        assert_eq!(
            days_to_summarize(
                &d,
                "2026-11-11",
                100,
                Some(("2026-11-09", "2026-11-09")),
                10
            )
            .unwrap(),
            vec!["2026-11-09"]
        );

        let u = unsummarized_day(&d, "2026-11-08").unwrap();
        assert_eq!(u.summary, None);
        assert_eq!(
            u.messages.iter().map(|m| m.label()).collect::<Vec<_>>(),
            vec!["[main chat, user]", "[main chat, assistant]"]
        );
        let last = u.messages.last().unwrap().id;
        set_day_summary(&d, "2026-11-08", "- Bali trip: flights first.", last, 0).unwrap();
        assert_eq!(
            waiting(100),
            vec!["2026-11-09"],
            "the summary covers the day"
        );
        assert!(
            unsummarized_day(&d, "2026-11-08")
                .unwrap()
                .messages
                .is_empty()
        );

        // A message that arrives later (an import) makes the day wait again,
        // also a short one. The new summary builds on the old one.
        main_msg(&d, "2026-11-08", "user", "Book a hotel too");
        assert_eq!(waiting(100), vec!["2026-11-09", "2026-11-08"]);
        let u = unsummarized_day(&d, "2026-11-08").unwrap();
        assert_eq!(u.summary.as_deref(), Some("- Bali trip: flights first."));
        assert_eq!(u.messages.len(), 1);

        let side = unsummarized_day(&d, "2026-11-09").unwrap();
        assert_eq!(side.messages[0].label(), "[plan chat for 2026-11-09, user]");
        assert_eq!(messages_of_day(&d, "2026-11-10").unwrap()[0].text, "Hi");
        let overview = days(&d, "2026-11-08", "2026-11-10").unwrap();
        assert_eq!(
            overview
                .iter()
                .map(|o| (o.day.as_str(), o.summary.is_some(), o.messages))
                .collect::<Vec<_>>(),
            vec![
                ("2026-11-08", true, 3),
                ("2026-11-09", false, 1),
                ("2026-11-10", false, 1)
            ],
            "a short day shows its messages and no summary"
        );
        assert_eq!(newest_day(&d).unwrap().unwrap().period, "2026-11-08");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn a_quarter_gets_a_summary_after_it_ends_and_its_days_are_done() {
        let d = temp("quarters");
        let long = "Release planning. ".repeat(20);
        main_msg(&d, "2026-08-03", "user", &long);
        main_msg(&d, "2026-08-20", "user", &long);
        main_msg(&d, "2026-11-02", "user", &long);
        let next = |today: &str| quarter_to_summarize(&d, today, 100).unwrap();
        assert_eq!(next("2026-11-11"), None, "no day summaries yet");
        set_day_summary(&d, "2026-08-03", "- Planned the release.", 1, 0).unwrap();
        assert_eq!(next("2026-11-11"), None, "2026-08-20 still waits");
        set_day_summary(&d, "2026-08-20", "- Released.", 2, 0).unwrap();
        assert_eq!(next("2026-09-30"), None, "the quarter has not ended");
        assert_eq!(next("2026-11-11").as_deref(), Some("2026-Q3"));
        set_quarter_summary(&d, "2026-Q3", "- Q3: the release.").unwrap();
        assert_eq!(next("2026-11-11"), None, "done; Q4 is still running");
        assert_eq!(
            get(&d, "2026-Q3").unwrap().unwrap().summary,
            "- Q3: the release."
        );
        // A new day summary in the quarter: the quarter summary is made again.
        std::thread::sleep(std::time::Duration::from_millis(1100));
        set_day_summary(&d, "2026-08-20", "- Released, with one bug.", 2, 0).unwrap();
        assert_eq!(next("2026-11-11").as_deref(), Some("2026-Q3"));
        assert_eq!(
            day_summaries(&d, "2026-08-01", "2026-08-31")
                .unwrap()
                .iter()
                .map(|s| s.period.as_str())
                .collect::<Vec<_>>(),
            vec!["2026-08-03", "2026-08-20"]
        );
        let found = search(&d, "releases", None).unwrap();
        assert!(found.iter().any(|s| s.period == "2026-Q3"), "{found:?}");
        assert!(found.iter().any(|s| s.period == "2026-08-20"), "{found:?}");
        std::fs::remove_dir_all(&d).unwrap();
    }
}
