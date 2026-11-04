//! User settings, stored in settings.json in the data folder.

use std::path::{Path, PathBuf};

use anyhow::bail;
use serde::{Deserialize, Serialize};

use crate::files;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// How often the GitHub and ClickUp lists load again, in minutes.
    /// 0 means only when the user clicks reload. The chat's ClickUp tool
    /// uses the saved list for the same time.
    /// Older settings files call it github_refresh_minutes.
    #[serde(alias = "github_refresh_minutes")]
    pub refresh_minutes: u32,
}

impl Default for Settings {
    fn default() -> Self {
        Self { refresh_minutes: 15 }
    }
}

/// The longest refresh time we accept: one day.
const MAX_MINUTES: u32 = 24 * 60;

fn path(dir: &Path) -> PathBuf {
    dir.join("settings.json")
}

pub fn load(dir: &Path) -> anyhow::Result<Settings> {
    files::load(&path(dir))
}

pub fn save(dir: &Path, s: &Settings) -> anyhow::Result<()> {
    if s.refresh_minutes > MAX_MINUTES {
        bail!("the refresh time must be {MAX_MINUTES} minutes or less");
    }
    // Replace, not update: a broken settings file must not block the fix.
    files::replace(&path(dir), s)
}

/// The settings, or the defaults if the file is broken. For places that
/// must keep working, like the GitHub list.
pub fn load_or_default(dir: &Path) -> Settings {
    load(dir).unwrap_or_default()
}

/// How old, in seconds, a saved GitHub or ClickUp list can be and still be used.
pub fn max_age_secs(dir: &Path) -> u64 {
    // A broken settings file must not hide the list: use the defaults.
    let minutes = load_or_default(dir).refresh_minutes;
    // 0 means "only when I click reload": the saved list never gets too old.
    if minutes == 0 { u64::MAX } else { u64::from(minutes) * 60 }
}

/// The oldest saved list that the chat tools use when the user reloads only
/// by hand (refresh time 0). The chat must not plan a day from a week-old list.
pub const TOOL_MANUAL_MAX_AGE_SECS: u64 = 15 * 60;

/// How old, in seconds, a saved list can be for the GitHub and ClickUp chat
/// tools. The same as max_age_secs, but never more than
/// TOOL_MANUAL_MAX_AGE_SECS when the user reloads only by hand.
pub fn tool_max_age_secs(dir: &Path) -> u64 {
    match max_age_secs(dir) {
        u64::MAX => TOOL_MANUAL_MAX_AGE_SECS,
        secs => secs,
    }
}

/// The models that the chat box offers. The `claude` program knows these
/// short names and always uses the newest model of each family.
pub const CHAT_MODELS: &[&str] = &["fable", "opus", "sonnet", "haiku"];

/// The model that the user picked last in the chat box, or None for the
/// default (AIFRED_MODEL, or the Claude Code default). It is in the database
/// and not in settings.json: the Settings page saves settings.json as a whole,
/// and an open Settings page must not put back an older model.
pub fn chat_model(dir: &Path) -> anyhow::Result<Option<String>> {
    crate::db::read(dir, |c| crate::db::meta(c, "chat_model"))
}

/// Saves the model for the next messages. None goes back to the default.
pub fn set_chat_model(dir: &Path, model: Option<&str>) -> anyhow::Result<()> {
    if let Some(m) = model
        && !CHAT_MODELS.contains(&m)
    {
        bail!("unknown model \"{m}\". Use one of: {}", CHAT_MODELS.join(", "));
    }
    crate::db::write(dir, |tx| {
        match model {
            Some(m) => tx.execute("INSERT INTO meta (key, value) VALUES ('chat_model', ?1) ON CONFLICT (key) DO UPDATE SET value = ?1", [m])?,
            None => tx.execute("DELETE FROM meta WHERE key = 'chat_model'", [])?,
        };
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("aifred-settings-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn missing_file_gives_defaults_and_save_round_trips() {
        let d = temp("round");
        assert_eq!(load(&d).unwrap().refresh_minutes, 15);
        save(&d, &Settings { refresh_minutes: 0 }).unwrap();
        assert_eq!(load(&d).unwrap().refresh_minutes, 0);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn an_older_settings_file_with_the_github_name_still_loads() {
        let d = temp("old-name");
        std::fs::write(d.join("settings.json"), r#"{"github_refresh_minutes":45}"#).unwrap();
        assert_eq!(load(&d).unwrap().refresh_minutes, 45);
        assert_eq!(max_age_secs(&d), 45 * 60);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_chat_tools_never_use_a_list_older_than_15_minutes_in_manual_mode() {
        let d = temp("tool-age");
        save(&d, &Settings { refresh_minutes: 0 }).unwrap();
        assert_eq!(max_age_secs(&d), u64::MAX, "the window keeps its list");
        assert_eq!(tool_max_age_secs(&d), 15 * 60);
        save(&d, &Settings { refresh_minutes: 45 }).unwrap();
        assert_eq!(tool_max_age_secs(&d), 45 * 60);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn the_chat_model_is_kept_and_an_unknown_model_is_refused() {
        let d = temp("chat-model");
        assert_eq!(chat_model(&d).unwrap(), None, "the default at first");
        set_chat_model(&d, Some("sonnet")).unwrap();
        assert_eq!(chat_model(&d).unwrap().as_deref(), Some("sonnet"));
        assert!(set_chat_model(&d, Some("gpt-4")).is_err());
        assert_eq!(chat_model(&d).unwrap().as_deref(), Some("sonnet"), "a bad name changes nothing");
        // A save on the Settings page does not touch the model.
        save(&d, &Settings { refresh_minutes: 30 }).unwrap();
        assert_eq!(chat_model(&d).unwrap().as_deref(), Some("sonnet"));
        set_chat_model(&d, None).unwrap();
        assert_eq!(chat_model(&d).unwrap(), None);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn rejects_a_refresh_time_over_one_day() {
        let d = temp("max");
        assert!(save(&d, &Settings { refresh_minutes: 24 * 60 + 1 }).is_err());
        assert_eq!(load(&d).unwrap(), Settings::default(), "nothing was saved");
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn save_repairs_a_broken_file() {
        let d = temp("broken");
        std::fs::write(d.join("settings.json"), "{ not json").unwrap();
        assert!(load(&d).is_err());
        assert_eq!(load_or_default(&d), Settings::default());
        save(&d, &Settings { refresh_minutes: 30 }).unwrap();
        assert_eq!(load(&d).unwrap().refresh_minutes, 30);
        std::fs::remove_dir_all(&d).unwrap();
    }

    #[test]
    fn unknown_and_missing_fields_are_fine() {
        let d = temp("fields");
        std::fs::write(d.join("settings.json"), r#"{"later_setting": true}"#).unwrap();
        assert_eq!(load(&d).unwrap(), Settings::default());
        std::fs::remove_dir_all(&d).unwrap();
    }
}
