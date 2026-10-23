//! Connections the launcher sets up for the AIs: the MCP registry (servers
//! you add once, given to Claude and ChatGPT), plus the settings menu's
//! commands for it and for API keys (`secrets.rs` keeps the values).
//!
//! - `mcp_registry.rs`: the settings (servers and their values) and the
//!   rules a server must follow (pure).
//! - `mcp_config.rs`: each AI's config from them (pure), and Claude Code's
//!   `--mcp-config` file.
//! - `mcp_edit.rs`: what saving or removing a server does in Credential
//!   Manager.
//! - `commands.rs`: the IPC commands.
//!
//! Claude Code gets the servers in the `--mcp-config` file it already reads
//! for the launcher's own server; Codex gets them in each thread's config
//! over stdin, like the launcher's. Each AI's own MCP servers and
//! connectors still load too.

pub mod commands;
pub mod mcp_config;
pub mod mcp_edit;
pub mod mcp_registry;

use std::path::{Path, PathBuf};

pub use mcp_config::{ResolvedLaunch, ResolvedServer};
pub use mcp_registry::{McpServer, McpSettings, McpTransport, McpValue};

use crate::mcp::Endpoint;
use crate::secrets;
use crate::state::AppState;

/// The registry's servers as the AIs get them now, with secrets read from
/// Credential Manager.
pub fn servers_now(settings: &McpSettings) -> Vec<ResolvedServer> {
    mcp_config::resolve(settings, secrets::read)
}

/// Claude Code's `--mcp-config` file in `dir`, brought up to date first.
/// `None` when there's nothing to give Claude (or no file to point at).
pub fn claude_config_file(
    dir: &Path,
    launcher: Option<&Endpoint>,
    servers: &[ResolvedServer],
) -> Option<PathBuf> {
    match mcp_config::write_claude_config(dir, launcher, servers) {
        Ok(path) => path,
        Err(e) => {
            crate::toast::report(format!("Couldn't update Claude's MCP server list: {e}"));
            // An older file still beats none.
            Some(dir.join(mcp_config::CLAUDE_CONFIG_FILE)).filter(|p| p.is_file())
        }
    }
}

/// Rewrites Claude Code's file from the current settings and secrets
/// (at startup, and when either changes). New conversations get it; one
/// that's running keeps what it started with.
pub fn refresh_claude_config(state: &AppState) {
    let servers = servers_now(&state.settings().mcp);
    let launcher = state.tool_files.get().map(|files| &files.endpoint);
    claude_config_file(&state.paths.data_dir, launcher, &servers);
}
