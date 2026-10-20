//! What the file index skips: hidden and system items (which covers
//! AppData), dot-folders such as `.git`, and anything inside a folder listed
//! in `search.exclusions`.

/// Normalized exclusions, each wrapped as `\segment\...\` in lowercase so a
/// plain substring test matches whole path segments only.
#[derive(Debug, Clone, Default)]
pub struct Rules {
    exclusions: Vec<String>,
}

impl Rules {
    pub fn new(exclusions: &[String]) -> Self {
        let exclusions = exclusions
            .iter()
            .map(|e| {
                e.trim_matches(['\\', '/'])
                    .replace('/', "\\")
                    .to_lowercase()
            })
            .filter(|e| !e.is_empty())
            .map(|e| format!("\\{e}\\"))
            .collect();
        Self { exclusions }
    }

    /// Whether a directory entry is left out of the index entirely.
    pub fn skip_entry(&self, name: &str, hidden_or_system: bool) -> bool {
        hidden_or_system || name.starts_with('.')
    }

    /// Whether the index should not look inside this folder.
    pub fn excludes_dir(&self, path: &str) -> bool {
        if self.exclusions.is_empty() {
            return false;
        }
        let normalized = format!("\\{}\\", path.replace('/', "\\").to_lowercase());
        self.exclusions
            .iter()
            .any(|e| normalized.contains(e.as_str()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rules() -> Rules {
        Rules::new(&[
            "node_modules".into(),
            r"AppData\Local\Temp\".into(),
            "".into(),
        ])
    }

    #[test]
    fn skips_hidden_and_dot_entries() {
        let r = rules();
        assert!(r.skip_entry(".git", false));
        assert!(r.skip_entry("AppData", true));
        assert!(!r.skip_entry("Documents", false));
    }

    #[test]
    fn excludes_whole_segments_only() {
        let r = rules();
        assert!(r.excludes_dir(r"C:\Users\n\code\site\node_modules"));
        assert!(r.excludes_dir(r"C:\Users\n\code\site\node_modules\react"));
        assert!(r.excludes_dir(r"C:\Users\n\AppData\Local\Temp\x"));
        assert!(!r.excludes_dir(r"C:\Users\n\code\node_modules_notes"));
        assert!(!r.excludes_dir(r"C:\Users\n\AppData\Local"));
        assert!(r.excludes_dir("/home/n/site/node_modules"));
    }

    #[test]
    fn ignores_case() {
        assert!(rules().excludes_dir(r"C:\Users\n\Site\NODE_MODULES"));
    }
}
