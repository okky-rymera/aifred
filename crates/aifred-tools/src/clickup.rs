//! ClickUp tools for aifred, served over MCP (stdio). They read, apart
//! from the four tools of the question chat (below).
//!
//! The server reads the ClickUp token from the aifred data folder
//! ($AIFRED_HOME, or ~/.aifred), or from AIFRED_CLICKUP_TOKEN. The token never
//! goes into the tool settings file that Claude reads.
//!
//! These tools only read, with one exception: the question chat on the home
//! page (QUESTION_CHAT_ENV) can mark a task that the home page asked about
//! today done when the user says it is finished, put the status from
//! before back, and write a comment on such a task in the user's words
//! (also on a task that it closed today, QUESTION_REOPEN_ENV). The user
//! also marks ClickUp tasks done in the window.
//!
//! my_tasks uses the list that the window saved in the database while it is
//! new enough, and saves each new list it gets from ClickUp. search_my_tasks
//! searches that saved list. task_details saves the description for search.

use aifred_core::clickup::{self, Client};
use aifred_core::clickup_list::{ClickupList, ClickupStore};
use aifred_core::files::now;
use aifred_core::text::cut;
use aifred_core::{default_data_dir, settings};
use rmcp::{
    ErrorData, ServerHandler, ServiceExt, handler::server::wrapper::Parameters, model::*, tool,
    tool_handler, tool_router, transport::stdio,
};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{Value, json};

const MAX_TEXT_CHARS: usize = 4000;
const MAX_COMMENTS: usize = 5;

#[derive(Clone)]
struct ClickupTools {
    dir: std::path::PathBuf,
}

#[derive(Deserialize, JsonSchema)]
struct ListArgs {
    /// Maximum number of tasks to return (1-200). Default 50.
    limit: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct SearchArgs {
    /// Words to find in the task name, the workspace and list, and the saved description. Example: "invoice client".
    query: String,
    /// Also include tasks that are no longer in the user's list, for example done tasks. Default false.
    include_old: Option<bool>,
    /// Maximum number of tasks to return (1-50). Default 10.
    limit: Option<u32>,
}

/// The task of a tool that changes ClickUp. The team's custom id (DEV-42)
/// is not accepted: the lists of the question chat and the notes for the
/// undo hold ClickUp's own id.
#[derive(Deserialize, JsonSchema)]
struct TaskArgs {
    /// ClickUp task id from my_tasks (for example "86abc123"), or the item key "clickup:86abc123". Not the team's custom id.
    id: String,
}

#[derive(Deserialize, JsonSchema)]
struct CommentArgs {
    /// ClickUp task id from my_tasks (for example "86abc123"), or the item key "clickup:86abc123". Not the team's custom id.
    id: String,
    /// The comment, in the user's own words from this chat, plain text, short.
    text: String,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteCommentArgs {
    /// ClickUp task id from my_tasks (for example "86abc123"), or the item key "clickup:86abc123". Not the team's custom id.
    id: String,
    /// The comment to delete, from add_task_comment. Leave it out when this chat wrote one comment on the task today: that one goes.
    comment_id: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct DetailArgs {
    /// ClickUp task id from my_tasks (for example "86abc123"), or the team's custom id (for example "DEV-42").
    id: String,
}

/// True when the question chat of the home page started this server and
/// `id` is in the list of the env value `var` (one task id on each line):
/// QUESTION_CHAT_ENV for mark_task_done (the tasks that the home page asked
/// about today and that are still open in ClickUp), QUESTION_REOPEN_ENV for
/// reopen_task (the tasks that the question chat closed today). Other chats
/// get item text from other people, and that text must not change tasks.
fn in_question_chat(var: &str, id: &str) -> bool {
    let allowed = std::env::var(var).unwrap_or_default();
    allowed.lines().any(|t| t.trim() == id.trim())
}

/// An error unless in_question_chat. `scope` names the tasks of the list,
/// and `advice` says what the user can do instead: both differ for a
/// close and for its undo.
fn check_question_chat(var: &str, id: &str, scope: &str, advice: &str) -> Result<(), ErrorData> {
    if in_question_chat(var, id) {
        Ok(())
    } else {
        Err(ErrorData::invalid_request(
            format!(
                "Only the question chat on the home page can change ClickUp, and only {scope}. {advice}"
            ),
            None,
        ))
    }
}

/// check_question_chat for a comment: the task is in QUESTION_CHAT_ENV (the
/// home page asked about it today and it is open), or in
/// QUESTION_REOPEN_ENV (this chat closed it today: the user can still say
/// why, one message after the close).
fn check_comment_chat(id: &str) -> Result<(), ErrorData> {
    if in_question_chat(aifred_core::assistant::QUESTION_REOPEN_ENV, id) {
        return Ok(());
    }
    check_question_chat(
        aifred_core::assistant::QUESTION_CHAT_ENV,
        id,
        "the tasks that the home page asked about today",
        "Tell the user to add the comment in ClickUp.",
    )
}

/// The words of 3 or more letters or digits in `s`, in lower case.
fn words(s: &str) -> Vec<String> {
    s.to_lowercase()
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| w.chars().count() >= 3)
        .map(String::from)
        .collect()
}

/// Nothing when `text` is in the user's own words `said` (the messages of
/// the user in this chat, QUESTION_SAID_ENV), else why not. The check is
/// a heuristic: at least 2 of 3 words of the comment are in the user's
/// words, where a word matches on its first 3 letters ("move" and
/// "moving"). Text from other people (a task description) has mostly
/// other words, so the AI cannot send it to the team as a comment. With
/// no user words at all, no comment passes.
fn in_users_words(text: &str, said: &str) -> Result<(), String> {
    let said = words(said);
    if said.is_empty() {
        return Err("The words of the user in this chat are not known, so no comment can go to ClickUp. Tell the user to add the comment in ClickUp.".into());
    }
    let stem = |w: &str| w.chars().take(3).collect::<String>();
    let stems: std::collections::HashSet<String> = said.iter().map(|w| stem(w)).collect();
    let all = words(text);
    let missing: Vec<&str> = all
        .iter()
        .filter(|w| !stems.contains(&stem(w)))
        .map(String::as_str)
        .collect();
    if missing.len() * 3 > all.len() {
        return Err(format!(
            "The comment must be in the user's own words from this chat, and these words are not there: {}. Write what the user wrote, or tell the user to add the comment in ClickUp.",
            missing.join(", ")
        ));
    }
    Ok(())
}

/// The comment that delete_task_comment deletes, out of the comments
/// `wrote` of this chat on the task today (newest first): the one with the
/// id `wanted`, or the only one. With another id, or with no id and more
/// than one comment, the error lists the comments with their text, so the
/// AI can ask the user which one.
fn pick_comment<'a>(
    wrote: &'a [aifred_core::clickup_list::Comment],
    wanted: Option<&str>,
) -> Result<&'a aifred_core::clickup_list::Comment, ErrorData> {
    let listed = || {
        wrote
            .iter()
            .map(|c| format!("\n- {}: {:?}", c.id, cut(&c.text, 200)))
            .collect::<String>()
    };
    match wanted.map(str::trim) {
        Some(wanted) => wrote.iter().find(|c| c.id == wanted).ok_or_else(|| {
            ErrorData::invalid_params(
                format!(
                    "This chat wrote no comment {wanted:?} on this task today. Its comments on the task today, newest first:{}",
                    listed()
                ),
                None,
            )
        }),
        None if wrote.len() == 1 => Ok(&wrote[0]),
        None => Err(ErrorData::invalid_params(
            format!(
                "This chat wrote {} comments on this task today. Give the comment_id of the one to delete, newest first:{}",
                wrote.len(),
                listed()
            ),
            None,
        )),
    }
}

/// The task id in `id`. The overview and the chat show tasks as
/// "clickup:<id>": the AI can give the key in place of the id.
fn task_id(id: &str) -> &str {
    let id = id.trim();
    id.strip_prefix("clickup:").unwrap_or(id).trim()
}

fn fail(e: anyhow::Error) -> ErrorData {
    ErrorData::internal_error(format!("{e:#}"), None)
}

fn ok_json(v: &Value) -> Result<CallToolResult, ErrorData> {
    Ok(CallToolResult::success(vec![ContentBlock::text(
        v.to_string(),
    )]))
}

impl ClickupTools {
    fn token(&self) -> Result<String, ErrorData> {
        clickup::token(&self.dir).ok_or_else(|| {
            ErrorData::invalid_request(
                "ClickUp is not connected. The user can add a ClickUp token on the AIfred Settings page.",
                None,
            )
        })
    }

    fn client(&self) -> Result<Client, ErrorData> {
        self.token().map(|t| Client::new(&t))
    }

    /// The saved list while it is new enough, or else a new list from ClickUp.
    async fn list(&self) -> Result<ClickupList, ErrorData> {
        let token = self.token()?;
        let account = clickup::account_key(&token);
        let store = ClickupStore::new(&self.dir);
        // A broken saved file is not an error here: ask ClickUp.
        if let Ok(Some(list)) =
            store.cached(&account, settings::tool_max_age_secs(&self.dir), now())
        {
            return Ok(list);
        }
        let fetched = Client::new(&token).my_tasks().await.map_err(fail)?;
        match store.save(&account, fetched.clone(), now()) {
            Ok(list) => Ok(list),
            Err(e) => {
                // The list is still good. Only the next call cannot use it.
                eprintln!("aifred-clickup-mcp: cannot save the ClickUp list: {e:#}");
                let mut items = fetched.items;
                items.sort_by_key(|i| std::cmp::Reverse(i.updated));
                Ok(ClickupList {
                    items,
                    fetched_at: now(),
                    warning: clickup::cut_warning(&fetched.cut),
                })
            }
        }
    }
}

#[tool_router]
impl ClickupTools {
    #[tool(
        description = "List the user's open ClickUp tasks (assigned to them, in all workspaces), newest change first. fetched_at is when the list came from ClickUp (Unix seconds). A note says when the list is not complete. `aifred_status` is AIfred's own status: use it, not the ClickUp `status` name, which can mean another thing in each list. A task with aifred_status Unfinished or Cancelled is closed in AIfred: it is not open work."
    )]
    async fn my_tasks(
        &self,
        Parameters(a): Parameters<ListArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let list = self.list().await?;
        let mut items = list.items;
        items.truncate(a.limit.unwrap_or(50).clamp(1, 200) as usize);
        // AIfred's own status of each task (aifred-core status.rs).
        let statuses = aifred_core::status::all(&self.dir).unwrap_or_default();
        let items: Vec<serde_json::Value> = items
            .into_iter()
            .map(|i| {
                let key = format!("clickup:{}", i.id);
                let mut v = json!(i);
                if let Some(st) = statuses.get(&key) {
                    v["aifred_status"] = aifred_core::status::label(&st.status).into();
                }
                v
            })
            .collect();
        let mut out = json!({ "tasks": items, "fetched_at": list.fetched_at });
        if let Some(note) = list.warning {
            out["note"] = json!(note);
        }
        ok_json(&out)
    }

    #[tool(
        description = "Search the user's ClickUp tasks by words in the name, the workspace and list, and the description, best match first. It searches the list that AIfred saved, and downloads a new list first when it is too old. Each result says if it is still in the user's open list (in_list) and when AIfred last saw it (last_seen, Unix seconds). A result can have `related`: the items linked to it (parent, subtask, same mission, closes, closed by), each with its key, title, place, and why. Read them to see the work around a result."
    )]
    async fn search_my_tasks(
        &self,
        Parameters(a): Parameters<SearchArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let account = clickup::account_key(&self.token()?);
        // list() downloads a new list when the saved one is too old.
        let note = match self.list().await {
            Ok(list) => list.warning,
            Err(e) => Some(format!(
                "Cannot reach ClickUp, so the results come from the saved list. {}",
                e.message
            )),
        };
        let hits = ClickupStore::new(&self.dir)
            .search(&account, &a.query, a.include_old.unwrap_or(false), a.limit)
            .map_err(fail)?;
        let tasks: Vec<Value> = hits.into_iter().map(|h| h.into_json()).collect();
        let mut out = json!({ "tasks": tasks });
        if let Some(note) = note {
            out["note"] = json!(note);
        }
        ok_json(&out)
    }

    #[tool(
        description = "Get one ClickUp task: name, status, description (shortened), custom fields that have a value (for example the project or the client), assignees, dates, and the last comments."
    )]
    async fn task_details(
        &self,
        Parameters(a): Parameters<DetailArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        clickup::check_task_id(&a.id)
            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        let client = self.client()?;
        let (task, comments) = client.task_with_comments(&a.id).await.map_err(fail)?;
        // The dates of custom fields are days in the account's timezone.
        let zone = match client.zone().await {
            Ok(z) => Some(z),
            Err(e) => {
                eprintln!("aifred-clickup-mcp: cannot read the ClickUp timezone: {e:#}");
                None
            }
        };
        let description =
            clickup::description_text(task["text_content"].as_str(), task["description"].as_str());
        let fields = clickup::field_values(
            &task["custom_fields"],
            zone.unwrap_or(clickup::DEFAULT_ZONE),
        );
        // Save the full description, so search_my_tasks can find words in it.
        // It is the same text as the list download saves (details_text: the
        // custom fields, then the description), so the summary of the task
        // does not have to be made again. Without the account timezone a date
        // can be one day off, and that changes the saved text, so it is not saved.
        // The id from ClickUp, because `a.id` can be the team's custom id.
        if zone.is_some()
            && let Some(id) = task["id"].as_str()
            && let Err(e) = ClickupStore::new(&self.dir)
                .save_details(id, &clickup::details_text(&description, &fields))
        {
            eprintln!("aifred-clickup-mcp: cannot save the description: {e:#}");
        }
        ok_json(&shorten(&task, &comments, &description, &fields))
    }

    #[tool(
        description = "Mark one ClickUp task done: it moves to the closed status of its list. Use it only when the user's own words say that the task is finished. The answer has `before`, the status from before. reopen_task with the same id is the undo."
    )]
    async fn mark_task_done(
        &self,
        Parameters(a): Parameters<TaskArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = task_id(&a.id);
        check_question_chat(
            aifred_core::assistant::QUESTION_CHAT_ENV,
            id,
            "the tasks that the home page asked about today and that are still open in ClickUp",
            "Tell the user to mark the task done in AIfred or in ClickUp.",
        )?;
        clickup::check_task_id(id).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        let before = self.client()?.mark_done(id).await.map_err(fail)?;
        // The undo (reopen_task) needs this note, with the chat that closed
        // the task. ClickUp has the change, so a failed note is not an
        // error: the user can open the task again in ClickUp.
        let chat = std::env::var(aifred_core::assistant::QUESTION_ITEM_ENV).unwrap_or_default();
        if let Err(e) = ClickupStore::new(&self.dir).note_closed(id, &before, &chat, now()) {
            eprintln!("aifred-clickup-mcp: cannot note the close for the undo: {e:#}");
        }
        // ClickUp has the change. The saved list shows the task until the
        // next download when this fails, so it is not an error.
        if let Err(e) = ClickupStore::new(&self.dir).remove(id, now()) {
            eprintln!("aifred-clickup-mcp: cannot update the saved ClickUp list: {e:#}");
        }
        ok_json(&json!({ "id": id, "done": true, "before": before }))
    }

    #[tool(
        description = "Undo mark_task_done: put a ClickUp task that you marked done back to its status from before. Use it only when the user asks for the undo."
    )]
    async fn reopen_task(
        &self,
        Parameters(a): Parameters<TaskArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = task_id(&a.id);
        check_question_chat(
            aifred_core::assistant::QUESTION_REOPEN_ENV,
            id,
            "the tasks that this chat marked done today",
            "Tell the user to open the task again in ClickUp.",
        )?;
        clickup::check_task_id(id).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        // Only the status that AIfred saved when it closed the task. So the
        // AI cannot give another status, for example a closed one.
        let store = ClickupStore::new(&self.dir);
        let Some(status) = store.closed_status(id).map_err(fail)? else {
            return Err(ErrorData::invalid_request(
                "AIfred did not mark this task done, so it cannot undo it. Tell the user to change the task in ClickUp.",
                None,
            ));
        };
        self.client()?.set_status(id, &status).await.map_err(fail)?;
        if let Err(e) = store.forget_closed(id) {
            eprintln!("aifred-clickup-mcp: cannot forget the close: {e:#}");
        }
        // The undo of remove (mark_task_done): the task is back in the
        // saved list at once, so the question chat can use it again in the
        // next message. ClickUp has the change, so a failure is not an
        // error: the next download puts the task back.
        if let Err(e) = store.restore(id) {
            eprintln!("aifred-clickup-mcp: cannot put the task back in the saved list: {e:#}");
        }
        ok_json(&json!({ "id": id, "status": status }))
    }

    #[tool(
        description = "Write one comment on a ClickUp task, as the user. Use it only when the user asks for a note or an explanation in ClickUp, and write only what the user's own words in this chat say, short: the tool refuses other words. The team sees it in ClickUp. Call it once: when the answer is an error that says the comment may be saved, do not call it again; tell the user to look at the task in ClickUp. delete_task_comment with the task id and the `comment_id` of the answer is the undo."
    )]
    async fn add_task_comment(
        &self,
        Parameters(a): Parameters<CommentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = task_id(&a.id);
        check_comment_chat(id)?;
        clickup::check_task_id(id).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        let text = clickup::check_comment_text(&a.text)
            .map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        // Only the user's own words go to the team. The chat note and the
        // item text come from other people, and the prompt alone does not
        // keep them out.
        let said = std::env::var(aifred_core::assistant::QUESTION_SAID_ENV).unwrap_or_default();
        in_users_words(text, &said).map_err(|e| ErrorData::invalid_params(e, None))?;
        let made = match self.client()?.add_comment(id, text).await {
            Ok(m) => m,
            // The call left, but no reply came: ClickUp may have the
            // comment. A second call could write it two times.
            Err(e) if e.downcast_ref::<clickup::NoReply>().is_some() => {
                return Err(ErrorData::internal_error(
                    format!(
                        "No reply from ClickUp, so the comment may be saved. Do not write it again. Tell the user to look at the task in ClickUp. ({e:#})"
                    ),
                    None,
                ));
            }
            Err(e) => return Err(fail(e)),
        };
        let mut out = json!({ "id": id, "text": text });
        // The undo (delete_task_comment) needs this note, with the chat
        // that wrote the comment: the chat does not keep the comment id
        // from one message to the next. ClickUp has the comment, so a
        // failed note is not an error: the user can delete it in ClickUp.
        // The undo gives the id to ClickUp in the address of the call, so
        // an id of another form is not noted: no error, and no undo.
        let chat = std::env::var(aifred_core::assistant::QUESTION_ITEM_ENV).unwrap_or_default();
        let noted = match &made {
            Some(comment_id) => clickup::check_comment_id(comment_id).and_then(|()| {
                ClickupStore::new(&self.dir).note_comment(comment_id, id, &chat, text, now())
            }),
            None => Err(anyhow::anyhow!("ClickUp sent no comment id")),
        };
        match (made, noted) {
            (Some(comment_id), Ok(())) => out["comment_id"] = json!(comment_id),
            (_, Err(e)) => {
                eprintln!("aifred-clickup-mcp: cannot note the comment for the undo: {e:#}");
                out["note"] = json!(
                    "ClickUp saved the comment, but AIfred cannot delete it from the chat. The user can delete it in ClickUp."
                );
            }
            (None, Ok(())) => unreachable!("no id is not noted"),
        }
        ok_json(&out)
    }

    #[tool(
        description = "Undo add_task_comment: delete one comment that this chat wrote today on a ClickUp task. Use it only when the user asks for the undo. Give `comment_id` from the add_task_comment answer; without it, the one comment of this chat on the task today goes, and with more than one the answer lists them with their text. The answer has the `text` of the deleted comment, `deleted` true when ClickUp deleted it now, and `gone` true when it was not there any more."
    )]
    async fn delete_task_comment(
        &self,
        Parameters(a): Parameters<DeleteCommentArgs>,
    ) -> Result<CallToolResult, ErrorData> {
        let id = task_id(&a.id);
        clickup::check_task_id(id).map_err(|e| ErrorData::invalid_params(e.to_string(), None))?;
        // Only a comment that this question chat wrote today. Another chat
        // gets item text from other people, and that text must not delete
        // comments. A bare comment id from the AI is not trusted: it must
        // be in the notes of this chat.
        let chat = std::env::var(aifred_core::assistant::QUESTION_ITEM_ENV).unwrap_or_default();
        let store = ClickupStore::new(&self.dir);
        let wrote = if chat.trim().is_empty() {
            vec![]
        } else {
            let since = aifred_core::calendar::today_start().map_err(fail)?;
            store.comments_since(id, &chat, since).map_err(fail)?
        };
        if wrote.is_empty() {
            return Err(ErrorData::invalid_request(
                "This chat wrote no comment on this task today, so it cannot delete one. Tell the user to delete the comment in ClickUp.",
                None,
            ));
        }
        let comment = pick_comment(&wrote, a.comment_id.as_deref())?;
        // A comment that is already gone (the user deleted it in ClickUp)
        // is forgotten too: else every later undo on the task would find
        // it again.
        let deleted = self
            .client()?
            .delete_comment(&comment.id)
            .await
            .map_err(fail)?;
        if let Err(e) = store.forget_comment(&comment.id) {
            eprintln!("aifred-clickup-mcp: cannot forget the comment: {e:#}");
        }
        let mut out = json!({
            "id": id,
            "comment_id": comment.id,
            "text": comment.text,
            "deleted": deleted,
        });
        if !deleted {
            out["gone"] = json!(true);
            out["note"] = json!(
                "ClickUp has no such comment for this account any more: the user deleted it, or the token is of another account now."
            );
        }
        ok_json(&out)
    }
}

#[tool_handler]
impl ServerHandler for ClickupTools {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
    }
}

/// Keeps the tool output small: only the useful task fields, a shortened
/// description, the custom fields that have a value, and the last comments
/// as plain text. `description` and `fields` come from task_details, so they are made once.
fn shorten(
    task: &Value,
    comments: &Value,
    description: &str,
    fields: &[clickup::FieldValue],
) -> Value {
    let names = |v: &Value| -> Vec<Value> {
        v.as_array()
            .map(|a| a.iter().map(|u| u["username"].clone()).collect())
            .unwrap_or_default()
    };
    let mut last: Vec<Value> = comments["comments"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|c| json!({ "by": c["user"]["username"], "date": c["date"], "text": cut(c["comment_text"].as_str().unwrap_or_default(), MAX_TEXT_CHARS) }))
                .collect()
        })
        .unwrap_or_default();
    // ClickUp sends the newest comment first.
    last.truncate(MAX_COMMENTS);
    // A list, not a map: two fields can have the same name.
    let fields: Vec<Value> = fields
        .iter()
        .map(|f| json!({"name": f.name, "value": f.value}))
        .collect();
    json!({
        "id": task["id"],
        "custom_id": task["custom_id"],
        "name": task["name"],
        "url": task["url"],
        "status": task["status"]["status"],
        "list": task["list"]["name"],
        "priority": task["priority"]["priority"],
        "due_date": task["due_date"],
        "date_updated": task["date_updated"],
        "assignees": names(&task["assignees"]),
        "tags": task["tags"].as_array().map(|t| t.iter().map(|x| x["name"].clone()).collect::<Vec<_>>()),
        "description": cut(description, MAX_TEXT_CHARS),
        "custom_fields": fields,
        "last_comments": last,
    })
}

/// Serves these tools over stdio until Claude Code closes the connection.
pub async fn serve() -> anyhow::Result<()> {
    let server = ClickupTools {
        dir: default_data_dir()?,
    }
    .serve(stdio())
    .await?;
    server.waiting().await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shorten_keeps_the_useful_fields_and_cuts_long_text() {
        let task = json!({
            "id": "86abc", "custom_id": "DEV-42", "name": "Write notes", "url": "u",
            "status": {"status": "in progress"}, "list": {"name": "Sprint"},
            "priority": {"priority": "high"}, "assignees": [{"username": "okky"}],
            "text_content": "x".repeat(MAX_TEXT_CHARS + 10),
            "custom_fields": [
                {"name": "Project", "type": "drop_down", "value": 1,
                 "type_config": {"options": [{"id": "a", "name": "Wholesale", "orderindex": 0}, {"id": "b", "name": "Coupons", "orderindex": 1}]}},
                {"name": "Score", "type": "formula", "value": null}
            ]
        });
        let comments = json!({"comments": (0..8).map(|i| json!({"comment_text": format!("c{i}"), "user": {"username": "sam"}, "date": "1"})).collect::<Vec<_>>()});
        let description = clickup::description_text(task["text_content"].as_str(), None);
        let fields = clickup::field_values(&task["custom_fields"], clickup::DEFAULT_ZONE);
        let s = shorten(&task, &comments, &description, &fields);
        assert_eq!(s["status"], "in progress");
        assert_eq!(s["assignees"], json!(["okky"]));
        assert!(
            s["description"]
                .as_str()
                .unwrap()
                .ends_with("[... shortened]")
        );
        assert_eq!(s["last_comments"].as_array().unwrap().len(), MAX_COMMENTS);
        assert_eq!(
            s["last_comments"][0]["text"], "c0",
            "the newest comment stays"
        );
        assert_eq!(
            s["custom_fields"],
            json!([{"name": "Project", "value": "Coupons"}]),
            "the dropdown shows the option name, and a field without a value is left out"
        );
    }

    #[test]
    fn task_id_takes_the_id_out_of_an_item_key() {
        assert_eq!(task_id("clickup:86abc"), "86abc");
        assert_eq!(task_id(" 86abc "), "86abc");
        assert_eq!(task_id("DEV-42"), "DEV-42");
    }

    #[tokio::test]
    async fn tools_without_a_token_say_how_to_connect() {
        let dir = std::env::temp_dir().join(format!("aifred-clickup-mcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let t = ClickupTools { dir: dir.clone() };
        // Only when the developer has no token in the environment.
        if std::env::var_os("AIFRED_CLICKUP_TOKEN").is_none() {
            let e = t
                .my_tasks(Parameters(ListArgs { limit: None }))
                .await
                .unwrap_err();
            assert!(e.message.contains("Settings page"), "{e:?}");
        }
        let e = t
            .task_details(Parameters(DetailArgs {
                id: "../user".into(),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("bad ClickUp task id"), "{e:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The comment tools without the question chat, or for a comment that
    /// another chat wrote: an error before any ClickUp call. The fake
    /// token would fail if ClickUp were asked.
    #[tokio::test]
    async fn comments_need_the_question_chat_and_only_its_own_comments_go() {
        let dir =
            std::env::temp_dir().join(format!("aifred-clickup-mcp-comment-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        clickup::save_token(&dir, "pk_test").unwrap();
        let t = ClickupTools { dir: dir.clone() };
        // The test process has no question chat env: no task is allowed.
        let e = t
            .add_task_comment(Parameters(CommentArgs {
                id: "clickup:86abc".into(),
                text: "Stopped for now.".into(),
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("question chat"), "{e:?}");
        // A comment that another chat wrote cannot be deleted here: this
        // process is no question chat (no QUESTION_ITEM_ENV).
        ClickupStore::new(&dir)
            .note_comment("458", "86abc", "clickup:86abc", "Stopped.", now())
            .unwrap();
        let e = t
            .delete_task_comment(Parameters(DeleteCommentArgs {
                id: "clickup:86abc".into(),
                comment_id: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("wrote no comment"), "{e:?}");
        let e = t
            .delete_task_comment(Parameters(DeleteCommentArgs {
                id: "../x".into(),
                comment_id: None,
            }))
            .await
            .unwrap_err();
        assert!(e.message.contains("bad ClickUp task id"), "{e:?}");
        // The close and its undo say what the user can do instead: a close
        // in AIfred or ClickUp, or a reopen in ClickUp.
        let e = t
            .mark_task_done(Parameters(TaskArgs { id: "86abc".into() }))
            .await
            .unwrap_err();
        assert!(e.message.contains("still open in ClickUp"), "{e:?}");
        assert!(e.message.contains("mark the task done"), "{e:?}");
        let e = t
            .reopen_task(Parameters(TaskArgs { id: "86abc".into() }))
            .await
            .unwrap_err();
        assert!(e.message.contains("marked done today"), "{e:?}");
        assert!(e.message.contains("open the task again"), "{e:?}");
        assert!(!e.message.contains("still open"), "{e:?}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The comment check is a heuristic: most words of the comment are
    /// words of the user, matched on their first 3 letters.
    #[test]
    fn a_comment_is_in_the_users_own_words_or_it_does_not_go() {
        let said = "Tell them I stopped because the sites are moving.\nUndo that.";
        assert_eq!(in_users_words("Stopped: the sites move.", said), Ok(()));
        assert_eq!(
            in_users_words("STOPPED, the sites are moving", said),
            Ok(())
        );
        // Short words and numbers do not count.
        assert_eq!(in_users_words("OK 42", said), Ok(()));
        // Text from a task description, or a memory fact: most words are
        // not the user's.
        let e = in_users_words("The user lives in Berlin and uses Vim.", said).unwrap_err();
        assert!(e.contains("berlin") && e.contains("vim"), "{e}");
        assert!(e.contains("own words"), "{e}");
        // One word of three can differ ("now" is not in the user's words).
        assert_eq!(in_users_words("Stopped the sites now.", said), Ok(()));
        // No user words at all: nothing goes.
        let e = in_users_words("Stopped.", "").unwrap_err();
        assert!(e.contains("not known"), "{e}");
    }

    /// The undo deletes the one comment of the chat on the task, or the
    /// one that the AI names from the notes; else it lists them.
    #[test]
    fn the_undo_picks_one_comment_or_lists_them() {
        use aifred_core::clickup_list::Comment;
        let one = vec![Comment {
            id: "9".into(),
            text: "Nine.".into(),
        }];
        assert_eq!(pick_comment(&one, None).unwrap().id, "9");
        assert_eq!(pick_comment(&one, Some(" 9 ")).unwrap().id, "9");
        let two = vec![
            Comment {
                id: "9".into(),
                text: "Nine.".into(),
            },
            Comment {
                id: "8".into(),
                text: "Eight.".into(),
            },
        ];
        let e = pick_comment(&two, None).unwrap_err();
        assert!(e.message.contains("2 comments"), "{e:?}");
        assert!(
            e.message.contains("\n- 9: \"Nine.\"\n- 8: \"Eight.\""),
            "{e:?}"
        );
        assert_eq!(pick_comment(&two, Some("8")).unwrap().text, "Eight.");
        // An id that is not in the notes of this chat: not trusted.
        let e = pick_comment(&two, Some("7")).unwrap_err();
        assert!(e.message.contains("no comment \"7\""), "{e:?}");
        assert!(e.message.contains("- 8: \"Eight.\""), "{e:?}");
    }

    #[tokio::test]
    async fn my_tasks_uses_the_saved_list_while_it_is_new() {
        let dir =
            std::env::temp_dir().join(format!("aifred-clickup-mcp-saved-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        clickup::save_token(&dir, "pk_test").unwrap();
        // The token in use: the file, or AIFRED_CLICKUP_TOKEN when it is set.
        let account = clickup::account_key(&clickup::token(&dir).unwrap());
        let item: clickup::ClickupItem = serde_json::from_value(json!({
            "id": "86abc", "title": "Write notes", "url": "u", "status": "to do",
            "list": "Sprint", "workspace": "Rymera", "updated": 1
        }))
        .unwrap();
        let fetched = clickup::MyTasks {
            items: vec![item],
            cut: vec!["Big".into()],
        };
        ClickupStore::new(&dir)
            .save(&account, fetched, now() - 60)
            .unwrap();
        // No ClickUp call: the fake token would fail if ClickUp were asked.
        let t = ClickupTools { dir: dir.clone() };
        let r = t
            .my_tasks(Parameters(ListArgs { limit: None }))
            .await
            .unwrap();
        let text = serde_json::to_value(&r.content[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["tasks"][0]["title"], "Write notes");
        assert!(
            v["note"].as_str().unwrap().contains("Big"),
            "the chat hears that the list is cut: {v}"
        );

        // The search uses the same saved list, also without a ClickUp call.
        let args = Parameters(SearchArgs {
            query: "notes sprint".into(),
            include_old: None,
            limit: None,
        });
        let r = t.search_my_tasks(args).await.unwrap();
        let text = serde_json::to_value(&r.content[0]).unwrap()["text"]
            .as_str()
            .unwrap()
            .to_string();
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["tasks"][0]["title"], "Write notes");
        assert_eq!(v["tasks"][0]["in_list"], true);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
