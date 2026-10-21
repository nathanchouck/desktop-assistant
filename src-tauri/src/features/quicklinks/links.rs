//! Matching a query to quicklinks and building the address to open.

use super::Quicklink;

/// Replaced by what you type after the keyword.
pub const PLACEHOLDER: &str = "{query}";
/// Typing at least this much of a link's name finds it too ("youtu").
const MIN_NAME_PREFIX: usize = 3;

/// What a query asks of one quicklink.
#[derive(Debug, Clone, PartialEq)]
pub enum Match<'a> {
    /// "yt lofi": search the link for this text.
    Search(&'a Quicklink, &'a str),
    /// "yt" (`exact`), or the start of its name ("youtu").
    Link(&'a Quicklink, bool),
}

/// Whether the link can be used: a keyword without spaces or colons (ids
/// use a colon) and a web address.
pub fn is_usable(link: &Quicklink) -> bool {
    let keyword = link.keyword.trim();
    !keyword.is_empty()
        && !keyword.contains(|c: char| c.is_whitespace() || c == ':')
        && is_web_url(&link.url)
}

/// Only http and https: a quicklink never runs a program or opens a file.
pub fn is_web_url(url: &str) -> bool {
    let url = url.trim();
    let lower = url.get(..8).unwrap_or(url).to_ascii_lowercase();
    (lower.starts_with("https://") && url.len() > 8)
        || (lower.starts_with("http://") && url.len() > 7)
}

/// Whether the link searches for what you type (has `{query}`).
pub fn takes_text(link: &Quicklink) -> bool {
    link.url.contains(PLACEHOLDER)
}

/// The quicklinks `query` points at, best first.
pub fn find<'a>(links: &'a [Quicklink], query: &'a str) -> Vec<Match<'a>> {
    let query = query.trim();
    let (word, rest) = match query.split_once(char::is_whitespace) {
        Some((word, rest)) => (word, rest.trim()),
        None => (query, ""),
    };
    let lower = query.to_lowercase();
    let mut found = Vec::new();
    for link in links.iter().filter(|l| is_usable(l)) {
        if word.eq_ignore_ascii_case(link.keyword.trim()) {
            if rest.is_empty() {
                found.push(Match::Link(link, true));
            } else if takes_text(link) {
                found.push(Match::Search(link, rest));
            }
        } else if lower.chars().count() >= MIN_NAME_PREFIX
            && link.name.to_lowercase().starts_with(&lower)
        {
            found.push(Match::Link(link, false));
        }
    }
    found
}

/// The link's address with `text` in place of `{query}`.
pub fn build_url(link: &Quicklink, text: &str) -> String {
    link.url.trim().replace(PLACEHOLDER, &encode(text))
}

/// Percent-encodes everything but unreserved characters, so the text can
/// go anywhere in an address (query string or path).
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// "https://www.youtube.com/results?…" → "www.youtube.com", for the
/// row's subtitle.
pub fn host(url: &str) -> &str {
    let url = url.trim();
    let rest = url.split_once("://").map_or(url, |(_, rest)| rest);
    rest.split(['/', '?', '#']).next().unwrap_or(rest)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn link(keyword: &str, name: &str, url: &str) -> Quicklink {
        Quicklink {
            keyword: keyword.into(),
            name: name.into(),
            url: url.into(),
        }
    }

    fn links() -> Vec<Quicklink> {
        vec![
            link(
                "yt",
                "YouTube",
                "https://www.youtube.com/results?search_query={query}",
            ),
            link("gh", "GitHub", "https://github.com/search?q={query}"),
            link("mail", "Gmail", "https://mail.google.com"),
            link("bad", "Bad", "file:///C:/Windows/System32/calc.exe"),
            link("two words", "Spaces", "https://example.com/{query}"),
        ]
    }

    #[test]
    fn keyword_and_text_searches() {
        let links = links();
        assert_eq!(
            find(&links, "yt lofi beats"),
            [Match::Search(&links[0], "lofi beats")]
        );
        assert_eq!(
            find(&links, "YT  cats "),
            [Match::Search(&links[0], "cats")]
        );
        assert_eq!(
            find(&links, "gh tauri"),
            [Match::Search(&links[1], "tauri")]
        );
    }

    #[test]
    fn keyword_alone_or_name_finds_the_link() {
        let links = links();
        assert_eq!(find(&links, "yt"), [Match::Link(&links[0], true)]);
        assert_eq!(find(&links, "youtu"), [Match::Link(&links[0], false)]);
        assert_eq!(find(&links, "mail"), [Match::Link(&links[2], true)]);
        assert_eq!(find(&links, "gma"), [Match::Link(&links[2], false)]);
        // Too short to mean the name; a link without {query} takes no text.
        assert!(find(&links, "yo").is_empty());
        assert!(find(&links, "mail inbox").is_empty());
        assert!(find(&links, "ytlofi").is_empty());
    }

    #[test]
    fn unusable_links_are_skipped() {
        let links = links();
        assert!(find(&links, "bad x").is_empty());
        assert!(find(&links, "two words x").is_empty());
        assert!(!is_web_url("javascript:alert(1)"));
        assert!(!is_web_url("https://"));
        assert!(is_web_url("HTTPS://Example.com"));
        assert!(is_web_url("http://localhost:8080"));
    }

    #[test]
    fn text_is_encoded() {
        let links = links();
        assert_eq!(
            build_url(&links[0], "lofi & chill/2"),
            "https://www.youtube.com/results?search_query=lofi%20%26%20chill%2F2"
        );
        assert_eq!(encode("café 100%"), "caf%C3%A9%20100%25");
        assert_eq!(encode("a-b_c.d~e"), "a-b_c.d~e");
        assert_eq!(build_url(&links[2], "ignored"), "https://mail.google.com");
    }

    #[test]
    fn hosts() {
        assert_eq!(
            host("https://www.youtube.com/results?q=1"),
            "www.youtube.com"
        );
        assert_eq!(host("http://localhost:8080"), "localhost:8080");
        assert_eq!(host("https://example.com?x"), "example.com");
    }
}
