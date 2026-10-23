//! AI inputs: saved AI commands (prompt templates), and questions that
//! carry attachments: the selected text (a global shortcut), a file or
//! folder (Ctrl+Shift+A on a file row), a screen region (a snip), plus
//! dictating into the search box.
//!
//! - `attachments.rs`: what a question can carry, and the blocks it adds
//!   to the message (pure).
//! - `images.rs`: attached pictures, saved as PNG files for the AI.
//! - `selection.rs`: copying the selection from another app.
//! - `shortcut.rs`: the "ask about the selection" shortcut.
//! - `screen.rs`: snipping a screen region.
//! - `template.rs`: placeholders in saved prompts (pure).
//! - `saved.rs`: saved commands, their defaults and search rows (pure).
//! - `provider.rs`: search rows and what Enter does.
//! - `commands.rs`: what the UI calls (dictate, pausing the shortcut).

pub mod attachments;
pub mod commands;
pub mod images;
mod provider;
pub mod saved;
mod screen;
mod selection;
pub mod shortcut;
pub mod template;

use std::path::Path;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::ProviderContext;
use crate::ai::message::{ImageInput, Message};
use crate::desktop::{self, DesktopContext};
use crate::search::SearchProvider;
use crate::state::AppState;
pub use attachments::Attachment;
pub use saved::{AiCommand, CommandAi};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AiCommandSettings {
    /// Saved AI commands, "Ask About Screen Region" and "Dictate" in search.
    pub enabled: bool,
    /// Global shortcut that copies what's selected in the window in front
    /// and opens the launcher to ask about it. Empty: off.
    pub selection_shortcut: String,
    /// After copying the selection, put back what was on the clipboard.
    pub restore_clipboard: bool,
    pub commands: Vec<AiCommand>,
}

impl Default for AiCommandSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            selection_shortcut: "Ctrl+Shift+Space".into(),
            restore_clipboard: true,
            commands: saved::defaults(),
        }
    }
}

/// Open the launcher with these attached to the next question (the
/// selection, a snip). The UI shows them as chips above the input.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ComposeRequested {
    pub attachments: Vec<Attachment>,
}

/// For the snip and the shortcut, which run outside any command
/// (providers are made before the app runs).
static APP: OnceLock<AppHandle> = OnceLock::new();

fn app() -> Option<AppHandle> {
    APP.get().cloned()
}

/// Tells the UI to start a question with `attachments`.
fn compose(app: &AppHandle, attachments: Vec<Attachment>) {
    if let Err(e) = (ComposeRequested { attachments }).emit(app) {
        tracing::warn!("could not open the launcher with the attachments: {e}");
    }
}

/// Saved commands and the AI inputs in search.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::AiCommandProvider::new(
        Arc::clone(&ctx.settings),
        Arc::clone(&ctx.shell),
    )))
}

/// Registers the selection shortcut and clears pictures left from last
/// time (no AI can still be reading them).
pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    let Some(state) = app.try_state::<AppState>() else {
        return;
    };
    shortcut::register(app, &state.settings().ai_commands.selection_shortcut);
    let dir = state.paths.data_dir.join(images::DIR_NAME);
    let thumbs = state.paths.thumbnails_dir();
    tauri::async_runtime::spawn_blocking(move || images::sweep(&dir, &thumbs, Duration::ZERO));
}

/// The message for a question: the desktop context, the attachments'
/// blocks, the question, and the attached pictures read from disk.
pub async fn message(
    data_dir: &Path,
    question: &str,
    attached: &[Attachment],
    context: Option<&DesktopContext>,
) -> Result<Message, String> {
    let text = desktop::with_context(&attachments::prompt_text(question, attached), context);
    let dir = data_dir.join(images::DIR_NAME);
    let mut pictures = Vec::new();
    for id in attachments::image_ids(attached) {
        let path = images::path(&dir, id).ok_or("That attached picture isn't one of ours.")?;
        let png = tokio::fs::read(&path)
            .await
            .map_err(|_| "The attached picture is gone. Snip it again.".to_owned())?;
        pictures.push(ImageInput { path, png });
    }
    Ok(Message::new(text, pictures))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults() {
        let s = AiCommandSettings::default();
        assert!(s.enabled && s.restore_clipboard);
        assert_eq!(s.selection_shortcut, "Ctrl+Shift+Space");
        assert_eq!(s.commands.len(), 8);
    }

    #[test]
    fn settings_round_trip_through_toml() {
        let mut s = AiCommandSettings::default();
        s.selection_shortcut.clear();
        s.commands[0].provider = CommandAi::Chatgpt;
        let text = toml::to_string(&s).unwrap();
        assert!(text.contains("provider = \"chatgpt\""), "{text}");
        assert_eq!(toml::from_str::<AiCommandSettings>(&text).unwrap(), s);
    }

    #[tokio::test]
    async fn messages_carry_blocks_and_pictures() {
        let dir = tempfile::tempdir().unwrap();
        let picture = crate::platform::Image {
            width: 2,
            height: 1,
            rgba: vec![1; 8],
        };
        let saved = images::save(
            picture,
            &dir.path().join(images::DIR_NAME),
            &dir.path().join("thumbnails"),
        )
        .unwrap();
        let attached = [Attachment::Text { text: "teh".into() }, saved];
        let message = message(dir.path(), "fix", &attached, None).await.unwrap();
        assert!(message.text.starts_with("<selected_text>\nteh\n</selected_text>"));
        assert!(message.text.ends_with("\n\nfix"));
        assert_eq!(message.images.len(), 1);
        assert!(message.images[0].png.starts_with(b"\x89PNG"));

        let gone = Attachment::Image {
            id: "abc".into(),
            width: 1,
            height: 1,
        };
        assert!(message_err(dir.path(), gone).await.contains("gone"));
        let sneaky = Attachment::Image {
            id: "../x".into(),
            width: 1,
            height: 1,
        };
        assert!(message_err(dir.path(), sneaky).await.contains("isn't one of ours"));
    }

    async fn message_err(dir: &Path, attachment: Attachment) -> String {
        message(dir, "q", &[attachment], None).await.unwrap_err()
    }
}
