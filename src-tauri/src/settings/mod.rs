//! User settings, stored as TOML at `%APPDATA%\desktop-assistant\settings.toml`.
//!
//! Every field has a default: the file on disk is layered over the defaults
//! before parsing, so a partial or older file still loads. Bump
//! `CURRENT_VERSION` and add a step to `migrate` when a change needs more than
//! a new defaulted field (renames, moved sections, changed meaning).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::ai::{Approvals, BillingMode, ProviderId};

pub const CURRENT_VERSION: u32 = 1;

#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not read or write settings at {path}: {source}")]
    Io { path: PathBuf, source: io::Error },
    #[error("settings file is not valid TOML: {0}")]
    Parse(#[from] toml::de::Error),
    #[error("could not serialize settings: {0}")]
    Serialize(#[from] toml::ser::Error),
    #[error("settings have an unexpected shape: {0}")]
    Shape(String),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Settings {
    pub version: u32,
    pub hotkeys: HotkeySettings,
    pub ai: AiSettings,
    pub search: SearchSettings,
    pub general: GeneralSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            hotkeys: HotkeySettings::default(),
            ai: AiSettings::default(),
            search: SearchSettings::default(),
            general: GeneralSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct HotkeySettings {
    /// Global shortcut that toggles the launcher, e.g. "Ctrl+Space".
    pub toggle: String,
    /// Tapping the Windows key on its own opens the launcher.
    pub win_key_override: bool,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Space".into(),
            win_key_override: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AiSettings {
    /// Gets any query that doesn't match an app, file or command.
    pub default_provider: ProviderId,
    pub claude: ClaudeSettings,
    pub chatgpt: ProviderSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_provider: ProviderId::Claude,
            claude: ClaudeSettings::default(),
            chatgpt: ProviderSettings::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ClaudeSettings {
    pub billing: BillingMode,
    /// Passed to `claude --model`: an alias ("sonnet", "haiku") or a full id.
    pub model: String,
    /// "ask": Approve / Deny in the launcher before Claude changes anything.
    /// "auto": Claude Code's auto mode decides, asking only when unsure.
    pub approvals: Approvals,
    /// Full path to `claude` (or its `cli.js`). Empty: find it automatically.
    pub path: String,
    /// Tell Claude which window you were in when you opened the launcher,
    /// and for File Explorer the folder and selected files. Explorer's
    /// folder also becomes Claude's working folder.
    pub desktop_context: bool,
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        Self {
            billing: BillingMode::Subscription,
            model: "sonnet".into(),
            approvals: Approvals::Ask,
            path: String::new(),
            desktop_context: true,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ProviderSettings {
    pub billing: BillingMode,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct SearchSettings {
    /// Folders whose contents never show up in file results: a folder name
    /// ("node_modules") or a partial path ("AppData\Local\Temp").
    pub exclusions: Vec<String>,
    /// Folders the built-in file index covers when Everything isn't running.
    /// `~` is your user folder.
    pub index_folders: Vec<String>,
    pub max_results: u32,
}

impl Default for SearchSettings {
    fn default() -> Self {
        Self {
            exclusions: [
                "node_modules",
                ".git",
                "$Recycle.Bin",
                r"AppData\Local\Temp",
            ]
            .map(String::from)
            .to_vec(),
            index_folders: vec!["~".into()],
            max_results: 8,
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct GeneralSettings {
    pub launch_at_login: bool,
}

/// Loads settings, creating the file with defaults if it doesn't exist.
/// A file that fails to parse is moved aside to `settings.toml.bad` and
/// replaced with defaults, so a typo never stops the launcher from starting.
pub fn load_or_create(path: &Path) -> Result<Settings, SettingsError> {
    let text = match fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            let settings = Settings::default();
            save(path, &settings)?;
            return Ok(settings);
        }
        Err(source) => {
            return Err(SettingsError::Io {
                path: path.to_owned(),
                source,
            })
        }
    };

    match parse(&text) {
        Ok((settings, migrated)) => {
            if migrated {
                save(path, &settings)?;
            }
            Ok(settings)
        }
        Err(e) => {
            let bad = path.with_extension("toml.bad");
            tracing::error!("{e}; moving it to {} and using defaults", bad.display());
            fs::rename(path, &bad).map_err(|source| SettingsError::Io {
                path: bad.clone(),
                source,
            })?;
            let settings = Settings::default();
            save(path, &settings)?;
            Ok(settings)
        }
    }
}

/// Writes settings atomically (temp file, then rename).
pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    let io_err = |source| SettingsError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let text = toml::to_string_pretty(settings)?;
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(io_err)?;
    fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// Parses settings text. Returns `true` alongside the settings if the file
/// was migrated and should be written back.
fn parse(text: &str) -> Result<(Settings, bool), SettingsError> {
    let mut user: toml::Table = toml::from_str(text)?;
    let migrated = migrate(&mut user);
    let mut merged = default_table()?;
    merge(&mut merged, user);
    let settings: Settings = merged.try_into()?;
    Ok((settings, migrated))
}

fn default_table() -> Result<toml::Table, SettingsError> {
    toml::Table::try_from(Settings::default()).map_err(|e| SettingsError::Shape(e.to_string()))
}

/// Recursively overlays `over` onto `base`. Tables merge; other values replace.
fn merge(base: &mut toml::Table, over: toml::Table) {
    for (key, value) in over {
        match (base.get_mut(&key), value) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(key, v);
            }
        }
    }
}

/// Upgrades an older settings table in place. Returns whether anything changed.
fn migrate(table: &mut toml::Table) -> bool {
    let version = table
        .get("version")
        .and_then(|v| v.as_integer())
        .unwrap_or(0);

    if version > i64::from(CURRENT_VERSION) {
        tracing::warn!("settings version {version} is newer than this app ({CURRENT_VERSION})");
        return false;
    }
    if version == i64::from(CURRENT_VERSION) {
        return false;
    }

    // Add steps here as versions are introduced, e.g.
    // if version < 2 { rename a key }
    table.insert("version".into(), i64::from(CURRENT_VERSION).into());
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_defaults_when_missing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cfg").join("settings.toml");
        let s = load_or_create(&path).unwrap();
        assert_eq!(s, Settings::default());
        assert!(path.exists());
    }

    #[test]
    fn round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let mut s = Settings::default();
        s.hotkeys.toggle = "Alt+Space".into();
        s.ai.default_provider = ProviderId::Chatgpt;
        s.ai.claude.billing = BillingMode::ApiKey;
        s.ai.claude.model = "haiku".into();
        s.ai.claude.approvals = Approvals::Auto;
        save(&path, &s).unwrap();
        assert_eq!(load_or_create(&path).unwrap(), s);
    }

    #[test]
    fn partial_file_fills_defaults() {
        let (s, _) = parse("version = 1\n[hotkeys]\ntoggle = \"Alt+Space\"\n").unwrap();
        assert_eq!(s.hotkeys.toggle, "Alt+Space");
        assert!(s.hotkeys.win_key_override);
        assert_eq!(s.search, SearchSettings::default());
    }

    #[test]
    fn unversioned_file_is_migrated() {
        let (s, migrated) = parse("[general]\nlaunch_at_login = true\n").unwrap();
        assert!(migrated);
        assert_eq!(s.version, CURRENT_VERSION);
        assert!(s.general.launch_at_login);
    }

    #[test]
    fn broken_file_is_moved_aside() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        fs::write(&path, "this is = = not toml").unwrap();
        let s = load_or_create(&path).unwrap();
        assert_eq!(s, Settings::default());
        assert!(dir.path().join("settings.toml.bad").exists());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let (s, _) = parse("version = 1\nold_thing = 3\n[hotkeys]\nmystery = true\n").unwrap();
        assert_eq!(s, Settings::default());
    }

    #[test]
    fn wrong_type_is_an_error() {
        assert!(parse("[hotkeys]\nwin_key_override = \"yes\"\n").is_err());
    }

    #[test]
    fn enums_are_snake_case_in_toml() {
        let text = toml::to_string(&Settings::default()).unwrap();
        assert!(text.contains("default_provider = \"claude\""), "{text}");
        assert!(text.contains("billing = \"subscription\""), "{text}");
    }
}
