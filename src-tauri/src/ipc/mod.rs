//! Commands the UI can call. Keep these thin: validate, delegate, convert errors.
//! After changing anything here, run `npm run bindings`.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde::{Deserialize, Serialize};
use specta_typescript::Typescript;
use tauri::{AppHandle, State, Wry};
use tauri_specta::{collect_commands, collect_events, Builder, Event};

use crate::actions;
use crate::ai::claude::ClaudeStatus;
use crate::ai::{claude, AiEvent, ProviderId};
use crate::desktop::{self, DesktopContext};
use crate::error::IpcError;
use crate::file_index::IndexStatus;
use crate::search::{Action, AskRow, Frecency, ResultItem};
use crate::settings::Settings;
use crate::state::AppState;
use crate::window;

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct AppInfo {
    pub version: String,
    pub settings_file: String,
    pub log_dir: String,
    /// True when the OS blur is active behind the window.
    pub glass: bool,
}

#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SearchResponse {
    /// Echo of the request's sequence number, so the UI can drop stale replies.
    pub seq: u32,
    pub items: Vec<ResultItem>,
}

#[tauri::command]
#[specta::specta]
fn app_info(state: State<'_, AppState>) -> AppInfo {
    AppInfo {
        version: env!("CARGO_PKG_VERSION").into(),
        settings_file: state.paths.settings_file().display().to_string(),
        log_dir: state.paths.log_dir().display().to_string(),
        glass: state.launcher.glass(),
    }
}

#[tauri::command]
#[specta::specta]
fn settings_get(state: State<'_, AppState>) -> Settings {
    state.settings()
}

#[tauri::command]
#[specta::specta]
async fn search_query(
    app: AppHandle,
    state: State<'_, AppState>,
    query: String,
    seq: u32,
) -> Result<SearchResponse, IpcError> {
    let settings = state.settings();
    let limit = settings.search.max_results as usize;
    let store = Arc::clone(&state.store);
    let frecency: Frecency = Arc::new(move |id| store.frecency(id));
    let ask = match (settings.ai.default_provider, state.claude_status.get()) {
        (ProviderId::Claude, ClaudeStatus::NotInstalled) => AskRow::InstallClaude,
        (ProviderId::Claude, ClaudeStatus::SignedOut) => AskRow::SignInToClaude,
        (provider, _) => AskRow::Ask(provider),
    };
    let outcome = state.search.search(&query, limit, ask, frecency).await;
    // A slow provider (big file index, busy disk) sends its results later
    // instead of dropping them.
    if let Some(complete) = outcome.complete {
        tauri::async_runtime::spawn(async move {
            let items = complete.await;
            if let Err(e) = (SearchUpdated { seq, items }).emit(&app) {
                tracing::debug!("could not send late results: {e}");
            }
        });
    }
    Ok(SearchResponse {
        seq,
        items: outcome.items,
    })
}

/// How far along the built-in file index is (for the footer).
#[tauri::command]
#[specta::specta]
fn file_index_status(state: State<'_, AppState>) -> IndexStatus {
    state.files.status()
}

/// Runs an action on a result (open an app, run a command, ...).
#[tauri::command]
#[specta::specta]
async fn item_activate(app: AppHandle, id: String, action: Action) -> Result<(), IpcError> {
    actions::activate(&app, &id, action)
        .await
        .map_err(Into::into)
}

#[tauri::command]
#[specta::specta]
fn launcher_hide(app: AppHandle) {
    window::hide(&app);
}

/// The UI reports its content height so the window can fit it.
#[tauri::command]
#[specta::specta]
fn launcher_set_height(app: AppHandle, height: u32) {
    window::set_height(&app, height);
}

/// How long asking waits for File Explorer's folder and selection.
const CONTEXT_WAIT: Duration = Duration::from_millis(800);

/// A question was sent.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct AskReply {
    pub conversation: u32,
    /// What Claude was told about the screen, in a few words
    /// ("Downloads · 2 selected"). `None` when nothing was added.
    pub context: Option<String>,
}

/// The desktop context to send now: `None` when it's off, or when it's
/// the same as the last one sent in this conversation (`fresh: false`).
async fn context_to_send(state: &AppState, fresh: bool) -> Option<DesktopContext> {
    if !state.settings().ai.claude.desktop_context {
        return None;
    }
    let context = state.desktop.current(CONTEXT_WAIT).await;
    context.window.as_ref()?;
    let mut sent = state.sent_context.lock().ok()?;
    if !fresh && sent.as_ref() == Some(&context) {
        return None;
    }
    *sent = Some(context.clone());
    Some(context)
}

/// Asks Claude, starting a new conversation in the folder File Explorer
/// showed (else your home folder).
#[tauri::command]
#[specta::specta]
async fn ai_ask(state: State<'_, AppState>, text: String) -> Result<AskReply, IpcError> {
    let (launch, mut options) = claude::prepare(&state.settings().ai.claude).map_err(ai_err)?;
    let context = context_to_send(&state, true).await;
    if let Some(folder) = context.as_ref().and_then(DesktopContext::folder) {
        if Path::new(folder).is_dir() {
            options.cwd = PathBuf::from(folder);
        }
    }
    let message = desktop::with_context(&text, context.as_ref());
    let conversation = state
        .chat
        .start(&message, &launch, options)
        .await
        .map_err(ai_err)?;
    Ok(AskReply {
        conversation,
        context: context.as_ref().and_then(DesktopContext::summary),
    })
}

/// Sends a follow-up in the current conversation, with the desktop
/// context if it changed since the last message.
#[tauri::command]
#[specta::specta]
async fn ai_follow_up(state: State<'_, AppState>, text: String) -> Result<AskReply, IpcError> {
    let (launch, options) = claude::prepare(&state.settings().ai.claude).map_err(ai_err)?;
    let context = context_to_send(&state, false).await;
    let message = desktop::with_context(&text, context.as_ref());
    let conversation = state
        .chat
        .follow_up(&message, &launch, options)
        .await
        .map_err(ai_err)?;
    Ok(AskReply {
        conversation,
        context: context.as_ref().and_then(DesktopContext::summary),
    })
}

/// Approves or denies a pending action.
#[tauri::command]
#[specta::specta]
async fn ai_approve(state: State<'_, AppState>, id: String, allow: bool) -> Result<(), IpcError> {
    state.chat.respond(&id, allow).await.map_err(ai_err)
}

/// Stops the current answer; the conversation stays open.
#[tauri::command]
#[specta::specta]
async fn ai_stop(state: State<'_, AppState>) -> Result<(), IpcError> {
    state.chat.stop().await;
    Ok(())
}

/// Ends the conversation.
#[tauri::command]
#[specta::specta]
async fn ai_close(state: State<'_, AppState>) -> Result<(), IpcError> {
    state.chat.close().await;
    Ok(())
}

/// Opens a web link from an answer in the default browser.
#[tauri::command]
#[specta::specta]
fn open_external(app: AppHandle, url: String) -> Result<(), IpcError> {
    use tauri_plugin_opener::OpenerExt;
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(IpcError {
            message: "only web links can be opened".into(),
        });
    }
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| IpcError {
            message: e.to_string(),
        })
}

fn ai_err(e: crate::ai::AiError) -> IpcError {
    crate::error::AppError::from(e).into()
}

/// One step of a conversation, streamed to the UI.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct AiStream {
    pub conversation: u32,
    pub event: AiEvent,
}

/// Fuller results for search `seq`, from providers that missed the
/// per-keystroke budget. Replaces that search's list.
#[derive(Debug, Clone, Serialize, specta::Type, tauri_specta::Event)]
pub struct SearchUpdated {
    pub seq: u32,
    pub items: Vec<ResultItem>,
}

/// The file index's progress changed (sent a few times a second while it
/// scans).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct FileIndexChanged {
    pub status: IndexStatus,
}

/// Sent each time the launcher appears, so the UI can focus the search box.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct LauncherShown;

pub fn builder() -> Builder<Wry> {
    Builder::<Wry>::new()
        .commands(collect_commands![
            app_info,
            settings_get,
            search_query,
            file_index_status,
            item_activate,
            launcher_hide,
            launcher_set_height,
            ai_ask,
            ai_follow_up,
            ai_approve,
            ai_stop,
            ai_close,
            open_external
        ])
        .events(collect_events![
            LauncherShown,
            AiStream,
            FileIndexChanged,
            SearchUpdated
        ])
}

/// Where the generated TypeScript bindings live.
pub fn bindings_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../ui/src/lib/bindings.ts")
}

pub fn export_bindings(path: &Path) -> Result<(), String> {
    builder()
        .export(
            Typescript::default().header("// GENERATED by `npm run bindings`. Do not edit.\n"),
            path,
        )
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fails if someone changed an IPC type without regenerating bindings.
    #[test]
    fn bindings_are_up_to_date() {
        let dir = tempfile::tempdir().unwrap();
        let fresh = dir.path().join("bindings.ts");
        export_bindings(&fresh).unwrap();
        let committed = std::fs::read_to_string(bindings_path()).unwrap_or_default();
        let fresh = std::fs::read_to_string(fresh).unwrap();
        assert!(
            committed == fresh,
            "ui/src/lib/bindings.ts is stale; run `npm run bindings`"
        );
    }
}
