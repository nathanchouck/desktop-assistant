//! Carrying out what the user picked. Shared by the results list and the tray.

use std::path::Path;

use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;
use tauri_specta::Event;

use crate::error::{AppError, AppResult};
use crate::platform::{vk, PlatformError, Shell};
use crate::search::apps::target_from_id;
use crate::search::commands::BuiltinCommand;
use crate::search::files::path_from_id;
use crate::search::{Action, Activation};
use crate::state::AppState;
use crate::window;

const CLAUDE_INSTALL_PAGE: &str = "https://code.claude.com/docs/en/setup";
/// The Codex README's install steps (Windows installer, npm).
const CODEX_INSTALL_PAGE: &str = "https://github.com/openai/codex#installing-and-running-codex-cli";
/// Between bringing your window back and pasting into it.
const PASTE_DELAY: std::time::Duration = std::time::Duration::from_millis(60);

/// Runs `action` on the result with `id`, records the open for ranking,
/// and carries out what follows (hide, copy, paste, open a link). Returns
/// what's left for the UI (see `Activation`).
pub async fn activate(app: &AppHandle, id: &str, action: Action) -> AppResult<Activation> {
    let state = app.try_state::<AppState>().ok_or(AppError::NotReady)?;
    let activation = if let Some(provider) = state.search.owner(id) {
        provider
            .activate(id, action)
            .await
            .map_err(|e| AppError::Launch(e.to_string()))?
    } else {
        activate_builtin(app, &state.shell, id, action).await?
    };
    let opened = !matches!(action, Action::Delete | Action::Pin)
        && !matches!(activation, Activation::Notice { .. } | Activation::Refresh);
    if opened {
        if let Err(e) = state.store.record_open(id) {
            tracing::warn!("could not record open: {e}");
        }
    }
    carry_out(app, activation).await
}

/// Does the part of an activation that happens in Rust.
pub async fn carry_out(app: &AppHandle, activation: Activation) -> AppResult<Activation> {
    match &activation {
        Activation::Hide => window::hide(app),
        Activation::Copy { text } => {
            copy_text(app, text)?;
            window::hide(app);
        }
        Activation::Paste { text } => paste(app, text.as_deref()).await?,
        Activation::OpenUrl { url } => {
            open_url(app, url)?;
            window::hide(app);
        }
        Activation::SetQuery { .. }
        | Activation::Ask { .. }
        | Activation::Notice { .. }
        | Activation::Refresh
        | Activation::OpenSettings
        | Activation::OpenChat { .. } => {}
    }
    Ok(activation)
}

pub fn copy_text(app: &AppHandle, text: &str) -> AppResult<()> {
    app.clipboard()
        .write_text(text)
        .map_err(|e| AppError::Clipboard(e.to_string()))
}

/// Hides the launcher and pastes into the window you were in: `text`, or
/// what's on the clipboard already.
pub async fn paste(app: &AppHandle, text: Option<&str>) -> AppResult<()> {
    if let Some(text) = text {
        copy_text(app, text)?;
    }
    let state = app.try_state::<AppState>().ok_or(AppError::NotReady)?;
    let target = state.desktop.window();
    window::hide(app);
    if let Some(target) = target {
        if let Err(e) = state.shell.focus_window(target.handle).await {
            tracing::debug!("could not bring {} back to the front: {e}", target.app);
        }
    }
    // Give the window a moment to take the keyboard.
    tokio::time::sleep(PASTE_DELAY).await;
    state
        .shell
        .press_keys(vec![vk::CONTROL, vk::V])
        .await
        .map_err(|e| AppError::Launch(format!("couldn't paste: {e}")))
}

/// Opens a web link in the browser. Only http and https.
pub fn open_url(app: &AppHandle, url: &str) -> AppResult<()> {
    if !(url.starts_with("https://") || url.starts_with("http://")) {
        return Err(AppError::Open {
            path: url.to_owned(),
            reason: "only web links can be opened".into(),
        });
    }
    app.opener()
        .open_url(url, None::<&str>)
        .map_err(|e| AppError::Open {
            path: url.to_owned(),
            reason: e.to_string(),
        })
}

/// Commands, apps and files: the results that came before `features/`.
async fn activate_builtin(
    app: &AppHandle,
    shell: &Shell,
    id: &str,
    action: Action,
) -> AppResult<Activation> {
    let launch_err = |e: PlatformError| AppError::Launch(e.to_string());

    if let Some(cmd) = BuiltinCommand::from_id(id) {
        if cmd == BuiltinCommand::OpenSettings {
            return Ok(Activation::OpenSettings);
        }
        run_builtin(app, cmd)?;
        return Ok(Activation::Hide);
    }
    if let Some(target) = target_from_id(id) {
        let as_admin = action == Action::RunAsAdmin;
        tracing::info!(
            "launching {target}{}",
            if as_admin { " as admin" } else { "" }
        );
        shell
            .launch_app(target.to_owned(), as_admin)
            .await
            .map_err(launch_err)?;
        return Ok(Activation::Hide);
    }
    if let Some(path) = path_from_id(id) {
        match action {
            Action::RevealInExplorer => shell
                .reveal_path(path.to_owned())
                .await
                .map_err(launch_err)?,
            Action::CopyPath => {
                return Ok(Activation::Copy {
                    text: path.to_owned(),
                })
            }
            _ => shell.open_path(path.to_owned()).await.map_err(launch_err)?,
        }
        return Ok(Activation::Hide);
    }
    Err(AppError::UnknownItem(id.to_owned()))
}

pub fn run_builtin(app: &AppHandle, cmd: BuiltinCommand) -> AppResult<()> {
    tracing::info!("running {}", cmd.id());
    let state = app.try_state::<AppState>().ok_or(AppError::NotReady)?;
    match cmd {
        BuiltinCommand::OpenSettings => {
            // The settings menu lives in the launcher: show it there.
            window::show(app, std::time::Instant::now());
            if let Err(e) = crate::ipc::ShowSettings.emit(app) {
                tracing::warn!("could not open settings: {e}");
            }
            return Ok(());
        }
        BuiltinCommand::OpenSettingsFile => open_path(app, &state.paths.settings_file())?,
        BuiltinCommand::Restart => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                restart(&app).await;
            });
        }
        BuiltinCommand::OpenLogFolder => open_path(app, &state.paths.log_dir())?,
        BuiltinCommand::ReindexApps => {
            state.icons.clear();
            state.apps.refresh_in_background();
        }
        BuiltinCommand::SignInToClaude => {
            let mut login = crate::ai::claude::sign_in(&state.settings().ai.claude)
                .map_err(|e| AppError::Launch(e.to_string()))?;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = login.wait().await;
                crate::check_claude_status(&app, true);
            });
        }
        BuiltinCommand::InstallClaude => open_url(app, CLAUDE_INSTALL_PAGE)?,
        BuiltinCommand::SignInToChatgpt => {
            let mut login = crate::ai::codex::sign_in(&state.settings().ai.chatgpt)
                .map_err(|e| AppError::Launch(e.to_string()))?;
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = login.wait().await;
                crate::check_codex_status(&app, true);
            });
        }
        BuiltinCommand::InstallCodex => open_url(app, CODEX_INSTALL_PAGE)?,
        BuiltinCommand::Quit => app.exit(0),
    }
    window::hide(app);
    Ok(())
}

/// Restarts the launcher, closing Claude Code or Codex first so it isn't
/// left behind. A downloaded update is installed instead, which restarts
/// into the new version ("restart to install").
pub async fn restart(app: &AppHandle) {
    crate::features::updater::install_if_ready(app).await;
    if let Some(state) = app.try_state::<AppState>() {
        state.chat.shut_down().await;
    }
    app.restart();
}

fn open_path(app: &AppHandle, path: &Path) -> AppResult<()> {
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| AppError::Open {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
}
