//! Commands the UI can call. Keep these thin: validate, delegate, convert errors.
//! After changing anything here, run `npm run bindings`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use specta_typescript::Typescript;
use tauri::{AppHandle, State, Wry};
use tauri_specta::{collect_commands, collect_events, Builder};

use crate::actions;
use crate::ai::{claude, AiEvent};
use crate::error::IpcError;
use crate::search::{Action, ResultItem};
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
    state: State<'_, AppState>,
    query: String,
    seq: u32,
) -> Result<SearchResponse, IpcError> {
    let settings = state.settings();
    let limit = settings.search.max_results as usize;
    let store = &state.store;
    let items = state
        .search
        .search(&query, limit, settings.ai.default_provider, &|id| {
            store.frecency(id)
        })
        .await;
    Ok(SearchResponse { seq, items })
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

/// Asks Claude, starting a new conversation. Returns its id.
#[tauri::command]
#[specta::specta]
async fn ai_ask(state: State<'_, AppState>, text: String) -> Result<u32, IpcError> {
    let (launch, options) = claude::prepare(&state.settings().ai.claude).map_err(ai_err)?;
    state
        .chat
        .start(&text, &launch, options)
        .await
        .map_err(ai_err)
}

/// Sends a follow-up in the current conversation.
#[tauri::command]
#[specta::specta]
async fn ai_follow_up(state: State<'_, AppState>, text: String) -> Result<u32, IpcError> {
    let (launch, options) = claude::prepare(&state.settings().ai.claude).map_err(ai_err)?;
    state
        .chat
        .follow_up(&text, &launch, options)
        .await
        .map_err(ai_err)
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

/// Sent each time the launcher appears, so the UI can focus the search box.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct LauncherShown;

pub fn builder() -> Builder<Wry> {
    Builder::<Wry>::new()
        .commands(collect_commands![
            app_info,
            settings_get,
            search_query,
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
        .events(collect_events![LauncherShown, AiStream])
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
