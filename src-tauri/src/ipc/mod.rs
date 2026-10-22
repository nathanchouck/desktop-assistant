//! Commands the UI can call. Keep these thin: validate, delegate, convert errors.
//! After changing anything here, run `npm run bindings`.

mod chat;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use specta_typescript::Typescript;
use tauri::{AppHandle, State, Wry};
use tauri_specta::{collect_commands, collect_events, Builder, Event};

use crate::actions;
use crate::ai::claude::ClaudeStatus;
use crate::ai::codex::CodexStatus;
use crate::ai::{self, AiError, AiEvent, ProviderId};
use crate::desktop::{self, DesktopContext};
use crate::error::IpcError;
use crate::file_index::IndexStatus;
use crate::hotkey;
use crate::search::{self, Action, Activation, Frecency, ResultItem};
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
    let codex_changed = settings.ai.chatgpt.path != old.ai.chatgpt.path
        || settings.ai.chatgpt.billing != old.ai.chatgpt.billing
        || settings.ai.default_provider != old.ai.default_provider;
    if let Ok(mut current) = state.settings.write() {
        *current = settings.clone();
    }
    if claude_changed {
        crate::check_claude_status(&app, true);
    }
    if codex_changed {
        crate::check_codex_status(&app, true);
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
    /// Codex's install and ChatGPT sign-in ("Codex health").
    pub chatgpt: CodexStatus,
    pub file_index: IndexStatus,
    /// Everything is running, so file search covers every drive.
    pub everything: bool,
    pub win_key: hotkey::WinKeyStatus,
}

#[tauri::command]
#[specta::specta]
async fn app_status(app: AppHandle, state: State<'_, AppState>) -> Result<AppStatus, IpcError> {
    crate::check_claude_status(&app, false);
    crate::check_codex_status(&app, false);
    let settings = state.settings();
    Ok(AppStatus {
        claude: state.claude_status.get(),
        chatgpt: state.codex_status.get(),
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

/// Gives the Windows key hook a full second to answer before Windows uses
/// the key anyway (`longer`), or puts Windows' default back. Applies after
/// signing out.
#[tauri::command]
#[specta::specta]
fn win_key_set_timeout(longer: bool) -> Result<(), IpcError> {
    hotkey::set_longer_timeout(longer).map_err(|e| IpcError {
        message: e.to_string(),
    })
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
    // `c `/`g ` + a question is for that AI alone: no search, just its row.
    if let Some((provider, question)) = ai::forced_provider(&query) {
        match provider {
            ProviderId::Claude => crate::check_claude_status(&app, false),
            ProviderId::Chatgpt => crate::check_codex_status(&app, false),
        }
        let row = search::ask_ai_item(question, chat::ask_row(&state, provider));
        return Ok(SearchResponse {
            seq,
            items: vec![row],
        });
    }
    let limit = settings.search.max_results as usize;
    let store = Arc::clone(&state.store);
    let frecency: Frecency = Arc::new(move |id| store.frecency(id));
    let ask = chat::ask_row(&state, settings.ai.default_provider);
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

/// Runs an action on a result (open an app, run a command, ...). The reply
/// says what the UI should do next, if anything.
#[tauri::command]
#[specta::specta]
async fn item_activate(app: AppHandle, id: String, action: Action) -> Result<Activation, IpcError> {
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

/// A question was sent.
#[derive(Debug, Clone, Serialize, specta::Type)]
pub struct AskReply {
    pub conversation: u32,
    /// What the AI was told about the screen, in a few words
    /// ("Downloads · 2 selected"). `None` when nothing was added.
    pub context: Option<String>,
    /// Who answers this conversation (fixed when it starts).
    pub provider: ProviderId,
}

/// Starts the AI ahead of a question you seem to be typing, so the answer
/// starts sooner: `provider`, else the default one. Does nothing if that's
/// off, the AI can't answer, or a matching one is already waiting.
#[tauri::command]
#[specta::specta]
async fn ai_warm(state: State<'_, AppState>, provider: Option<ProviderId>) -> Result<(), IpcError> {
    let settings = state.settings().ai;
    let provider = provider.unwrap_or(settings.default_provider);
    if !settings.warm_start(provider) || !chat::ready(&state, provider) {
        return Ok(());
    }
    let context = if settings.desktop_context(provider) {
        Some(state.desktop.current(chat::CONTEXT_WAIT).await)
    } else {
        None
    };
    let setup = chat::new_conversation(&state, provider, context.as_ref()).map_err(ai_err)?;
    if let Err(e) = state.chat.warm(setup).await {
        tracing::debug!("could not warm up {}: {e}", provider.name());
    }
    Ok(())
}

/// Asks `provider` (else the default AI), starting a new conversation in
/// the folder File Explorer showed (else your home folder).
#[tauri::command]
#[specta::specta]
async fn ai_ask(
    state: State<'_, AppState>,
    text: String,
    provider: Option<ProviderId>,
) -> Result<AskReply, IpcError> {
    let provider = provider.unwrap_or(state.settings().ai.default_provider);
    let context = chat::context_to_send(&state, provider, true).await;
    let setup = chat::new_conversation(&state, provider, context.as_ref()).map_err(ai_err)?;
    let message = desktop::with_context(&text, context.as_ref());
    let conversation = state.chat.start(&message, setup).await.map_err(ai_err)?;
    let context = context.as_ref().and_then(DesktopContext::summary);
    crate::features::history::asked(&state.chat, conversation, &text, context.as_deref(), true)
        .await;
    Ok(AskReply {
        conversation,
        context,
        provider,
    })
}

/// Sends a follow-up in the current conversation (to the AI it started
/// with), with the desktop context if it changed since the last message.
#[tauri::command]
#[specta::specta]
async fn ai_follow_up(state: State<'_, AppState>, text: String) -> Result<AskReply, IpcError> {
    let provider = state
        .chat
        .provider()
        .await
        .ok_or_else(|| ai_err(AiError::NoConversation))?;
    let context = chat::context_to_send(&state, provider, false).await;
    let message = desktop::with_context(&text, context.as_ref());
    let conversation = state
        .chat
        .follow_up(&message, |provider| chat::setup_for(&state, provider))
        .await
        .map_err(ai_err)?;
    let context = context.as_ref().and_then(DesktopContext::summary);
    crate::features::history::asked(&state.chat, conversation, &text, context.as_deref(), false)
        .await;
    Ok(AskReply {
        conversation,
        context,
        provider,
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
    crate::features::history::closed();
    Ok(())
}

/// Opens a web link from an answer in the default browser.
#[tauri::command]
#[specta::specta]
fn open_external(app: AppHandle, url: String) -> Result<(), IpcError> {
    actions::open_url(&app, &url).map_err(Into::into)
}

/// Opens the folder with the log files (from an error toast).
#[tauri::command]
#[specta::specta]
fn open_log_folder(app: AppHandle) -> Result<(), IpcError> {
    actions::run_builtin(&app, crate::search::commands::BuiltinCommand::OpenLogFolder)
        .map_err(Into::into)
}

/// The scripts folder and what's in it, for the settings menu; read again
/// first when `reload` is set.
#[tauri::command]
#[specta::specta]
async fn scripts_list(
    reload: bool,
) -> Result<crate::features::scripts::library::ScriptsOverview, IpcError> {
    crate::features::scripts::overview(reload)
        .await
        .map_err(|message| IpcError { message })
}

/// Opens the scripts folder in File Explorer, making it first (with a
/// README and an example) if it doesn't exist.
#[tauri::command]
#[specta::specta]
async fn scripts_open_folder(state: State<'_, AppState>) -> Result<(), IpcError> {
    crate::features::scripts::open_folder(&state.shell)
        .await
        .map_err(|message| IpcError { message })
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
            win_key_set_timeout,
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
            open_external,
            open_log_folder,
            // Features add their commands below their own line, so
            // parallel work doesn't collide.
            // [ai]
            // [history]
            crate::features::history::commands::history_overview,
            crate::features::history::commands::history_clear,
            crate::features::history::commands::chat_copy_answer,
            crate::features::history::commands::chat_paste_answer,
            // [clipboard]
            // [snippets]
            // [system]
            // [color]
            // [scripts]
            scripts_list,
            scripts_open_folder,
            // [updater]
            crate::features::updater::commands::update_status,
            crate::features::updater::commands::update_check,
            crate::features::updater::commands::update_install,
            // [integrations]
            // [ai_commands]
            // [appearance]
            // [google]
            // [polish]
        ])
        .events(collect_events![
            LauncherShown,
            AiStream,
            FileIndexChanged,
            SearchUpdated,
            ShowSettings,
            crate::toast::Toast,
            // [ai]
            // [history]
            // [clipboard]
            // [color]
            crate::features::color::ColorPicked,
            // [updater]
            crate::features::updater::UpdateStatusChanged,
            // [ai_commands]
            // [google]
            // [polish]
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
