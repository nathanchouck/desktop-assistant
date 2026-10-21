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
use crate::hotkey;
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

/// The result of saving settings.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct SettingsSaved {
    /// Index folders or exclusions differ from what the running index uses;
    /// they apply after a restart.
    pub restart_needed: bool,
}

/// Saves settings from the settings menu and applies what can change live:
/// the shortcut, the Windows key, starting at login. Claude and search
/// options are read fresh each time, so they apply at once too.
#[tauri::command]
#[specta::specta]
async fn settings_set(
    app: AppHandle,
    state: State<'_, AppState>,
    settings: Settings,
) -> Result<SettingsSaved, IpcError> {
    let old = state.settings();
    // The shortcut first: if it can't be registered, nothing is saved.
    if settings.hotkeys.toggle != old.hotkeys.toggle {
        hotkey::change_toggle(&app, &old.hotkeys.toggle, &settings.hotkeys.toggle).map_err(
            |e| IpcError {
                message: e.to_string(),
            },
        )?;
    }
    if settings.hotkeys.win_key_override != old.hotkeys.win_key_override {
        hotkey::set_win_key(&app, settings.hotkeys.win_key_override);
    }
    if settings.general.launch_at_login != old.general.launch_at_login {
        crate::sync_launch_at_login(&app, settings.general.launch_at_login);
    }
    crate::settings::save(&state.paths.settings_file(), &settings).map_err(|e| IpcError {
        message: e.to_string(),
    })?;
    let claude_changed = settings.ai.claude.path != old.ai.claude.path
        || settings.ai.claude.billing != old.ai.claude.billing;
    if let Ok(mut current) = state.settings.write() {
        *current = settings.clone();
    }
    if claude_changed {
        crate::check_claude_status(&app, true);
    }
    let running = &state.startup_search;
    Ok(SettingsSaved {
        restart_needed: settings.search.index_folders != running.index_folders
            || settings.search.exclusions != running.exclusions,
    })
}

/// What the settings menu shows under "Status".
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct AppStatus {
    pub claude: ClaudeStatus,
    pub file_index: IndexStatus,
    /// Everything is running, so file search covers every drive.
    pub everything: bool,
    pub win_key: hotkey::WinKeyStatus,
}

#[tauri::command]
#[specta::specta]
async fn app_status(app: AppHandle, state: State<'_, AppState>) -> Result<AppStatus, IpcError> {
    crate::check_claude_status(&app, false);
    let settings = state.settings();
    Ok(AppStatus {
        claude: state.claude_status.get(),
        file_index: state.files.status(),
        everything: state.shell.everything_running().await,
        win_key: hotkey::win_key_status(settings.hotkeys.win_key_override),
    })
}

/// Pauses the launcher's shortcut while the settings menu records a new one,
/// so pressing the old one doesn't hide the launcher.
#[tauri::command]
#[specta::specta]
fn shortcut_pause(app: AppHandle, state: State<'_, AppState>, paused: bool) {
    let toggle = state.settings().hotkeys.toggle;
    let result = if paused {
        hotkey::shortcut::unregister(&app, &toggle)
    } else {
        hotkey::shortcut::register(&app, &toggle)
    };
    if let Err(e) = result {
        tracing::debug!("{e}");
    }
}

/// Restarts the launcher (to re-index with new folders).
#[tauri::command]
#[specta::specta]
async fn app_restart(app: AppHandle) -> Result<(), IpcError> {
    actions::restart(&app).await;
    Ok(())
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

/// Points Claude Code at the launcher's MCP server once it's running.
fn add_launcher_tools(state: &AppState, options: &mut claude::Options) {
    if let Some(files) = state.tool_files.get() {
        options.mcp_config = Some(files.mcp_config.clone());
        options.settings = Some(files.settings.clone());
    }
}

/// How a new conversation starts: the settings, the launcher's tools, and
/// Explorer's folder (from `context`) as the working folder.
fn new_conversation(
    state: &AppState,
    context: Option<&DesktopContext>,
) -> Result<(claude::Launch, claude::Options), IpcError> {
    let (launch, mut options) = claude::prepare(&state.settings().ai.claude).map_err(ai_err)?;
    add_launcher_tools(state, &mut options);
    if let Some(folder) = context.and_then(DesktopContext::folder) {
        if Path::new(folder).is_dir() {
            options.cwd = PathBuf::from(folder);
        }
    }
    Ok((launch, options))
}

/// Starts Claude Code ahead of a question you seem to be typing, so the
/// answer starts sooner. Does nothing if it's off, Claude Code can't
/// answer, or a matching one is already waiting.
#[tauri::command]
#[specta::specta]
async fn ai_warm(state: State<'_, AppState>) -> Result<(), IpcError> {
    let settings = state.settings().ai.claude;
    let ready = matches!(
        state.claude_status.get(),
        ClaudeStatus::Ready | ClaudeStatus::Unknown
    );
    if !settings.warm_start || !ready {
        return Ok(());
    }
    let context = if settings.desktop_context {
        Some(state.desktop.current(CONTEXT_WAIT).await)
    } else {
        None
    };
    let (launch, options) = new_conversation(&state, context.as_ref())?;
    if let Err(e) = state.chat.warm(&launch, options).await {
        tracing::debug!("could not warm up Claude Code: {e}");
    }
    Ok(())
}

/// Asks Claude, starting a new conversation in the folder File Explorer
/// showed (else your home folder).
#[tauri::command]
#[specta::specta]
async fn ai_ask(state: State<'_, AppState>, text: String) -> Result<AskReply, IpcError> {
    let context = context_to_send(&state, true).await;
    let (launch, options) = new_conversation(&state, context.as_ref())?;
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
    let (launch, mut options) = claude::prepare(&state.settings().ai.claude).map_err(ai_err)?;
    add_launcher_tools(&state, &mut options);
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

/// Open the settings menu (from the tray).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ShowSettings;

/// Sent each time the launcher appears, so the UI can focus the search box.
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct LauncherShown;

pub fn builder() -> Builder<Wry> {
    Builder::<Wry>::new()
        .commands(collect_commands![
            app_info,
            settings_get,
            settings_set,
            app_status,
            app_restart,
            shortcut_pause,
            search_query,
            file_index_status,
            item_activate,
            launcher_hide,
            launcher_set_height,
            ai_warm,
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
            SearchUpdated,
            ShowSettings
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
