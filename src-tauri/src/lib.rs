//! Desktop Assistant: a personal Raycast-style launcher for Windows.
//! See docs/ARCHITECTURE.md for how the pieces fit together.

pub mod ai;
pub mod error;
pub mod ipc;
pub mod logging;
pub mod paths;
pub mod search;
pub mod settings;
pub mod state;

use tauri::Manager;

use crate::paths::AppPaths;
use crate::settings::Settings;
use crate::state::AppState;

pub fn run() {
    // Without a home directory there is nowhere to keep settings or logs.
    let paths = AppPaths::resolve().expect("could not resolve app directories");
    let _log_guard = logging::init(&paths.log_dir());
    tracing::info!("starting v{}", env!("CARGO_PKG_VERSION"));

    let settings = settings::load_or_create(&paths.settings_file()).unwrap_or_else(|e| {
        tracing::error!("{e}; using default settings");
        Settings::default()
    });

    let builder = ipc::builder();

    #[cfg(debug_assertions)]
    if let Err(e) = ipc::export_bindings(&ipc::bindings_path()) {
        tracing::warn!("could not export TypeScript bindings: {e}");
    }

    let state = AppState::new(paths, settings);

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(builder.invoke_handler())
        .setup(move |app| {
            builder.mount_events(app);
            app.manage(state);
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running Desktop Assistant");
}
