//! The "ask about the selection" shortcut (Ctrl+Shift+Space by default). It
//! is registered with the same global-shortcut plugin as the launcher's own
//! shortcut; the plugin's handler asks `is_selection_shortcut` to tell the
//! two apart. Empty in settings: off.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

use tauri::{AppHandle, Manager};
use tauri_plugin_global_shortcut::Shortcut;

use super::selection;
use crate::hotkey::{shortcut, HotkeyError};
use crate::state::AppState;
use crate::{toast, window};

/// One grab at a time: a held shortcut repeats.
static GRABBING: AtomicBool = AtomicBool::new(false);

/// Registers the shortcut at startup, if one is set. If another app has
/// it, a toast says so (shown when the launcher first opens).
pub fn register(app: &AppHandle, text: &str) {
    if text.trim().is_empty() {
        return;
    }
    match shortcut::register(app, text) {
        Ok(()) => tracing::info!("selection shortcut {text} registered"),
        Err(e) => toast::error(
            app,
            format!("Ask about the selection is off: {e}. Pick another shortcut in Settings."),
        ),
    }
}

/// Swaps the shortcut (either may be empty: off). If the new one can't be
/// registered, the old one stays and the error says why.
pub fn change(app: &AppHandle, old: &str, new: &str) -> Result<(), HotkeyError> {
    let (old, new) = (old.trim(), new.trim());
    if !new.is_empty() {
        shortcut::parse(new)?;
    }
    if !old.is_empty() {
        let _ = shortcut::unregister(app, old);
    }
    if new.is_empty() {
        return Ok(());
    }
    if let Err(e) = shortcut::register(app, new) {
        if !old.is_empty() {
            let _ = shortcut::register(app, old);
        }
        return Err(e);
    }
    tracing::info!("selection shortcut changed to {new}");
    Ok(())
}

/// Pauses the shortcut while the settings menu records a new one.
pub fn pause(app: &AppHandle, text: &str, paused: bool) {
    if text.trim().is_empty() {
        return;
    }
    let result = if paused {
        shortcut::unregister(app, text)
    } else {
        shortcut::register(app, text)
    };
    if let Err(e) = result {
        tracing::debug!("{e}");
    }
}

/// Whether `pressed` is the selection shortcut (else it's the launcher's).
pub fn is_selection_shortcut(app: &AppHandle, pressed: &Shortcut) -> bool {
    let Some(state) = app.try_state::<AppState>() else {
        return false;
    };
    let text = match state.settings.read() {
        Ok(s) => s.ai_commands.selection_shortcut.clone(),
        Err(poisoned) => poisoned.into_inner().ai_commands.selection_shortcut.clone(),
    };
    !text.trim().is_empty() && shortcut::parse(&text).is_ok_and(|s| s == *pressed)
}

/// The shortcut was pressed: copy the selection in the window in front
/// (or, with the launcher open, the window it came from), then open the
/// launcher with it attached.
pub fn pressed(app: &AppHandle) {
    if GRABBING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        ask_about_selection(&app).await;
        GRABBING.store(false, Ordering::SeqCst);
    });
}

async fn ask_about_selection(app: &AppHandle) {
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    let restore = state.settings().ai_commands.restore_clipboard;
    let open = window::launcher(app).is_some_and(|w| w.is_visible().unwrap_or(false));
    let grabbed = if open {
        selection::grab_behind_launcher(app, restore).await
    } else {
        selection::grab(&state.shell, restore).await
    };
    let attachments = match grabbed {
        Ok(Some(copied)) => {
            selection::to_attachments(copied, &state.paths.data_dir, &state.paths.thumbnails_dir())
                .await
        }
        Ok(None) => Vec::new(),
        Err(e) => {
            tracing::warn!("could not copy the selection: {e}");
            Vec::new()
        }
    };
    window::show(app, Instant::now());
    if attachments.is_empty() {
        toast::info(app, "Nothing selected: select some text or files first.");
    } else {
        super::compose(app, attachments);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::ai_commands::AiCommandSettings;

    #[test]
    fn default_shortcut_parses_and_differs_from_the_launchers() {
        let ours = shortcut::parse(&AiCommandSettings::default().selection_shortcut).unwrap();
        let toggle = shortcut::parse(&crate::settings::HotkeySettings::default().toggle).unwrap();
        assert_ne!(ours, toggle);
        assert_eq!(ours, shortcut::parse("ctrl+shift+space").unwrap());
    }
}
