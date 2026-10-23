//! The configurable global shortcut, via Tauri's global-shortcut plugin.

use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, Wry};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

use super::HotkeyError;

/// The plugin, with a handler that toggles the launcher on key press (or,
/// for the "ask about the selection" shortcut, grabs the selection).
pub fn plugin() -> TauriPlugin<Wry> {
    tauri_plugin_global_shortcut::Builder::new()
        .with_handler(|app, shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }
            // [ai_commands]
            if crate::features::ai_commands::shortcut::is_selection_shortcut(app, shortcut) {
                crate::features::ai_commands::shortcut::pressed(app);
                return;
            }
            super::toggle_now(app);
        })
        .build()
}

pub fn parse(text: &str) -> Result<Shortcut, HotkeyError> {
    text.parse::<Shortcut>()
        .map_err(|e| HotkeyError::InvalidShortcut {
            shortcut: text.into(),
            reason: e.to_string(),
        })
}

pub fn unregister(app: &AppHandle, text: &str) -> Result<(), HotkeyError> {
    let shortcut = parse(text)?;
    app.global_shortcut()
        .unregister(shortcut)
        .map_err(|e| HotkeyError::Register {
            shortcut: text.into(),
            reason: e.to_string(),
        })
}

pub fn register(app: &AppHandle, text: &str) -> Result<(), HotkeyError> {
    let shortcut = parse(text)?;
    app.global_shortcut()
        .register(shortcut)
        .map_err(|e| HotkeyError::Register {
            shortcut: text.into(),
            reason: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::HotkeySettings;

    #[test]
    fn default_shortcut_parses() {
        parse(&HotkeySettings::default().toggle).unwrap();
    }

    #[test]
    fn common_shortcuts_parse() {
        for s in ["Alt+Space", "ctrl+shift+k", "Super+Space", "F12"] {
            parse(s).unwrap_or_else(|e| panic!("{s}: {e}"));
        }
    }

    #[test]
    fn bad_shortcut_is_a_clear_error() {
        let err = parse("Ctrl+Nope").unwrap_err().to_string();
        assert!(err.contains("Ctrl+Nope"), "{err}");
    }
}
