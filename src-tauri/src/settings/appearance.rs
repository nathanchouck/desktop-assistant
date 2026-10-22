//! How the launcher looks: theme variants on top of `tokens.css`.
//!
//! Rust only stores the choices. The UI turns them into CSS custom
//! properties on `:root` (`ui/src/lib/theme.ts`), so `tokens.css` stays the
//! one place the actual colors and sizes live.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AppearanceSettings {
    /// Accent color: a preset ("blue", "purple", "pink", "red", "orange",
    /// "green", "teal", "graphite") or a custom "#rrggbb". Empty or
    /// unknown means blue.
    pub tint: String,
    /// How much of the desktop shows through the window while the OS blur
    /// is on.
    pub glass: GlassStrength,
    /// How tall result rows are.
    pub density: Density,
}

impl Default for AppearanceSettings {
    fn default() -> Self {
        Self {
            tint: "blue".into(),
            glass: GlassStrength::Normal,
            density: Density::Comfortable,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum GlassStrength {
    /// Mostly solid: easiest to read over a busy wallpaper.
    Subtle,
    Normal,
    /// Mostly see-through.
    Strong,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum Density {
    Comfortable,
    /// Shorter rows and smaller icons.
    Compact,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;

    #[test]
    fn defaults_are_the_tokens_css_look() {
        let a = AppearanceSettings::default();
        assert_eq!(a.tint, "blue");
        assert_eq!(a.glass, GlassStrength::Normal);
        assert_eq!(a.density, Density::Comfortable);
    }

    #[test]
    fn older_files_and_toml_names() {
        // Files saved before theme variants have only `tint = ""`.
        let table: toml::Table = toml::from_str("tint = \"\"").unwrap();
        let mut full = toml::Table::try_from(Settings::default()).unwrap();
        full.insert("appearance".into(), {
            let mut a = toml::Table::try_from(AppearanceSettings::default()).unwrap();
            a.extend(table);
            toml::Value::Table(a)
        });
        let s: Settings = full.try_into().unwrap();
        assert_eq!(s.appearance.tint, "");
        assert_eq!(s.appearance.glass, GlassStrength::Normal);

        let text = toml::to_string(&AppearanceSettings {
            tint: "#12ab34".into(),
            glass: GlassStrength::Strong,
            density: Density::Compact,
        })
        .unwrap();
        assert!(text.contains("glass = \"strong\""), "{text}");
        assert!(text.contains("density = \"compact\""), "{text}");
    }
}
