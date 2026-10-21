//! The last colors you picked, newest first, kept in `colors.json` in the
//! data folder.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::model::{self, Rgb};

/// Colors kept.
pub const MAX_COLORS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Picked {
    pub color: Rgb,
    /// When, in Unix milliseconds.
    pub at: i64,
}

/// On disk: `[{"hex": "#ff8800", "at": 1761000000000}]`.
#[derive(Serialize, Deserialize)]
struct Stored {
    hex: String,
    at: i64,
}

#[derive(Debug, Default)]
pub struct Recent {
    colors: Vec<Picked>,
    file: Option<PathBuf>,
}

impl Recent {
    /// Reads the list from `file`; a missing or damaged file starts empty.
    pub fn load(file: PathBuf) -> Self {
        let colors = std::fs::read_to_string(&file)
            .ok()
            .and_then(|text| serde_json::from_str::<Vec<Stored>>(&text).ok())
            .unwrap_or_default()
            .into_iter()
            .filter_map(|s| {
                Some(Picked {
                    color: model::from_hex(&s.hex)?,
                    at: s.at,
                })
            })
            .take(MAX_COLORS)
            .collect();
        Self {
            colors,
            file: Some(file),
        }
    }

    pub fn colors(&self) -> &[Picked] {
        &self.colors
    }

    /// Puts `color` first (moving it if it was already listed).
    pub fn add(&mut self, color: Rgb, at: i64) {
        self.colors.retain(|p| p.color != color);
        self.colors.insert(0, Picked { color, at });
        self.colors.truncate(MAX_COLORS);
    }

    /// Whether `color` was listed.
    pub fn remove(&mut self, color: Rgb) -> bool {
        let before = self.colors.len();
        self.colors.retain(|p| p.color != color);
        self.colors.len() != before
    }

    /// Writes the list (a temporary file, then renamed over the old one).
    pub fn save(&self) -> Result<(), String> {
        let Some(file) = &self.file else {
            return Ok(());
        };
        let stored: Vec<Stored> = self
            .colors
            .iter()
            .map(|p| Stored {
                hex: p.color.hex(),
                at: p.at,
            })
            .collect();
        let text = serde_json::to_string_pretty(&stored).map_err(|e| e.to_string())?;
        write_atomic(file, &text).map_err(|e| format!("{}: {e}", file.display()))
    }
}

fn write_atomic(file: &Path, text: &str) -> std::io::Result<()> {
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, file)
}

#[cfg(test)]
mod tests {
    use super::*;

    const RED: Rgb = Rgb::new(255, 0, 0);
    const BLUE: Rgb = Rgb::new(0, 0, 255);

    #[test]
    fn newest_first_without_repeats() {
        let mut r = Recent::default();
        r.add(RED, 1);
        r.add(BLUE, 2);
        r.add(RED, 3);
        let colors: Vec<(Rgb, i64)> = r.colors().iter().map(|p| (p.color, p.at)).collect();
        assert_eq!(colors, [(RED, 3), (BLUE, 2)]);
    }

    #[test]
    fn keeps_the_last_twenty() {
        let mut r = Recent::default();
        for i in 0..30u8 {
            r.add(Rgb::new(i, i, i), i64::from(i));
        }
        assert_eq!(r.colors().len(), MAX_COLORS);
        assert_eq!(r.colors()[0].color, Rgb::new(29, 29, 29));
    }

    #[test]
    fn removing() {
        let mut r = Recent::default();
        r.add(RED, 1);
        assert!(r.remove(RED));
        assert!(!r.remove(RED));
        assert!(r.colors().is_empty());
    }

    #[test]
    fn saves_and_loads() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("sub").join("colors.json");
        let mut r = Recent::load(file.clone());
        assert!(r.colors().is_empty(), "no file yet");
        r.add(RED, 1);
        r.add(BLUE, 2);
        r.save().unwrap();
        let again = Recent::load(file.clone());
        assert_eq!(again.colors(), r.colors());

        // A damaged file starts empty; bad entries are skipped.
        std::fs::write(&file, "not json").unwrap();
        assert!(Recent::load(file.clone()).colors().is_empty());
        std::fs::write(&file, r##"[{"hex":"#zzzzzz","at":1},{"hex":"#00ff00","at":2}]"##).unwrap();
        let mixed = Recent::load(file);
        assert_eq!(mixed.colors().len(), 1);
        assert_eq!(mixed.colors()[0].color, Rgb::new(0, 255, 0));
    }
}
