//! Carrying out what the user picked. Shared by the results list and the tray.

use std::path::Path;

use tauri::{AppHandle, Manager};
use tauri_plugin_clipboard_manager::ClipboardExt;
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::platform::{self, PlatformError};
use crate::search::apps::target_from_id;
use crate::search::commands::BuiltinCommand;
use crate::search::files::{self, path_from_id};
use crate::search::Action;
use crate::state::AppState;
use crate::window;

/// Runs `action` on the result with `id`, records the open for ranking, and
/// hides the launcher.
pub async fn activate(app: &AppHandle, id: &str, action: Action) -> AppResult<()> {
    let shell = app
        .try_state::<AppState>()
        .ok_or(AppError::NotReady)?
        .shell
        .clone();
    let launch_err = |e: PlatformError| AppError::Launch(e.to_string());

    if let Some(cmd) = BuiltinCommand::from_id(id) {
        run_builtin(app, cmd)?;
    } else if let Some(target) = target_from_id(id) {
        let as_admin = action == Action::RunAsAdmin;
        tracing::info!(
            "launching {target}{}",
            if as_admin { " as admin" } else { "" }
        );
        shell
            .launch_app(target.to_owned(), as_admin)
            .await
            .map_err(launch_err)?;
    } else if let Some(path) = path_from_id(id) {
        match action {
            Action::RevealInExplorer => shell
                .reveal_path(path.to_owned())
                .await
                .map_err(launch_err)?,
            Action::CopyPath => app
                .clipboard()
                .write_text(path)
                .map_err(|e| AppError::Clipboard(e.to_string()))?,
            _ => shell.open_path(path.to_owned()).await.map_err(launch_err)?,
        }
    } else if id == files::SETUP_START_ID || id == files::SETUP_INSTALL_ID {
        set_up_everything(app, id)?;
        window::hide(app);
        // Setup rows aren't worth remembering for ranking.
        return Ok(());
    } else {
        return Err(AppError::UnknownItem(id.to_owned()));
    }
    window::hide(app);

    if let Some(state) = app.try_state::<AppState>() {
        if let Err(e) = state.store.record_open(id) {
            tracing::warn!("could not record open: {e}");
        }
    }
    Ok(())
}

pub fn run_builtin(app: &AppHandle, cmd: BuiltinCommand) -> AppResult<()> {
    tracing::info!("running {}", cmd.id());
    let state = app.try_state::<AppState>().ok_or(AppError::NotReady)?;
    match cmd {
        BuiltinCommand::OpenSettings => open_path(app, &state.paths.settings_file())?,
        BuiltinCommand::OpenLogFolder => open_path(app, &state.paths.log_dir())?,
        BuiltinCommand::ReindexApps => {
            state.icons.clear();
            state.apps.refresh_in_background();
        }
        BuiltinCommand::Quit => app.exit(0),
    }
    window::hide(app);
    Ok(())
}

/// Starts Everything (it then lives in the tray), or opens its download page.
fn set_up_everything(app: &AppHandle, id: &str) -> AppResult<()> {
    if id == files::SETUP_START_ID {
        if let Some(exe) = platform::everything_exe() {
            tracing::info!("starting {}", exe.display());
            return std::process::Command::new(&exe)
                .arg("-startup")
                .spawn()
                .map(drop)
                .map_err(|e| AppError::Launch(format!("could not start Everything: {e}")));
        }
    }
    app.opener()
        .open_url(files::EVERYTHING_DOWNLOAD_URL, None::<&str>)
        .map_err(|e| AppError::Open {
            path: files::EVERYTHING_DOWNLOAD_URL.into(),
            reason: e.to_string(),
        })
}

fn open_path(app: &AppHandle, path: &Path) -> AppResult<()> {
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| AppError::Open {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
}
