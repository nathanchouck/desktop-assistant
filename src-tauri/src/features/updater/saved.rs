//! What the updater remembers on this PC between runs, in `updates.json`
//! in the data folder. Not in settings.toml: that file may be synced to
//! other PCs, and each PC checks and installs for itself.

use std::fs;
use std::io;
use std::path::Path;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Saved {
    /// When a check last got an answer (seconds since 1970).
    pub last_checked: Option<u64>,
    /// The version an install was started for, until the next start
    /// says whether it worked.
    pub installing: Option<String>,
}

impl Saved {
    /// What's saved at `path`; nothing if the file is missing or unreadable
    /// (it only holds conveniences).
    pub fn load(path: &Path) -> Self {
        let Ok(text) = fs::read_to_string(path) else {
            return Self::default();
        };
        serde_json::from_str(&text).unwrap_or_else(|e| {
            tracing::debug!("ignoring {}: {e}", path.display());
            Self::default()
        })
    }

    /// Writes atomically (temp file, then rename).
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        fs::write(&tmp, serde_json::to_string_pretty(self)?)?;
        fs::rename(&tmp, path)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trips_and_tolerates_a_bad_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data").join("updates.json");
        assert_eq!(Saved::load(&path), Saved::default());
        let saved = Saved {
            last_checked: Some(1_800_000_000),
            installing: Some("0.2.0".into()),
        };
        saved.save(&path).unwrap();
        assert_eq!(Saved::load(&path), saved);
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(Saved::load(&path), Saved::default());
        fs::write(&path, r#"{"last_checked": 5}"#).unwrap();
        assert_eq!(Saved::load(&path).last_checked, Some(5));
    }
}
