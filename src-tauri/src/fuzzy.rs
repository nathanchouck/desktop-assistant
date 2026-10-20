//! Fuzzy matching for results, on top of nucleo (the matcher from Helix).
//! Handles acronyms and gaps: "vsc" finds "Visual Studio Code".

use nucleo_matcher::pattern::{AtomKind, CaseMatching, Normalization, Pattern};
use nucleo_matcher::{Config, Matcher, Utf32Str};

/// One query, scored against many candidates. Create one per search.
pub struct FuzzyQuery {
    pattern: Pattern,
    matcher: Matcher,
    buf: Vec<char>,
    /// The query matched against itself: the best score this query can get.
    best: f32,
}

impl FuzzyQuery {
    pub fn new(query: &str) -> Self {
        let mut config = Config::DEFAULT;
        // Launcher queries are typed from the start of a name.
        config.prefer_prefix = true;
        let mut matcher = Matcher::new(config);
        // Plain fuzzy atoms: no fzf-style syntax (^, $, !, ') to trip over.
        let pattern = Pattern::new(
            query,
            CaseMatching::Ignore,
            Normalization::Smart,
            AtomKind::Fuzzy,
        );
        let mut buf = Vec::new();
        let best = pattern
            .score(Utf32Str::new(query, &mut buf), &mut matcher)
            .unwrap_or(1)
            .max(1) as f32;
        Self {
            pattern,
            matcher,
            buf,
            best,
        }
    }

    /// A score in 0..=1, or `None` if `text` doesn't match.
    pub fn score(&mut self, text: &str) -> Option<f32> {
        let raw = self
            .pattern
            .score(Utf32Str::new(text, &mut self.buf), &mut self.matcher)?;
        Some((raw as f32 / self.best).min(1.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_acronyms() {
        let mut q = FuzzyQuery::new("vsc");
        assert!(q.score("Visual Studio Code").is_some());
    }

    #[test]
    fn rejects_non_matches() {
        let mut q = FuzzyQuery::new("xyz");
        assert_eq!(q.score("Notepad"), None);
    }

    #[test]
    fn ignores_case_and_exact_is_best() {
        let mut q = FuzzyQuery::new("notepad");
        assert_eq!(q.score("Notepad"), Some(1.0));
    }

    #[test]
    fn prefix_beats_scattered() {
        let mut q = FuzzyQuery::new("cal");
        let prefix = q.score("Calculator").unwrap();
        let scattered = q.score("Microsoft Clipchamp Audio Library").unwrap_or(0.0);
        assert!(prefix > scattered, "{prefix} vs {scattered}");
    }

    #[test]
    fn multiple_words_all_must_match() {
        let mut q = FuzzyQuery::new("studio code");
        assert!(q.score("Visual Studio Code").is_some());
        assert_eq!(q.score("Android Studio"), None);
    }
}
