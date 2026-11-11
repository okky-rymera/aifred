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
/// 2: the saved text has the custom fields. Older lists do not have them.
/// 3: the saved tasks have the status group, list, folder, and space, for
///    AIfred's own status (status.rs).
const VERSION: u32 = 3;

/// How long the notes of note_closed stay, in seconds. The undo works only
/// on the day of the close: 2 days is enough for a day that ends late.
const KEEP_CLOSED: u64 = 2 * 24 * 3600;

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

/// One comment that the question chat wrote (ClickupStore::note_comment).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Comment {
    /// The comment id from ClickUp.
    pub id: String,
    /// The comment, as written.
    pub text: String,
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
            crate::links::drop_gone_in(tx, now)?;
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

    /// Notes that the question chat of the item `chat` marked the task `id`
    /// done, and its status from before. reopen_task uses it
    /// (closed_status). The undo works only on the same day, so it also
    /// removes the notes older than KEEP_CLOSED.
    pub fn note_closed(&self, id: &str, before: &str, chat: &str, now: u64) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute(
                "DELETE FROM clickup_closed WHERE ts < ?1",
                [now.saturating_sub(KEEP_CLOSED) as i64],
            )?;
            tx.execute(
                "INSERT INTO clickup_closed (id, before, ts, chat) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT (id) DO UPDATE SET before = ?2, ts = ?3, chat = ?4",
                rusqlite::params![id.trim(), before, now as i64, chat.trim()],
            )?;
            Ok(())
        })
    }

    /// The status from before of a task that the question chat marked
    /// done (note_closed). None: AIfred did not close it.
    pub fn closed_status(&self, id: &str) -> anyhow::Result<Option<String>> {
        db::read(&self.dir, |c| {
            Ok(c.query_row(
                "SELECT before FROM clickup_closed WHERE id = ?1",
                [id.trim()],
                |r| r.get(0),
            )
            .optional()?)
        })
    }

    /// The tasks that the question chat of the item `chat` can put back (the
    /// undo), oldest first: the tasks that this chat marked done at `since`
    /// or later, and the task of the chat itself when a chat closed it then.
    /// Another chat cannot put back a task that it did not close: the text
    /// of its item comes from other people.
    pub fn closed_since(&self, since: u64, chat: &str) -> anyhow::Result<Vec<String>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(
                "SELECT id FROM clickup_closed
                 WHERE ts >= ?1 AND (chat = ?2 OR 'clickup:' || id = ?2)
                 ORDER BY ts, id",
            )?;
            let rows = q.query_map(rusqlite::params![since as i64, chat.trim()], |r| r.get(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Forgets the close of `id`, after the undo put the task back.
    pub fn forget_closed(&self, id: &str) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute("DELETE FROM clickup_closed WHERE id = ?1", [id.trim()])?;
            Ok(())
        })
    }

    /// Notes that the question chat of the item `chat` wrote the comment
    /// `id` with `text` on the task `task`. delete_task_comment uses it
    /// (comments_since). The undo works only on the same day, so it also
    /// removes the notes older than KEEP_CLOSED. `id` must pass
    /// clickup::check_comment_id: the undo gives it to ClickUp in the
    /// address of the call.
    pub fn note_comment(
        &self,
        id: &str,
        task: &str,
        chat: &str,
        text: &str,
        now: u64,
    ) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute(
                "DELETE FROM clickup_comment WHERE ts < ?1",
                [now.saturating_sub(KEEP_CLOSED) as i64],
            )?;
            tx.execute(
                "INSERT INTO clickup_comment (id, task, chat, text, ts) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT (id) DO UPDATE SET task = ?2, chat = ?3, text = ?4, ts = ?5",
                rusqlite::params![id.trim(), task.trim(), chat.trim(), text.trim(), now as i64],
            )?;
            Ok(())
        })
    }

    /// The comments that the question chat of the item `chat` wrote on the
    /// task `task` at `since` or later (note_comment), newest first: the
    /// undo deletes one of them. Two comments in the same second: the one
    /// noted last is first. The ids are text and not always numbers, so
    /// the row order breaks the tie, not the id. Empty: this chat wrote no
    /// comment on the task since then. Another chat cannot delete them:
    /// the text of its item comes from other people.
    pub fn comments_since(
        &self,
        task: &str,
        chat: &str,
        since: u64,
    ) -> anyhow::Result<Vec<Comment>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(
                "SELECT id, text FROM clickup_comment
                 WHERE task = ?1 AND chat = ?2 AND ts >= ?3
                 ORDER BY ts DESC, rowid DESC",
            )?;
            let rows = q.query_map(
                rusqlite::params![task.trim(), chat.trim(), since as i64],
                |r| {
                    Ok(Comment {
                        id: r.get(0)?,
                        text: r.get(1)?,
                    })
                },
            )?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// The tasks with a comment that the question chat of the item `chat`
    /// wrote at `since` or later, each once, oldest first. The chat gets
    /// its comment tools while this is not empty, so the undo works also
    /// when the task left the home page (assistant::ask_side).
    pub fn commented_since(&self, since: u64, chat: &str) -> anyhow::Result<Vec<String>> {
        db::read(&self.dir, |c| {
            let mut q = c.prepare(
                "SELECT task FROM clickup_comment WHERE ts >= ?1 AND chat = ?2
                 GROUP BY task ORDER BY MIN(ts), task",
            )?;
            let rows = q.query_map(rusqlite::params![since as i64, chat.trim()], |r| r.get(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Forgets the comment `id`, after the undo deleted it.
    pub fn forget_comment(&self, id: &str) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute("DELETE FROM clickup_comment WHERE id = ?1", [id.trim()])?;
            Ok(())
        })
    }

    /// Takes one task out of the saved list, after the user marked it done
    /// at `now`. The task stays for search. The time of the list does not
    /// change. A missing task is not an error. The close is AIfred's own,
    /// so its reason is known: Done. A status that the user picked before
    /// the close does not win over it (status.rs). restore is the undo.
    pub fn remove(&self, id: &str, now: u64) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute(
                "UPDATE work_item SET in_list = 0, closed_at = ?3, close_reason = ?4,
                   close_label = NULL, close_tries = 0, close_checked = ?3
                 WHERE source = ?1 AND item_id = ?2 AND in_list = 1",
                rusqlite::params![SOURCE, id, now as i64, crate::status::DONE],
            )?;
            Ok(())
        })
    }

    /// Puts one task back in the saved list, after the undo of a close in
    /// the window (clickup_undo_done) or in the question chat
    /// (reopen_task). The undo of remove: the close columns go back to
    /// what a download sets for an open task (db::save_list), and the
    /// AIfred status column is written again, as after a download
    /// (status::refresh_one_in). The time of the list does not change. A
    /// missing task is not an error.
    pub fn restore(&self, id: &str) -> anyhow::Result<()> {
        let id = id.trim();
        db::write(&self.dir, |tx| {
            tx.execute(
                "UPDATE work_item SET in_list = 1, closed_at = NULL, close_reason = NULL,
                   close_label = NULL, close_tries = 0, close_checked = 0
                 WHERE source = ?1 AND item_id = ?2 AND in_list = 0",
                rusqlite::params![SOURCE, id],
            )?;
            crate::status::refresh_one_in(tx, &format!("{SOURCE}:{id}"))
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

    /// Deletes all saved ClickUp tasks, and the notes of the closes and
    /// the comments of the question chat. Use it when the token changes:
    /// the old tasks can belong to another ClickUp account, and the undo
    /// of a close or a comment must not ask that account.
    pub fn clear(&self) -> anyhow::Result<()> {
        db::write(&self.dir, |tx| {
            tx.execute("DELETE FROM work_item WHERE source = ?1", [SOURCE])?;
            tx.execute("DELETE FROM work_fetch WHERE source = ?1", [SOURCE])?;
            tx.execute("DELETE FROM clickup_closed", [])?;
            tx.execute("DELETE FROM clickup_comment", [])?;
            crate::links::drop_gone_in(tx, crate::files::now())?;
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
            status_type: "open".into(),
            list: "Sprint".into(),
            list_id: "901".into(),
            folder_id: String::new(),
            folder: String::new(),
            space_id: "s1".into(),
            space: "Team".into(),
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
    fn the_comments_of_the_question_chat_are_kept_for_the_undo() {
        let d = temp("comments");
        let s = ClickupStore::new(&d);
        let ids = |task: &str, chat: &str, since: u64| -> Vec<String> {
            s.comments_since(task, chat, since)
                .unwrap()
                .into_iter()
                .map(|c| c.id)
                .collect()
        };
        assert!(ids("c1", "clickup:c1", 0).is_empty());
        s.note_comment(" 8 ", "c1", "clickup:c1", " Eight. ", 90)
            .unwrap();
        s.note_comment(" 9 ", "c1", "clickup:c1", "Nine.", 100)
            .unwrap();
        // The comments of this chat on the task from `since` on, newest
        // first, with their text.
        assert_eq!(
            s.comments_since("c1", "clickup:c1", 0).unwrap(),
            vec![
                Comment {
                    id: "9".into(),
                    text: "Nine.".into()
                },
                Comment {
                    id: "8".into(),
                    text: "Eight.".into()
                },
            ]
        );
        assert_eq!(ids("c1", "clickup:c1", 95), vec!["9"]);
        assert!(ids("c1", "clickup:c1", 101).is_empty());
        // Another chat cannot delete them.
        assert!(ids("c1", "task:t9", 0).is_empty());
        // The tasks with a comment of this chat, each once, oldest first:
        // the chat keeps its comment tools for them.
        s.note_comment("10", "c0", "clickup:c1", "Zero.", 95)
            .unwrap();
        assert_eq!(
            s.commented_since(0, "clickup:c1").unwrap(),
            vec!["c1", "c0"]
        );
        assert_eq!(s.commented_since(96, "clickup:c1").unwrap(), vec!["c1"]);
        assert!(s.commented_since(0, "task:t9").unwrap().is_empty());
        s.forget_comment("10").unwrap();
        s.forget_comment("9").unwrap();
        assert_eq!(ids("c1", "clickup:c1", 0), vec!["8"]);
        // Two comments in the same second: the last one noted is first,
        // also when its id sorts lower as text ("1000" < "999").
        s.note_comment("999", "c1", "clickup:c1", "A", 100).unwrap();
        s.note_comment("1000", "c1", "clickup:c1", "B", 100)
            .unwrap();
        assert_eq!(ids("c1", "clickup:c1", 0), vec!["1000", "999", "8"]);
        s.forget_comment("1000").unwrap();
        assert_eq!(ids("c1", "clickup:c1", 0), vec!["999", "8"]);
        s.forget_comment("999").unwrap();
        // A new comment removes the notes older than KEEP_CLOSED.
        s.note_comment("11", "c2", "task:t1", "Late.", 100 + KEEP_CLOSED + 1)
            .unwrap();
        assert!(ids("c1", "clickup:c1", 0).is_empty());
        assert_eq!(ids("c2", "task:t1", 0), vec!["11"]);
        // A new token: the notes go with the tasks, so the undo never asks
        // another account to delete the comment or put the task back.
        s.note_closed("c3", "to do", "clickup:c3", 100 + KEEP_CLOSED + 1)
            .unwrap();
        s.clear().unwrap();
        assert!(ids("c2", "task:t1", 0).is_empty());
        assert_eq!(s.closed_status("c3").unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_closes_of_the_question_chat_are_kept_for_the_undo() {
        let d = temp("closed");
        let s = ClickupStore::new(&d);
        assert_eq!(s.closed_status("c1").unwrap(), None);
        s.note_closed("c1", "in progress", "clickup:c1", 100)
            .unwrap();
        s.note_closed("c2", "to do", "clickup:c1", 50).unwrap();
        s.note_closed("c3", "to do", "task:t9", 80).unwrap();
        assert_eq!(
            s.closed_status(" c1 ").unwrap().as_deref(),
            Some("in progress")
        );
        assert_eq!(
            s.closed_since(60, "clickup:c1").unwrap(),
            vec!["c1".to_string()]
        );
        assert_eq!(
            s.closed_since(0, "clickup:c1").unwrap(),
            vec!["c2".to_string(), "c1".to_string()]
        );
        // Another chat cannot put back what it did not close. The chat of
        // c3 can put back c3, also when another chat closed it.
        assert_eq!(
            s.closed_since(0, "github:a/b#1").unwrap(),
            Vec::<String>::new()
        );
        assert_eq!(
            s.closed_since(0, "task:t9").unwrap(),
            vec!["c3".to_string()]
        );
        assert_eq!(
            s.closed_since(0, "clickup:c3").unwrap(),
            vec!["c3".to_string()]
        );
        // A new close removes the notes older than KEEP_CLOSED.
        s.note_closed("c4", "to do", "clickup:c4", 100 + KEEP_CLOSED - 10)
            .unwrap();
        assert_eq!(s.closed_status("c2").unwrap(), None);
        assert_eq!(s.closed_status("c3").unwrap(), None);
        assert_eq!(
            s.closed_status("c1").unwrap().as_deref(),
            Some("in progress")
        );
        s.forget_closed("c1").unwrap();
        assert_eq!(s.closed_status("c1").unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
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
        s.remove("a", 5).unwrap();
        s.remove("nope", 5).unwrap();
        let l = s.cached("acct", u64::MAX, 5000).unwrap().unwrap();
        assert_eq!(ids(&l), vec!["b"]);
        assert_eq!(l.fetched_at, 1000);
        // The undo of the close puts the task back, with no close columns,
        // so the question chat can use it again at once. The AIfred status
        // column is written again, as after a download: a stale value does
        // not stay.
        db::write(&d, |tx| {
            tx.execute(
                "UPDATE work_item SET status = 'stale' WHERE item_id = 'a'",
                [],
            )?;
            Ok(())
        })
        .unwrap();
        s.restore(" a ").unwrap();
        s.restore("nope").unwrap();
        let status: String = db::read(&d, |c| {
            Ok(c.query_row(
                "SELECT status FROM work_item WHERE item_id = 'a'",
                [],
                |r| r.get(0),
            )?)
        })
        .unwrap();
        assert_eq!(status, crate::status::TODO);
        let l = s.cached("acct", u64::MAX, 5000).unwrap().unwrap();
        assert_eq!(ids(&l), vec!["b", "a"], "newest change first");
        assert_eq!(l.fetched_at, 1000);
        let close: (bool, Option<String>, Option<i64>) = db::read(&d, |c| {
            Ok(c.query_row(
                "SELECT in_list, close_reason, closed_at FROM work_item WHERE item_id = 'a'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
        .unwrap();
        assert_eq!(close, (true, None, None));
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
        // A saved field is a search word, and the field sign is not.
        s.save_details(
            "a",
            &crate::clickup::details_text(
                "",
                &[crate::clickup::FieldValue {
                    name: "Client".into(),
                    value: "Acme".into(),
                    saved: true,
                }],
            ),
        )
        .unwrap();
        assert_eq!(
            ids(s.search("acct", "acme", false, None).unwrap()),
            vec!["a"]
        );
        let mark = crate::clickup::FIELD_MARK.trim();
        let tokens: i64 = db::read(&d, |c| {
            Ok(c.query_row(
                "SELECT count(*) FROM work_item_fts WHERE work_item_fts MATCH ?1",
                [format!("\"{mark}\"")],
                |r| r.get(0),
            )?)
        })
        .unwrap();
        assert_eq!(tokens, 0, "search does not index the field sign");
        assert!(
            s.search("other", "invoice", false, None)
                .unwrap()
                .is_empty(),
            "another account sees nothing"
        );
        s.remove("b", 5).unwrap();
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
