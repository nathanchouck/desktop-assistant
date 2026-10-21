//! Desktop Assistant: a personal Raycast-style launcher for Windows.
//! See docs/ARCHITECTURE.md for how the pieces fit together.

pub mod actions;
pub mod ai;
pub mod desktop;
pub mod error;
pub mod file_index;
pub mod fuzzy;
pub mod hotkey;
pub mod icons;
pub mod ipc;
pub mod logging;
pub mod mcp;
pub mod paths;
pub mod platform;
pub mod search;
pub mod settings;
pub mod state;
pub mod store;
pub mod window;

use std::sync::Arc;
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, RunEvent};
use tauri_specta::Event;

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
            start_file_index_unless_everything(handle);
            start_chat(handle);
            start_launcher_tools(handle);
            check_claude_status(handle, true);

            // Handy while developing; release builds start hidden in the tray.
            if cfg!(debug_assertions) {
                window::show(handle, Instant::now());
            }
            Ok(())
        })
        .on_window_event(window::on_window_event)
        .build(tauri::generate_context!())
        .expect("error while building Desktop Assistant")
        .run(|app, event| {
            if let RunEvent::Exit = event {
                // Don't leave a Claude Code process behind.
                if let Some(state) = app.try_state::<AppState>() {
                    tauri::async_runtime::block_on(state.chat.close());
                }
            }
        });
}

/// Checks in the background whether Claude Code is installed and signed in,
/// if the last check is stale (or `force`). Search shows a fix-it row
/// instead of "Ask Claude" while it isn't.
pub(crate) fn check_claude_status(app: &AppHandle, force: bool) {
    let state = app.state::<AppState>();
    let cache = Arc::clone(&state.claude_status);
    if !(force || cache.is_stale()) || !cache.begin_check() {
        return;
    }
    let settings = state.settings().ai.claude;
    tauri::async_runtime::spawn(async move {
        let status = ai::claude::check_status(&settings).await;
        tracing::info!("Claude Code status: {status:?}");
        cache.finish_check(status);
    });
}

/// Starts the MCP server that gives Claude the launcher's tools, and writes
/// the files that point Claude Code at it.
fn start_launcher_tools(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let host = Arc::new(mcp::tools::LauncherTools::new(app.clone()));
        let endpoint = match mcp::start(host).await {
            Ok(endpoint) => endpoint,
            Err(e) => {
                tracing::warn!("launcher tools for Claude are off: {e}");
                return;
            }
        };
        let state = app.state::<AppState>();
        let dir = state.paths.data_dir.clone();
        let written = endpoint.write_config(&dir).and_then(|mcp_config| {
            let settings = dir.join("claude-settings.json");
            std::fs::write(&settings, mcp::tools::claude_settings().to_string())?;
            Ok(state::ToolFiles {
                mcp_config,
                settings,
            })
        });
        match written {
            Ok(files) => {
                let _ = state.tool_files.set(files);
            }
            Err(e) => tracing::warn!("could not write the launcher tools config: {e}"),
        }
    });
}

/// How long an unused conversation keeps its Claude Code process.
const CHAT_IDLE: Duration = Duration::from_secs(10 * 60);

/// Forwards chat events to the UI and frees idle Claude Code processes.
fn start_chat(app: &AppHandle) {
    let state = app.state::<AppState>();
    let events = state.chat_events.lock().ok().and_then(|mut rx| rx.take());
    if let Some(mut events) = events {
        let app = app.clone();
        tauri::async_runtime::spawn(async move {
            while let Some((conversation, event)) = events.recv().await {
                // An error may mean Claude Code was signed out or removed.
                if matches!(event, ai::AiEvent::Error { .. }) {
                    check_claude_status(&app, true);
                }
                if let Err(e) = (ipc::AiStream {
                    conversation,
                    event,
                })
                .emit(&app)
                {
                    tracing::warn!("could not send a chat event: {e}");
                }
            }
        });
    }
    let chat = Arc::clone(&state.chat);
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            chat.shut_down_if_idle(CHAT_IDLE).await;
        }
    });
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

/// Builds the built-in file index right away unless Everything is running
/// (then it starts later, only if a search finds Everything gone).
fn start_file_index_unless_everything(app: &AppHandle) {
    let state = app.state::<AppState>();
    let shell = Arc::clone(&state.shell);
    let files = Arc::clone(&state.files);
    // Progress for the footer while the index loads or scans.
    let handle = app.clone();
    files.on_status(move |status| {
        if let Err(e) = (ipc::FileIndexChanged { status }).emit(&handle) {
            tracing::debug!("could not send file index status: {e}");
        }
    });
    tauri::async_runtime::spawn(async move {
        if shell.everything_running().await {
            tracing::info!("Everything is running; using it for file search");
        } else {
            files.ensure_started();
        }
    });
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
