//! The tools the launcher offers Claude over MCP.
//!
//! Reading tools (desktop context, search) are pre-approved; opening,
//! revealing and the screenshot go through the usual Approve / Deny card,
//! and the screenshot asks even in auto mode (see `claude_settings`).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use base64::Engine;
use serde_json::{json, Value};
use tauri::{AppHandle, Manager};

use super::protocol::{Content, ToolHost, ToolInfo, ToolOutput, SERVER_NAME};
use crate::ai::ProviderId;
use crate::icons::{encode_png, fit_within};
use crate::platform::PlatformError;
use crate::search::apps::target_from_id;
use crate::search::files::path_from_id;
use crate::search::{AskRow, Frecency, ItemKind, ResultItem};
use crate::state::AppState;

/// Screenshots are scaled down to this on their long side (what the model
/// reads best).
const SCREENSHOT_MAX_SIDE: u32 = 1568;
const SEARCH_DEFAULT: usize = 15;
const SEARCH_MAX: usize = 50;

const CONTEXT: &str = "get_desktop_context";
const SEARCH: &str = "search";
const OPEN: &str = "open";
const REVEAL: &str = "reveal";
const SCREENSHOT: &str = "screenshot";

/// The tool's full name as Claude Code knows it.
pub fn qualified(tool: &str) -> String {
    format!("mcp__{SERVER_NAME}__{tool}")
}

/// Extra Claude Code settings for the launcher's tools: reading tools need
/// no approval; a screenshot is always asked for, even in auto mode.
pub fn claude_settings() -> Value {
    json!({
        "permissions": {
            "allow": [qualified(CONTEXT), qualified(SEARCH)],
            "ask": [qualified(SCREENSHOT)],
        }
    })
}

pub struct LauncherTools {
    app: AppHandle,
}

impl LauncherTools {
    pub fn new(app: AppHandle) -> Self {
        Self { app }
    }

    fn state(&self) -> Option<tauri::State<'_, AppState>> {
        self.app.try_state::<AppState>()
    }

    async fn desktop_context(&self, state: &AppState) -> ToolOutput {
        if !state.settings().ai.claude.desktop_context {
            return ToolOutput::error("The user turned desktop context off in settings.");
        }
        let context = state.desktop.refreshed(&state.shell).await;
        match context.prompt_block() {
            Some(block) => ToolOutput::text(block),
            None => {
                ToolOutput::text("Nothing captured: the launcher hasn't seen another window yet.")
            }
        }
    }

    async fn search(&self, state: &AppState, args: &Value) -> ToolOutput {
        let Some(query) = args["query"].as_str().filter(|q| !q.trim().is_empty()) else {
            return ToolOutput::error("query is required");
        };
        let limit = args["limit"]
            .as_u64()
            .map_or(SEARCH_DEFAULT, |n| n as usize)
            .clamp(1, SEARCH_MAX);
        let store = Arc::clone(&state.store);
        let frecency: Frecency = Arc::new(move |id| store.frecency(id));
        let outcome = state
            .search
            .search(query, limit, AskRow::Ask(ProviderId::Claude), frecency)
            .await;
        // Claude can wait for the slower sources; the UI can't.
        let items = match outcome.complete {
            Some(complete) => complete.await,
            None => outcome.items,
        };
        ToolOutput::text(format_results(&items))
    }

    async fn open(&self, state: &AppState, args: &Value, reveal: bool) -> ToolOutput {
        let Some(target) = args["target"].as_str().map(str::trim) else {
            return ToolOutput::error("target is required");
        };
        let result = if let Some(app) = target_from_id(target).filter(|_| !reveal) {
            state.shell.launch_app(app.to_owned(), false).await
        } else {
            let path = path_from_id(target).unwrap_or(target);
            if !Path::new(path).exists() {
                return ToolOutput::error(format!("{path} doesn't exist"));
            }
            if reveal {
                state.shell.reveal_path(path.to_owned()).await
            } else {
                state.shell.open_path(path.to_owned()).await
            }
        };
        match result {
            Ok(()) if reveal => ToolOutput::text(format!("Showed {target} in File Explorer.")),
            Ok(()) => ToolOutput::text(format!("Opened {target}.")),
            Err(e) => ToolOutput::error(e.to_string()),
        }
    }

    async fn screenshot(&self, state: &AppState) -> ToolOutput {
        if !state.settings().ai.claude.desktop_context {
            return ToolOutput::error("The user turned desktop context off in settings.");
        }
        let context = state.desktop.current(std::time::Duration::ZERO).await;
        let Some(window) = context.window else {
            return ToolOutput::error("No window to capture: the launcher hasn't seen one yet.");
        };
        let image = match state.shell.capture_window(window.handle).await {
            Ok(image) => fit_within(image, SCREENSHOT_MAX_SIDE),
            Err(PlatformError::Unsupported) => {
                return ToolOutput::error("Screenshots only work on Windows.")
            }
            Err(e) => return ToolOutput::error(format!("Couldn't capture the window: {e}")),
        };
        let png = match encode_png(&image) {
            Ok(png) => png,
            Err(e) => return ToolOutput::error(format!("Couldn't encode the screenshot: {e}")),
        };
        ToolOutput {
            content: vec![
                Content::Text(format!(
                    "Screenshot of \"{}\" ({})",
                    window.title, window.app
                )),
                Content::Png(base64::engine::general_purpose::STANDARD.encode(png)),
            ],
            is_error: false,
        }
    }
}

#[async_trait]
impl ToolHost for LauncherTools {
    fn tools(&self) -> Vec<ToolInfo> {
        vec![
            ToolInfo {
                name: CONTEXT,
                description: "What's on the user's screen, read fresh: the window they were in \
                    when they opened the launcher, and for File Explorer or the desktop the \
                    folder shown and the selected files. Use it when they say \"this\", \
                    \"these\", \"here\" or \"the selected files\".",
                input_schema: json!({ "type": "object", "properties": {} }),
            },
            ToolInfo {
                name: SEARCH,
                description: "Find the user's apps, files and folders by name with the \
                    launcher's index: instant, covers their user folder (every drive when \
                    Everything is running). Fuzzy, so partial names work. `ext:pdf` limits \
                    the type; a leading `f ` means files only, `d ` folders only. Prefer this \
                    to listing directories when looking for something by name.",
                input_schema: json!({
                    "type": "object",
                    "properties": {
                        "query": { "type": "string", "description": "Part of the name, e.g. \"tax 2024 ext:pdf\"" },
                        "limit": { "type": "integer", "minimum": 1, "maximum": SEARCH_MAX, "default": SEARCH_DEFAULT },
                    },
                    "required": ["query"],
                }),
            },
            ToolInfo {
                name: OPEN,
                description: "Open a file or folder with its default app, or start an app, \
                    on the user's screen. `target` is a path, or an app id (app:…) from search.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "target": { "type": "string" } },
                    "required": ["target"],
                }),
            },
            ToolInfo {
                name: REVEAL,
                description: "Open File Explorer with this file or folder selected, to show \
                    the user where something is.",
                input_schema: json!({
                    "type": "object",
                    "properties": { "target": { "type": "string", "description": "A path" } },
                    "required": ["target"],
                }),
            },
            ToolInfo {
                name: SCREENSHOT,
                description: "A screenshot of the window the user was in when they opened the \
                    launcher (never the launcher itself). Only when you need to see their \
                    screen to help; they're asked to approve each one.",
                input_schema: json!({ "type": "object", "properties": {} }),
            },
        ]
    }

    async fn call(&self, name: &str, args: Value) -> ToolOutput {
        let Some(state) = self.state() else {
            return ToolOutput::error("The launcher is still starting.");
        };
        match name {
            CONTEXT => self.desktop_context(&state).await,
            SEARCH => self.search(&state, &args).await,
            OPEN => self.open(&state, &args, false).await,
            REVEAL => self.open(&state, &args, true).await,
            SCREENSHOT => self.screenshot(&state).await,
            other => ToolOutput::error(format!("unknown tool {other}")),
        }
    }
}

/// One line per result, the way Claude can act on it.
fn format_results(items: &[ResultItem]) -> String {
    let lines: Vec<String> = items
        .iter()
        .filter_map(|item| match item.kind {
            ItemKind::File | ItemKind::Folder => {
                let kind = if item.kind == ItemKind::Folder {
                    "folder"
                } else {
                    "file"
                };
                path_from_id(&item.id).map(|path| format!("{kind}: {path}"))
            }
            ItemKind::App => Some(format!("app: {} (id: {})", item.title, item.id)),
            // Claude has its own ways to calculate, and the rest only make
            // sense in the launcher.
            _ => None,
        })
        .collect();
    if lines.is_empty() {
        "No matches.".to_owned()
    } else {
        lines.join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::Action;

    fn item(id: &str, kind: ItemKind, title: &str) -> ResultItem {
        ResultItem {
            id: id.into(),
            kind,
            title: title.into(),
            subtitle: String::new(),
            icon: None,
            score: 0.0,
            actions: vec![Action::Open],
            accessory: None,
            confirm: None,
        }
    }

    #[test]
    fn lists_results_claude_can_act_on() {
        let items = [
            item(
                r"file:C:\Users\me\tax-2024.pdf",
                ItemKind::File,
                "tax-2024.pdf",
            ),
            item(r"file:C:\Users\me\Taxes", ItemKind::Folder, "Taxes"),
            item(
                "app:Microsoft.WindowsCalculator!App",
                ItemKind::App,
                "Calculator",
            ),
            item("ask-ai", ItemKind::AskAi, "Ask Claude: tax"),
            item("cmd:settings", ItemKind::Command, "Open Settings"),
        ];
        assert_eq!(
            format_results(&items),
            "file: C:\\Users\\me\\tax-2024.pdf\nfolder: C:\\Users\\me\\Taxes\n\
             app: Calculator (id: app:Microsoft.WindowsCalculator!App)"
        );
        assert_eq!(format_results(&[]), "No matches.");
    }

    #[test]
    fn screenshots_always_ask() {
        let settings = claude_settings();
        assert_eq!(
            settings["permissions"]["ask"][0],
            "mcp__launcher__screenshot"
        );
        let allowed = settings["permissions"]["allow"].as_array().unwrap();
        assert!(allowed.iter().all(|t| t != "mcp__launcher__open"));
    }
}
