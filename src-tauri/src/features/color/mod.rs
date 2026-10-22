//! Color picker: "Pick Color" hides the launcher, you click anywhere on
//! screen (Enter picks under the cursor, Esc or a right click cancels),
//! and the color is copied as hex, rgb() or hsl(). The last 20 colors are
//! listed by `color`/`colors`; a typed color (`#ff8800`, `rgb(1, 2, 3)`,
//! `hsl(...)`) shows its other formats.
//!
//! - `model.rs`: reading and writing colors (pure).
//! - `recent.rs`: the picked colors, in `colors.json`.
//! - `picking.rs`: what each click and key does while picking (pure).
//! - `provider.rs`: search rows and their actions.
//! - `win.rs` (Windows): the hooks, the crosshair and the screen pixel;
//!   `fallback.rs` elsewhere.

#[cfg(not(windows))]
mod fallback;
mod model;
// Fed by the hooks in `win.rs`; elsewhere only its tests use it.
#[cfg_attr(not(windows), allow(dead_code))]
mod picking;
mod provider;
mod recent;
#[cfg(windows)]
mod win;

#[cfg(not(windows))]
use fallback as os;
#[cfg(windows)]
use win as os;

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tauri_specta::Event;

use super::ProviderContext;
use crate::paths::AppPaths;
use crate::search::SearchProvider;
use crate::settings::{self, SharedSettings};
use crate::state::AppState;
pub use model::ColorFormat;
use model::Rgb;
use recent::Recent;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct ColorSettings {
    /// "Pick Color" and colors in search.
    pub enabled: bool,
    /// How colors are copied: `#ff8800`, `rgb(255, 136, 0)` or
    /// `hsl(32, 100%, 50%)`.
    pub format: ColorFormat,
    /// After a pick, open the launcher on your recent colors (the new one
    /// first). Off: the color is only copied.
    pub show_after_pick: bool,
}

impl Default for ColorSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            format: ColorFormat::Hex,
            show_after_pick: true,
        }
    }
}

/// A color was picked and the launcher opened to show it: the UI lists
/// recent colors (`colors`).
#[derive(Debug, Clone, Serialize, Deserialize, specta::Type, tauri_specta::Event)]
pub struct ColorPicked {
    /// `#rrggbb`.
    pub hex: String,
}

/// Between hiding the launcher and starting to pick, so the click that
/// counts can't land on it.
const HIDE_DELAY: Duration = Duration::from_millis(150);
/// Picking stops by itself after this, so a forgotten pick doesn't keep
/// eating clicks.
const PICK_TIMEOUT: Duration = Duration::from_secs(30);

/// For copying and toasts from the picker thread (providers are made
/// before the app runs).
static APP: OnceLock<AppHandle> = OnceLock::new();
/// One pick at a time.
static PICKING: AtomicBool = AtomicBool::new(false);
/// Shared by the provider and the picker.
static RECENT: OnceLock<Arc<Mutex<Recent>>> = OnceLock::new();

fn recent(paths: &AppPaths) -> Arc<Mutex<Recent>> {
    Arc::clone(
        RECENT
            .get_or_init(|| Arc::new(Mutex::new(Recent::load(paths.data_dir.join("colors.json"))))),
    )
}

/// Notes that the system cursors are replaced (see `win.rs`).
fn cursor_marker(paths: &AppPaths) -> PathBuf {
    paths.data_dir.join("color-picker-cursor")
}

/// Color rows in search.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    Some(Arc::new(provider::ColorProvider::new(
        Arc::clone(&ctx.settings),
        recent(&ctx.paths),
        cursor_marker(&ctx.paths),
    )))
}

/// Puts the cursors back if the launcher stopped mid-pick last time.
pub fn start(app: &AppHandle) {
    let _ = APP.set(app.clone());
    if let Some(state) = app.try_state::<AppState>() {
        os::recover(&cursor_marker(&state.paths));
    }
}

/// Starts picking on its own thread once the launcher has hidden; the
/// result is copied, remembered and announced from there.
fn start_pick(
    settings: SharedSettings,
    recent: Arc<Mutex<Recent>>,
    marker: PathBuf,
) -> Result<(), String> {
    if !os::SUPPORTED {
        return Err("The color picker works only on Windows.".into());
    }
    if PICKING.swap(true, Ordering::AcqRel) {
        return Err("Already picking: click anywhere on screen, or press Esc.".into());
    }
    let started = std::thread::Builder::new()
        .name("color-picker".into())
        .spawn(move || {
            std::thread::sleep(HIDE_DELAY);
            let picked = os::pick(PICK_TIMEOUT, &marker);
            PICKING.store(false, Ordering::Release);
            match picked {
                Ok(Some(color)) => finish(color, &settings, &recent),
                Ok(None) => tracing::debug!("color pick cancelled"),
                Err(e) => {
                    if let Some(app) = APP.get() {
                        crate::toast::error(app, format!("Couldn't pick a color: {e}"));
                    }
                }
            }
        });
    started.map(|_| ()).map_err(|e| {
        PICKING.store(false, Ordering::Release);
        e.to_string()
    })
}

/// Copies the color, remembers it, and shows it.
fn finish(color: Rgb, settings: &SharedSettings, recent: &Mutex<Recent>) {
    let options = settings::current(settings).color_picker;
    let text = color.format(options.format);
    {
        let mut recent = match recent.lock() {
            Ok(r) => r,
            Err(poisoned) => poisoned.into_inner(),
        };
        recent.add(color, now_ms());
        if let Err(e) = recent.save() {
            tracing::warn!("could not save recent colors: {e}");
        }
    }
    let Some(app) = APP.get() else { return };
    if let Err(e) = crate::actions::copy_text(app, &text) {
        crate::toast::error(app, format!("Couldn't copy {text}: {e}"));
        return;
    }
    if options.show_after_pick {
        crate::window::show(app, Instant::now());
        if let Err(e) = (ColorPicked { hex: color.hex() }).emit(app) {
            tracing::debug!("could not show the picked color: {e}");
        }
    }
    crate::toast::info(app, format!("Copied {text}"));
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_copy_hex_and_show_the_pick() {
        let s = ColorSettings::default();
        assert!(s.enabled && s.show_after_pick);
        assert_eq!(s.format, ColorFormat::Hex);
    }

    #[test]
    fn older_settings_files_get_the_new_fields() {
        let mut merged = toml::Table::try_from(ColorSettings::default()).unwrap();
        merged.extend("enabled = false\n".parse::<toml::Table>().unwrap());
        let s: ColorSettings = merged.try_into().unwrap();
        assert!(!s.enabled && s.show_after_pick);
        let rgb: ColorSettings =
            toml::from_str("enabled = true\nformat = \"rgb\"\nshow_after_pick = false\n").unwrap();
        assert_eq!(rgb.format, ColorFormat::Rgb);
    }

    #[test]
    fn picking_is_windows_only_here() {
        if !os::SUPPORTED {
            let r = start_pick(
                Arc::new(std::sync::RwLock::new(crate::settings::Settings::default())),
                Arc::new(Mutex::new(Recent::default())),
                PathBuf::from("unused"),
            );
            assert!(r.is_err());
        }
    }
}
