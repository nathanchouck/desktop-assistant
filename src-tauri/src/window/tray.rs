//! Tray icon: left-click opens the launcher; the menu has the built-in commands.

use std::time::Instant;

use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::AppHandle;

use crate::actions;
use crate::search::commands::BuiltinCommand;

const OPEN_ID: &str = "open";

pub fn create(app: &AppHandle) -> tauri::Result<()> {
    let item = |id: &str, title: &str| MenuItem::with_id(app, id, title, true, None::<&str>);
    let menu = Menu::with_items(
        app,
        &[
            &item(OPEN_ID, "Open Launcher")?,
            &PredefinedMenuItem::separator(app)?,
            &item(BuiltinCommand::OpenSettings.id(), "Settings")?,
            &item(BuiltinCommand::OpenLogFolder.id(), "Open Log Folder")?,
            &item(BuiltinCommand::ReindexApps.id(), "Reindex Apps")?,
            &PredefinedMenuItem::separator(app)?,
            &item(BuiltinCommand::Quit.id(), "Quit")?,
        ],
    )?;

    let mut builder = TrayIconBuilder::with_id("main")
        .tooltip("Desktop Assistant")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| on_menu(app, event.id().as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                super::show(tray.app_handle(), Instant::now());
            }
        });
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder.build(app)?;
    Ok(())
}

fn on_menu(app: &AppHandle, id: &str) {
    if id == OPEN_ID {
        super::show(app, Instant::now());
    } else if let Some(cmd) = BuiltinCommand::from_id(id) {
        if let Err(e) = actions::run_builtin(app, cmd) {
            tracing::warn!("{e}");
        }
    }
}
