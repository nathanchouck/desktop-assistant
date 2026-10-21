//! Connections the launcher sets up for the AIs: the MCP server registry
//! (servers you add once, given to Claude and ChatGPT).
//!
//! Not built yet.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct McpSettings {
    pub enabled: bool,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self { enabled: true }
    }
}
