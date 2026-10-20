//! Carrying out what the user picked. Shared by the results list and the tray.

use std::path::Path;

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::search::apps::target_from_id;
use crate::search::commands::BuiltinCommand;
use crate::search::Action;
use crate::state::AppState;
use crate::window;

/// Runs `action` on the result with `id`, records the open for ranking, and
/// hides the launcher.
pub async fn activate(app: &AppHandle, id: &str, action: Action) -> AppResult<()> {
    if let Some(cmd) = BuiltinCommand::from_id(id) {
        run_builtin(app, cmd)?;
    } else if let Some(target) = target_from_id(id) {
        let shell = app
            .try_state::<AppState>()
            .ok_or(AppError::NotReady)?
            .shell
            .clone();
        let as_admin = action == Action::RunAsAdmin;
        tracing::info!(
            "launching {target}{}",
            if as_admin { " as admin" } else { "" }
        );
        shell
            .launch_app(target.to_owned(), as_admin)
            .await
            .map_err(|e| AppError::Launch(e.to_string()))?;
        window::hide(app);
    } else {
        return Err(AppError::UnknownItem(id.to_owned()));
    }

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

fn open_path(app: &AppHandle, path: &Path) -> AppResult<()> {
    app.opener()
        .open_path(path.display().to_string(), None::<&str>)
        .map_err(|e| AppError::Open {
            path: path.display().to_string(),
            reason: e.to_string(),
        })
}
