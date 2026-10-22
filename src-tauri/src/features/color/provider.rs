//! Color rows: "Pick Color", the colors you picked (`color`, `colors`),
//! and a typed color's other formats (`#ff8800` → rgb() and hsl()).

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use super::model::{self, ColorFormat, Rgb};
use super::recent::Recent;
use crate::features::clipboard::content::age;
use crate::features::system::matching;
use crate::fuzzy::FuzzyQuery;
use crate::search::{
    color_icon, glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider,
};
use crate::settings::{self, SharedSettings};

const NAME: &str = "color";
const PICK_ID: &str = "color:pick";
const RECENT_PREFIX: &str = "color:recent:";
const VALUE_PREFIX: &str = "color:value:";
const PICK_TITLE: &str = "Pick Color";
const PICK_KEYWORDS: &[&str] = &[
    "color picker",
    "colour picker",
    "pick colour",
    "eyedropper",
    "eye dropper",
    "screen color",
    "colors",
];
/// A typed color's conversions rank in this order (the ranker puts
/// answers first anyway).
const VALUE_SCORE: f32 = 0.9;
/// Recent colors start here and step down, newest first.
const RECENT_SCORE: f32 = 0.6;
const RECENT_STEP: f32 = 0.01;

/// Queries that list the colors you picked.
fn lists_recent(query: &str) -> bool {
    ["color", "colors", "colour", "colours"]
        .iter()
        .any(|w| query.eq_ignore_ascii_case(w))
}

pub struct ColorProvider {
    settings: SharedSettings,
    recent: Arc<Mutex<Recent>>,
    /// Marks replaced cursors while picking (see `win.rs`).
    marker: PathBuf,
}

impl ColorProvider {
    pub fn new(settings: SharedSettings, recent: Arc<Mutex<Recent>>, marker: PathBuf) -> Self {
        Self {
            settings,
            recent,
            marker,
        }
    }

    fn rows(&self, query: &str, now_ms: i64) -> Vec<ResultItem> {
        let format = settings::current(&self.settings).color_picker.format;
        if let Some((color, typed)) = model::parse(query) {
            return value_rows(color, typed, query);
        }
        let mut rows = Vec::new();
        if matching::long_enough(query) {
            let mut fuzzy = FuzzyQuery::new(query);
            if let Some(score) = matching::score(&mut fuzzy, PICK_TITLE, PICK_KEYWORDS) {
                rows.push(pick_row().with_score(score));
            }
        }
        if lists_recent(query) {
            let recent = match self.recent.lock() {
                Ok(r) => r,
                Err(poisoned) => poisoned.into_inner(),
            };
            rows.extend(recent.colors().iter().enumerate().map(|(i, p)| {
                recent_row(p.color, format)
                    .with_accessory(age(now_ms, p.at))
                    .with_score(RECENT_SCORE - i as f32 * RECENT_STEP)
            }));
        }
        rows
    }
}

fn pick_row() -> ResultItem {
    ResultItem::new(PICK_ID, ItemKind::Command, PICK_TITLE)
        .with_subtitle("Click anywhere on screen to copy its color · Esc cancels")
        .with_icon(glyph("eyedropper"))
        .with_actions([Action::Run])
}

/// A picked color in your format, the others in the subtitle.
fn recent_row(color: Rgb, format: ColorFormat) -> ResultItem {
    let others: Vec<String> = ColorFormat::ALL
        .into_iter()
        .filter(|&f| f != format)
        .map(|f| color.format(f))
        .collect();
    ResultItem::new(
        format!("{RECENT_PREFIX}{}", color.hex()),
        ItemKind::Command,
        color.format(format),
    )
    .with_subtitle(others.join(" · "))
    .with_icon(color_icon(&color.hex()))
    .with_actions([Action::Copy, Action::Paste, Action::Delete])
}

/// The typed color in each format but the one you typed.
fn value_rows(color: Rgb, typed: ColorFormat, query: &str) -> Vec<ResultItem> {
    let plain = |s: &str| s.replace(' ', "").to_ascii_lowercase();
    ColorFormat::ALL
        .into_iter()
        .enumerate()
        .filter_map(|(i, format)| {
            let text = color.format(format);
            if format == typed && plain(&text) == plain(query) {
                return None;
            }
            Some(
                ResultItem::new(
                    format!("{VALUE_PREFIX}{}:{}", slug(format), color.hex()),
                    ItemKind::Answer,
                    text,
                )
                .with_subtitle(format!("{} · Enter copies", format.label()))
                .with_icon(color_icon(&color.hex()))
                .with_score(VALUE_SCORE - i as f32 * RECENT_STEP)
                .with_actions([Action::Copy, Action::Paste]),
            )
        })
        .collect()
}

fn slug(format: ColorFormat) -> &'static str {
    match format {
        ColorFormat::Hex => "hex",
        ColorFormat::Rgb => "rgb",
        ColorFormat::Hsl => "hsl",
    }
}

/// The color and format a value row's id names.
fn value_from_id(rest: &str) -> Option<(Rgb, ColorFormat)> {
    let (format, hex) = rest.split_once(':')?;
    let format = ColorFormat::ALL.into_iter().find(|&f| slug(f) == format)?;
    Some((model::from_hex(hex)?, format))
}

fn unknown(id: &str) -> SearchError {
    SearchError::Provider {
        provider: NAME,
        message: format!("{id} is unknown"),
    }
}

/// Copy or paste `text`, as the action asks.
fn deliver(text: String, action: Action) -> Activation {
    if action == Action::Paste {
        Activation::Paste { text: Some(text) }
    } else {
        Activation::Copy { text }
    }
}

#[async_trait]
impl SearchProvider for ColorProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, _limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        if !settings::current(&self.settings).color_picker.enabled {
            return Ok(Vec::new());
        }
        Ok(self.rows(query, super::now_ms()))
    }

    fn claims(&self, query: &str) -> bool {
        // "colors" (plural) means "show me my colors"; "color" also finds
        // apps and settings.
        (query.eq_ignore_ascii_case("colors") || query.eq_ignore_ascii_case("colours"))
            && settings::current(&self.settings).color_picker.enabled
    }

    fn owns(&self, id: &str) -> bool {
        id == PICK_ID || id.starts_with(RECENT_PREFIX) || id.starts_with(VALUE_PREFIX)
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        if id == PICK_ID {
            return Ok(
                match super::start_pick(
                    Arc::clone(&self.settings),
                    Arc::clone(&self.recent),
                    self.marker.clone(),
                ) {
                    Ok(()) => Activation::Hide,
                    Err(message) => Activation::Notice { message },
                },
            );
        }
        if let Some(rest) = id.strip_prefix(VALUE_PREFIX) {
            let (color, format) = value_from_id(rest).ok_or_else(|| unknown(id))?;
            return Ok(deliver(color.format(format), action));
        }
        let hex = id.strip_prefix(RECENT_PREFIX).ok_or_else(|| unknown(id))?;
        let color = model::from_hex(hex).ok_or_else(|| unknown(id))?;
        if action == Action::Delete {
            let recent = Arc::clone(&self.recent);
            let saved = tokio::task::spawn_blocking(move || {
                let mut recent = match recent.lock() {
                    Ok(r) => r,
                    Err(poisoned) => poisoned.into_inner(),
                };
                recent.remove(color);
                recent.save()
            })
            .await
            .unwrap_or_else(|e| Err(e.to_string()));
            if let Err(e) = saved {
                tracing::warn!("could not save recent colors: {e}");
            }
            return Ok(Activation::Refresh);
        }
        let format = settings::current(&self.settings).color_picker.format;
        Ok(deliver(color.format(format), action))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::RwLock;

    const ORANGE: Rgb = Rgb::new(255, 136, 0);
    const TEAL: Rgb = Rgb::new(0, 128, 128);

    fn provider() -> ColorProvider {
        let mut recent = Recent::default();
        recent.add(TEAL, 1_000);
        recent.add(ORANGE, 61_000);
        ColorProvider::new(
            Arc::new(RwLock::new(Settings::default())),
            Arc::new(Mutex::new(recent)),
            PathBuf::from("unused"),
        )
    }

    fn ids(rows: &[ResultItem]) -> Vec<&str> {
        rows.iter().map(|r| r.id.as_str()).collect()
    }

    #[test]
    fn a_typed_color_shows_its_other_formats() {
        let p = provider();
        let rows = p.rows("#ff8800", 0);
        assert_eq!(
            ids(&rows),
            ["color:value:rgb:#ff8800", "color:value:hsl:#ff8800"]
        );
        assert_eq!(rows[0].title, "rgb(255, 136, 0)");
        assert_eq!(rows[0].kind, ItemKind::Answer);
        assert_eq!(rows[0].icon.as_deref(), Some("color:#ff8800"));
        assert_eq!(rows[1].title, "hsl(32, 100%, 50%)");
        // Short hex: the long form is new information.
        assert_eq!(p.rows("#F80", 0).len(), 3);
        let from_rgb = p.rows("rgb(255,136,0)", 0);
        assert_eq!(
            ids(&from_rgb),
            ["color:value:hex:#ff8800", "color:value:hsl:#ff8800"]
        );
    }

    #[test]
    fn color_lists_what_you_picked_newest_first() {
        let p = provider();
        let rows = p.rows("colors", 121_000);
        assert_eq!(
            ids(&rows),
            ["color:pick", "color:recent:#ff8800", "color:recent:#008080"]
        );
        let orange = &rows[1];
        assert_eq!(orange.title, "#ff8800");
        assert_eq!(orange.subtitle, "rgb(255, 136, 0) · hsl(32, 100%, 50%)");
        assert_eq!(orange.accessory.as_deref(), Some("1 min ago"));
        assert!(rows[1].score > rows[2].score);
        assert_eq!(ids(&p.rows("color", 0)).len(), 3);
    }

    #[test]
    fn rows_follow_the_format_setting() {
        let p = provider();
        if let Ok(mut s) = p.settings.write() {
            s.color_picker.format = ColorFormat::Rgb;
        }
        let rows = p.rows("colors", 0);
        assert_eq!(rows[1].title, "rgb(255, 136, 0)");
        assert_eq!(rows[1].subtitle, "#ff8800 · hsl(32, 100%, 50%)");
    }

    #[test]
    fn pick_color_is_found_by_name() {
        let p = provider();
        assert_eq!(ids(&p.rows("pick color", 0)), ["color:pick"]);
        assert_eq!(ids(&p.rows("eyedropper", 0)), ["color:pick"]);
        assert_eq!(ids(&p.rows("color picker", 0)), ["color:pick"]);
        assert!(p.rows("excel", 0).is_empty());
        assert!(p.rows("#", 0).is_empty());
    }

    #[tokio::test]
    async fn turned_off_shows_nothing() {
        let p = provider();
        if let Ok(mut s) = p.settings.write() {
            s.color_picker.enabled = false;
        }
        assert!(p.query("colors", 8).await.unwrap().is_empty());
        assert!(!p.claims("colors"));
    }

    #[test]
    fn claims_only_the_plural() {
        let p = provider();
        assert!(p.claims("colors") && p.claims("Colours"));
        assert!(!p.claims("color") && !p.claims("colorful"));
    }

    #[tokio::test]
    async fn enter_copies_ctrl_enter_pastes_ctrl_d_forgets() {
        let p = provider();
        assert_eq!(
            p.activate("color:recent:#ff8800", Action::Copy)
                .await
                .unwrap(),
            Activation::Copy {
                text: "#ff8800".into()
            }
        );
        assert_eq!(
            p.activate("color:value:hsl:#ff8800", Action::Paste)
                .await
                .unwrap(),
            Activation::Paste {
                text: Some("hsl(32, 100%, 50%)".into())
            }
        );
        assert_eq!(
            p.activate("color:recent:#ff8800", Action::Delete)
                .await
                .unwrap(),
            Activation::Refresh
        );
        assert_eq!(
            ids(&p.rows("colors", 0)),
            ["color:pick", "color:recent:#008080"]
        );
        assert!(p.activate("color:recent:nope", Action::Copy).await.is_err());
        assert!(p
            .activate("color:value:cmyk:#ff8800", Action::Copy)
            .await
            .is_err());
    }

    #[test]
    fn its_icon_is_drawn() {
        assert!(crate::features::system::matching::glyph_exists(
            "eyedropper"
        ));
        assert_eq!(pick_row().icon.as_deref(), Some("glyph:eyedropper"));
    }

    #[tokio::test]
    async fn owns_its_ids() {
        let p = provider();
        assert!(p.owns("color:pick") && p.owns("color:recent:#000000"));
        assert!(p.owns("color:value:rgb:#000000"));
        assert!(!p.owns("calc:1+1"));
    }
}
