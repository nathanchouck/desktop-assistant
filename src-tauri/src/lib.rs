//! Desktop Assistant: a personal Raycast-style launcher for Windows.
//! See docs/ARCHITECTURE.md for how the pieces fit together.

pub mod actions;
pub mod ai;
pub mod error;
pub mod fuzzy;
pub mod hotkey;
pub mod icons;
pub mod ipc;
pub mod logging;
pub mod paths;
pub mod platform;
pub mod search;
pub mod settings;
pub mod state;
pub mod store;
pub mod window;

use std::sync::Arc;
use std::time::Instant;

use tauri::{AppHandle, Manager};

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

    let state = AppState::new(paths, settings.clone());

    tauri::Builder::default()
        // Must be first: a second launch just shows the running launcher.
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            window::show(app, Instant::now());
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(hotkey::shortcut::plugin())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .invoke_handler(builder.invoke_handler())
        // App icons for the UI: <img src={convertFileSrc(key, "icon")}>.
        .register_asynchronous_uri_scheme_protocol("icon", |ctx, request, responder| {
            let app = ctx.app_handle().clone();
            let key = request.uri().path().trim_start_matches('/').to_owned();
            tauri::async_runtime::spawn(async move {
                let icons = app.try_state::<AppState>().map(|s| Arc::clone(&s.icons));
                let png = match icons {
                    Some(icons) => icons.png(&key).await,
                    None => None,
                };
                responder.respond(icons::protocol_response(png));
            });
        })
        .setup(move |app| {
            builder.mount_events(app);
            app.manage(state);
            let handle = app.handle();

            let glass = window::apply_glass(handle);
            tracing::info!("glass effect {}", if glass { "on" } else { "off" });

            if let Err(e) = window::tray::create(handle) {
                tracing::warn!("could not create tray icon: {e}");
            }
            hotkey::init(handle, &settings.hotkeys);
            sync_launch_at_login(handle, settings.general.launch_at_login);
            start_app_index(handle);

            // Handy while developing; release builds start hidden in the tray.
            if cfg!(debug_assertions) {
                window::show(handle, Instant::now());
            }
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .run(tauri::generate_context!())
        .expect("error while running Desktop Assistant");
}

/// Indexes installed apps in the background and re-indexes when the Start
/// Menu changes.
fn start_app_index(app: &AppHandle) {
    let state = app.state::<AppState>();
    state.apps.refresh_in_background();
    let watcher = search::apps::watch_start_menu(Arc::clone(&state.apps));
    match state.watcher.lock() {
        Ok(mut slot) => *slot = watcher,
        Err(_) => tracing::warn!("could not keep the Start Menu watcher"),
    };
}

/// Makes "start with Windows" match the setting. Skipped in debug builds so
/// a dev binary never gets registered to run at login.
fn sync_launch_at_login(app: &AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;

    if cfg!(debug_assertions) {
        return;
    }
    let autostart = app.autolaunch();
    let result = match autostart.is_enabled() {
        Ok(current) if current == enabled => Ok(()),
        Ok(_) if enabled => autostart.enable(),
        Ok(_) => autostart.disable(),
        Err(e) => Err(e),
    };
    if let Err(e) = result {
        tracing::warn!("could not update launch at login: {e}");
    }
}
