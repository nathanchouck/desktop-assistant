//! How the launcher looks: theme variants on top of `tokens.css`.
//!
//! Not built yet.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AppearanceSettings {
    /// Empty: the default look.
    pub tint: String,
}
