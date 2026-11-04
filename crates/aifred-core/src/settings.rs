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
