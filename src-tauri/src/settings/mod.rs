//! User settings, stored as TOML in two files:
//! - `settings.toml` in the settings folder: `%APPDATA%\desktop-assistant`,
//!   or a synced folder (OneDrive) chosen in the menu (`folder.rs`), so
//!   several PCs share it;
//! - `settings.local.toml` in `%LOCALAPPDATA%\desktop-assistant`: this PC's
//!   own values (`layers::MACHINE_KEYS`, plus any key added there by hand).
//!
//! Every field has a default: the files are layered over the defaults
//! (defaults, then the synced file, then the local one) before parsing, so a
//! partial or older file still loads. Bump `CURRENT_VERSION` and add a step
//! to `migrate` when a change needs more than a new defaulted field
//! (renames, moved sections, changed meaning); it runs on each file.
//!
//! - `layers.rs`: the split and merge (pure, tested).
//! - `disk.rs`: reading and writing the files (tested with temp folders).
//! - `folder.rs`: choosing the settings folder (OneDrive), moving there.
//! - `live.rs`: saving from the menu and following changes made on another
//!   PC while running, applied like a change made in the menu.
//! - `commands.rs`: what the settings menu calls about the folder.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use serde::{Deserialize, Serialize};

use crate::ai::{Approvals, BillingMode, ProviderId};
use crate::features;
use crate::integrations::McpSettings;

mod appearance;
pub mod commands;
mod disk;
pub mod folder;
mod layers;
pub mod live;

pub use appearance::{AppearanceSettings, Density, GlassStrength};
pub use layers::MACHINE_KEYS;

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

/// Settings shared between the app and anything that reads them later
/// (search providers): always the current ones.
pub type SharedSettings = Arc<RwLock<Settings>>;

/// A copy of the current settings. A poisoned lock (a panic while writing)
/// falls back to the last value rather than failing.
pub fn current(shared: &SharedSettings) -> Settings {
    match shared.read() {
        Ok(s) => s.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct Settings {
    pub version: u32,
    pub hotkeys: HotkeySettings,
    pub ai: AiSettings,
    pub search: SearchSettings,
    pub general: GeneralSettings,
    // One section per feature (see `features/`).
    pub calculator: features::calc::CalcSettings,
    pub quicklinks: features::quicklinks::QuicklinkSettings,
    pub emoji: features::emoji::EmojiSettings,
    pub clipboard: features::clipboard::ClipboardSettings,
    pub snippets: features::snippets::SnippetSettings,
    pub scripts: features::scripts::ScriptSettings,
    pub updates: features::updater::UpdateSettings,
    pub system: features::system::SystemSettings,
    pub color_picker: features::color::ColorSettings,
    pub history: features::history::HistorySettings,
    pub ai_commands: features::ai_commands::AiCommandSettings,
    pub google: features::google::GoogleSettings,
    pub mcp: McpSettings,
    pub appearance: AppearanceSettings,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            hotkeys: HotkeySettings::default(),
            ai: AiSettings::default(),
            search: SearchSettings::default(),
            general: GeneralSettings::default(),
            calculator: Default::default(),
            quicklinks: Default::default(),
            emoji: Default::default(),
            clipboard: Default::default(),
            snippets: Default::default(),
            scripts: Default::default(),
            updates: Default::default(),
            system: Default::default(),
            color_picker: Default::default(),
            history: Default::default(),
            ai_commands: Default::default(),
            google: Default::default(),
            mcp: Default::default(),
            appearance: Default::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct HotkeySettings {
    /// Global shortcut that toggles the launcher, e.g. "Ctrl+Space".
    pub toggle: String,
    /// Tapping the Windows key on its own opens the launcher.
    pub win_key_override: bool,
    /// If Start opens anyway right after a tap (Windows didn't wait for the
    /// hook), close it and bring the launcher back.
    pub close_start: bool,
}

impl Default for HotkeySettings {
    fn default() -> Self {
        Self {
            toggle: "Ctrl+Space".into(),
            win_key_override: true,
            close_start: true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AiSettings {
    /// Gets any query that doesn't match an app, file or command, and Tab.
    /// (`c ` and `g ` in front of a question pick one; Shift+Tab asks the
    /// other.)
    pub default_provider: ProviderId,
    pub claude: ClaudeSettings,
    pub chatgpt: ChatGptSettings,
}

impl Default for AiSettings {
    fn default() -> Self {
        Self {
            default_provider: ProviderId::Claude,
            claude: ClaudeSettings::default(),
            chatgpt: ChatGptSettings::default(),
        }
    }
}

impl AiSettings {
    /// Whether `provider` is told what's on screen.
    pub fn desktop_context(&self, provider: ProviderId) -> bool {
        match provider {
            ProviderId::Claude => self.claude.desktop_context,
            ProviderId::Chatgpt => self.chatgpt.desktop_context,
        }
    }

    /// Whether `provider` may be started while a question is typed.
    pub fn warm_start(&self, provider: ProviderId) -> bool {
        match provider {
            ProviderId::Claude => self.claude.warm_start,
            ProviderId::Chatgpt => self.chatgpt.warm_start,
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
    /// Start Claude Code while you're typing a question, so the answer
    /// starts about a second sooner. Closed again if unused.
    pub warm_start: bool,
}

impl Default for ClaudeSettings {
    fn default() -> Self {
        Self {
            billing: BillingMode::Subscription,
            model: "sonnet".into(),
            approvals: Approvals::Ask,
            path: String::new(),
            desktop_context: true,
            warm_start: true,
        }
    }
}

/// ChatGPT, answered through OpenAI's Codex CLI.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ChatGptSettings {
    /// "subscription": your ChatGPT plan through Codex's sign-in.
    /// "api_key": the OPENAI_API_KEY environment variable, per token.
    pub billing: BillingMode,
    /// The model Codex uses ("gpt-6-luna"...). Empty: Codex's default
    /// (from its own config.toml, else its built-in choice).
    pub model: String,
    /// "ask": Codex runs in a read-only sandbox and you approve every
    /// change. "auto": it changes files inside the Explorer folder you came
    /// from without asking (no network), and asks for anything else.
    pub approvals: Approvals,
    /// Full path to `codex` (or its `codex.js`). Empty: find it automatically.
    pub path: String,
    /// Tell ChatGPT which window you were in, and Explorer's folder and
    /// selection; Explorer's folder also becomes its working folder.
    pub desktop_context: bool,
    /// Start Codex while you're typing a question for ChatGPT, so the
    /// answer starts sooner. Closed again if unused.
    pub warm_start: bool,
}

impl Default for ChatGptSettings {
    fn default() -> Self {
        Self {
            billing: BillingMode::Subscription,
            model: String::new(),
            approvals: Approvals::Ask,
            path: String::new(),
            desktop_context: true,
            warm_start: true,
        }
    }
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

/// Loads settings from one file, creating it with defaults if it doesn't
/// exist (the app itself uses `live::load_at_startup`, which also reads
/// this PC's file). A file that fails to parse is moved aside to `settings.toml.bad` and
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

/// Writes all settings to one file atomically (temp file, then rename).
pub fn save(path: &Path, settings: &Settings) -> Result<(), SettingsError> {
    disk::write_atomic(path, &toml::to_string_pretty(settings)?)
}

/// Parses one settings file's text. Returns `true` alongside the settings
/// if the file was migrated and should be written back.
fn parse(text: &str) -> Result<(Settings, bool), SettingsError> {
    let texts = disk::Texts {
        synced: Some(text.to_owned()),
        local: None,
    };
    disk::settings_from(&texts).map_err(|(_, e)| e)
}

fn default_table() -> Result<toml::Table, SettingsError> {
    toml::Table::try_from(Settings::default()).map_err(|e| SettingsError::Shape(e.to_string()))
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
        s.ai.chatgpt.billing = BillingMode::ApiKey;
        s.ai.chatgpt.model = "gpt-6-luna".into();
        s.ai.chatgpt.approvals = Approvals::Auto;
        s.ai.chatgpt.path = r"C:\tools\codex.exe".into();
        s.ai.chatgpt.desktop_context = false;
        s.ai.chatgpt.warm_start = false;
        save(&path, &s).unwrap();
        assert_eq!(load_or_create(&path).unwrap(), s);
    }

    #[test]
    fn chatgpt_defaults_and_older_files() {
        let d = ChatGptSettings::default();
        assert_eq!(d.billing, BillingMode::Subscription);
        assert_eq!(d.model, "", "Codex's own default model");
        assert_eq!(d.approvals, Approvals::Ask);
        assert!(d.desktop_context && d.warm_start);
        // A file from before these settings existed keeps its billing.
        let (s, _) = parse("version = 1\n[ai.chatgpt]\nbilling = \"api_key\"\n").unwrap();
        assert_eq!(s.ai.chatgpt.billing, BillingMode::ApiKey);
        assert_eq!(s.ai.chatgpt.approvals, Approvals::Ask);
        assert!(s.ai.desktop_context(ProviderId::Chatgpt));
        assert!(s.ai.chatgpt.path.is_empty());
    }

    #[test]
    fn partial_file_fills_defaults() {
        let (s, _) = parse("version = 1\n[hotkeys]\ntoggle = \"Alt+Space\"\n").unwrap();
        assert_eq!(s.hotkeys.toggle, "Alt+Space");
        assert!(s.hotkeys.win_key_override);
        assert!(s.hotkeys.close_start);
        assert_eq!(s.search, SearchSettings::default());
    }

    #[test]
    fn closing_start_can_be_turned_off() {
        let (s, _) = parse("version = 1\n[hotkeys]\nclose_start = false\n").unwrap();
        assert!(!s.hotkeys.close_start);
        assert!(s.hotkeys.win_key_override);
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
