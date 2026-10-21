//! Every emoji and symbol with the words it's found by, built once at
//! startup, and matching a search term against them.

use std::collections::HashMap;

use super::symbols::{EMOJI_KEYWORDS, POPULAR, SYMBOLS};

/// Newer emoji show as empty boxes on Windows (its emoji font is at
/// Emoji 15.1), so they're left out.
const NEWEST: emojis::UnicodeVersion = emojis::UnicodeVersion::new(15, 1);

/// One emoji or symbol.
pub struct Entry {
    /// What gets pasted (with the default skin tone); also the result id.
    pub glyph: &'static str,
    /// As Unicode names it ("thumbs up", "flag: Japan"), or ours for symbols.
    pub name: &'static str,
    /// GitHub shortcodes ("+1", "thumbsup").
    pub shortcodes: Vec<&'static str>,
    /// The emoji's data, for skin tones. `None` for symbols.
    pub emoji: Option<&'static emojis::Emoji>,
    lower: String,
    words: Vec<String>,
    keywords: &'static [&'static str],
    /// One of the emoji people use most: ahead of look-alikes.
    popular: bool,
}

impl Entry {
    /// Shown at the right of the row: ":fire:", or "U+2192" for a symbol.
    pub fn accessory(&self) -> Option<String> {
        if self.emoji.is_none() {
            let mut chars = self.glyph.chars();
            return match (chars.next(), chars.next()) {
                (Some(c), None) => Some(format!("U+{:04X}", c as u32)),
                _ => None,
            };
        }
        self.shortcodes.first().map(|s| format!(":{s}:"))
    }

    /// How well it matches `term` (lower case, words separated by single
    /// spaces), from 0 to 1.
    fn score(&self, term: &str, shortcode: &str, terms: &[&str]) -> Option<f32> {
        let short = term.chars().count() < 3;
        let score = if self.lower == term || self.shortcodes.iter().any(|s| *s == shortcode) {
            1.0
        } else if self.keywords.iter().any(|k| *k == term) {
            0.95
        } else if self.lower.starts_with(term) {
            0.9
        } else if self.shortcodes.iter().any(|s| s.starts_with(shortcode)) {
            0.85
        } else if terms.len() > 1
            && terms
                .iter()
                .all(|t| self.words.iter().any(|w| w.starts_with(t)))
        {
            0.8
        } else if self.words.iter().any(|w| w.starts_with(term)) {
            0.75
        } else if self.keywords.iter().any(|k| k.starts_with(term)) {
            0.7
        } else if !short && self.lower.contains(term) {
            0.4
        } else if !short && self.shortcodes.iter().any(|s| s.contains(shortcode)) {
            0.35
        } else {
            return None;
        };
        // Among equals, popular ones and shorter names first ("fire" before
        // "fire engine"). Both together move less than a step above (0.05).
        let length = self.lower.chars().count().min(40) as f32;
        let popular = if self.popular { 0.02 } else { 0.0 };
        Some(score + popular - length * 0.0005)
    }
}

pub struct Index {
    entries: Vec<Entry>,
    by_glyph: HashMap<&'static str, usize>,
    /// Whole names and shortcodes, for the one suggestion without a prefix.
    by_name: HashMap<String, usize>,
}

impl Index {
    pub fn build() -> Self {
        let keywords: HashMap<&str, &'static [&'static str]> = EMOJI_KEYWORDS
            .iter()
            .filter_map(|(glyph, words)| Some((emojis::get(glyph)?.as_str(), *words)))
            .collect();
        // Symbols first: where one shares a name with an emoji ("copyright",
        // "infinity"), the plain text one is usually what you're typing.
        let mut entries: Vec<Entry> = SYMBOLS
            .iter()
            .map(|&(glyph, name, words)| entry(glyph, name, Vec::new(), words, None))
            .collect();
        entries.extend(
            emojis::iter()
                .filter(|e| e.unicode_version() <= NEWEST)
                .map(|e| {
                    let glyph = e.as_str();
                    entry(
                        glyph,
                        e.name(),
                        e.shortcodes().collect(),
                        keywords.get(glyph).copied().unwrap_or_default(),
                        Some(e),
                    )
                }),
        );
        for e in &mut entries {
            e.popular = !e.keywords.is_empty() && e.emoji.is_some()
                || POPULAR
                    .iter()
                    .any(|p| *p == e.glyph || emojis::get(p).is_some_and(|x| x.as_str() == e.glyph));
        }

        let mut by_glyph = HashMap::new();
        let mut by_name = HashMap::new();
        for (i, e) in entries.iter().enumerate() {
            by_glyph.entry(e.glyph).or_insert(i);
            by_name.entry(e.lower.clone()).or_insert(i);
            for code in &e.shortcodes {
                by_name.entry(code.replace('_', " ")).or_insert(i);
            }
        }
        Self {
            entries,
            by_glyph,
            by_name,
        }
    }

    pub fn get(&self, i: usize) -> &Entry {
        &self.entries[i]
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn by_glyph(&self, glyph: &str) -> Option<usize> {
        self.by_glyph.get(glyph).copied()
    }

    /// The entry whose whole name or shortcode is `name` (lower case).
    pub fn by_name(&self, name: &str) -> Option<usize> {
        self.by_name.get(name).copied()
    }

    /// Entries matching `term`, best first: (index, score).
    pub fn search(&self, term: &str, limit: usize) -> Vec<(usize, f32)> {
        let term = normalize(term);
        if term.is_empty() {
            return Vec::new();
        }
        let shortcode = term.replace(' ', "_");
        let terms: Vec<&str> = term.split(' ').collect();
        let mut found: Vec<(usize, f32)> = self
            .entries
            .iter()
            .enumerate()
            .filter_map(|(i, e)| Some((i, e.score(&term, &shortcode, &terms)?)))
            .collect();
        // Best first; equal scores keep the index's order (symbols, then
        // Unicode's: smileys before flags).
        found.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
        found.truncate(limit);
        found
    }
}

fn entry(
    glyph: &'static str,
    name: &'static str,
    shortcodes: Vec<&'static str>,
    keywords: &'static [&'static str],
    emoji: Option<&'static emojis::Emoji>,
) -> Entry {
    let lower = name.to_lowercase();
    let words = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .map(str::to_owned)
        .collect();
    Entry {
        glyph,
        name,
        shortcodes,
        emoji,
        lower,
        words,
        keywords,
        popular: false,
    }
}

/// Lower case, `_` as a space (":thumbs_up"), no closing ':' (":fire:"),
/// single spaces.
pub fn normalize(term: &str) -> String {
    term.trim()
        .trim_end_matches(':')
        .to_lowercase()
        .replace('_', " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn top(index: &Index, term: &str, n: usize) -> Vec<&'static str> {
        index
            .search(term, n)
            .into_iter()
            .map(|(i, _)| index.get(i).glyph)
            .collect()
    }

    #[test]
    fn finds_by_name_shortcode_and_keyword() {
        let index = Index::build();
        assert_eq!(top(&index, "fire", 1), ["🔥"]);
        assert_eq!(top(&index, "thumbs up", 1), ["👍"]);
        assert_eq!(top(&index, "thumbs_up", 1), ["👍"]);
        assert_eq!(top(&index, "+1", 1), ["👍"]);
        assert_eq!(top(&index, "heart", 1), ["❤️"]);
        assert_eq!(top(&index, "tada", 1), ["🎉"]);
        assert_eq!(top(&index, "lol", 1), ["😂"]);
        assert_eq!(top(&index, "tears joy", 1), ["😂"]);
        assert_eq!(top(&index, "fire:", 1), ["🔥"]);
    }

    #[test]
    fn symbols_are_found_by_name_and_shorthand() {
        let index = Index::build();
        assert!(top(&index, "arrow", 40).contains(&"→"));
        assert_eq!(top(&index, "degree", 1), ["°"]);
        assert!(top(&index, "check", 10).contains(&"✓"));
        assert_eq!(top(&index, "->", 1), ["→"]);
        assert_eq!(top(&index, "!=", 1), ["≠"]);
        assert_eq!(top(&index, "infinity", 1), ["∞"]);
        assert_eq!(top(&index, "copyright", 1), ["©"]);
    }

    #[test]
    fn whole_names_for_suggestions() {
        let index = Index::build();
        let glyph = |name: &str| index.by_name(name).map(|i| index.get(i).glyph);
        assert_eq!(glyph("fire"), Some("🔥"));
        assert_eq!(glyph("thumbs up"), Some("👍"));
        assert_eq!(glyph("degree"), Some("°"));
        assert_eq!(glyph("chrome"), None);
        assert_eq!(glyph("fir"), None);
    }

    #[test]
    fn accessories() {
        let index = Index::build();
        let accessory = |glyph: &str| index.get(index.by_glyph(glyph).unwrap()).accessory();
        assert_eq!(accessory("🔥").as_deref(), Some(":fire:"));
        assert_eq!(accessory("→").as_deref(), Some("U+2192"));
    }

    #[test]
    fn leaves_out_emoji_windows_cant_draw() {
        let index = Index::build();
        assert!(index.len() > 1800);
        assert!((0..index.len()).all(|i| index
            .get(i)
            .emoji
            .is_none_or(|e| e.unicode_version() <= NEWEST)));
    }

    #[test]
    fn normalizing() {
        assert_eq!(normalize("  Thumbs_Up: "), "thumbs up");
        assert_eq!(normalize("face   with"), "face with");
    }
}
