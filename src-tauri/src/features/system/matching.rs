//! Finding a command by name: the query against its title and keywords.
//! Shared by the command lists in `system`, `window_layout`, `media` and
//! `color`.

use crate::fuzzy::FuzzyQuery;

/// Keyword matches count a little less than the title.
const KEYWORD_WEIGHT: f32 = 0.9;
/// Weaker matches are noise: gaps are allowed, so a long title holds the
/// letters of many short queries in order somewhere.
const MIN_SCORE: f32 = 0.3;
/// One letter matches nearly every command, so lists start at two.
const MIN_QUERY_CHARS: usize = 2;

/// Whether `query` is long enough to look for commands.
pub fn long_enough(query: &str) -> bool {
    query.chars().count() >= MIN_QUERY_CHARS
}

/// How well the query finds a command (0..=1), or `None` if it doesn't.
pub fn score(fuzzy: &mut FuzzyQuery, title: &str, keywords: &[&str]) -> Option<f32> {
    let title = fuzzy.score(title);
    let keyword = keywords
        .iter()
        .filter_map(|k| fuzzy.score(k))
        .map(|s| s * KEYWORD_WEIGHT)
        .reduce(f32::max);
    title
        .into_iter()
        .chain(keyword)
        .reduce(f32::max)
        .filter(|&s| s >= MIN_SCORE)
}

/// Whether `ui/src/lib/glyphs.ts` draws a glyph called `name`, so a
/// typo in a row's icon fails a test instead of showing the kind's glyph.
#[cfg(test)]
pub fn glyph_exists(name: &str) -> bool {
    const GLYPHS: &str = include_str!("../../../../ui/src/lib/glyphs.ts");
    GLYPHS.contains(&format!("\n  {name}: ")) || GLYPHS.contains(&format!("\n  \"{name}\": "))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn find(query: &str, title: &str, keywords: &[&str]) -> Option<f32> {
        score(&mut FuzzyQuery::new(query), title, keywords)
    }

    #[test]
    fn title_and_keywords_match() {
        assert_eq!(find("sleep", "Sleep", &[]), Some(1.0));
        assert!(find("dark", "Toggle Dark Mode", &[]).is_some());
        let keyword = find("logout", "Sign Out", &["log out", "logout"]).unwrap();
        assert!(keyword < 1.0 && keyword > 0.8);
    }

    #[test]
    fn unrelated_queries_dont() {
        assert_eq!(find("chrome", "Toggle Dark Mode", &["theme"]), None);
        assert_eq!(find("notepad", "Notification Settings", &[]), None);
    }

    #[test]
    fn glyph_lookup() {
        assert!(glyph_exists("lock"));
        assert!(glyph_exists("layout-left"));
        assert!(!glyph_exists("no-such-glyph"));
    }

    #[test]
    fn single_letters_are_too_short() {
        assert!(!long_enough("s"));
        assert!(long_enough("sl"));
        assert!(!long_enough(""));
    }
}
