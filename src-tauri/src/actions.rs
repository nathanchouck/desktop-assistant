//! Carrying out what the user picked. Shared by the results list and the tray.

use std::path::Path;

use tauri::{AppHandle, Manager};
use tauri_plugin_opener::OpenerExt;

use crate::error::{AppError, AppResult};
use crate::search::commands::BuiltinCommand;
use crate::state::AppState;
use crate::window;

pub fn run_builtin(app: &AppHandle, cmd: BuiltinCommand) -> AppResult<()> {
    tracing::info!("running {}", cmd.id());
    let state = app.try_state::<AppState>().ok_or(AppError::NotReady)?;
    match cmd {
        BuiltinCommand::OpenSettings => open_path(app, &state.paths.settings_file())?,
        BuiltinCommand::OpenLogFolder => open_path(app, &state.paths.log_dir())?,
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
