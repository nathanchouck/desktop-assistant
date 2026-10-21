//! Whether a query is a calculation, and rewriting it into something fend
//! understands ("72 f to c" → "72 °F to °C", "€50 in $" → "50 EUR in USD").
//!
//! The answer row always ranks first, so a false positive steals Enter from
//! the app or file you meant. The rules lean towards silence: a query needs a
//! digit and an operation (an operator, a conversion, a function), and things
//! shaped like dates, versions and phone numbers are left alone. Starting the
//! query with `=` skips those checks.

use super::rewrite::{
    conversion_keyword, is_money_word, merge_compound_units, rewrite_bytes, rewrite_hex_word,
    rewrite_inches, rewrite_money, rewrite_percentages, rewrite_temperatures, rewrite_times,
    target_base,
};
use super::tokens::{depth, join, tokenize, Kind, Token};

/// Longer queries aren't calculations (and keep fend's work small).
const MAX_CHARS: usize = 100;
/// Deeper nesting isn't typed by hand; fend's parser recurses on it.
const MAX_DEPTH: usize = 12;

/// Number bases an answer can be shown in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    Hex,
    Binary,
    Octal,
}

/// A query the calculator should answer.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Prepared {
    /// What fend evaluates.
    pub expr: String,
    /// The query as understood, shown under the answer.
    pub shown: String,
    /// It involves money: the answer is rounded to cents and needs rates.
    pub currency: bool,
    /// Converted to this base: the answer gets its prefix (`0x`, `0b`, `0o`).
    pub base: Option<Base>,
    /// A conversion (`to`, `in`, `as`): approximate answers are rounded.
    pub conversion: bool,
    /// Started with `=`: show the answer even if it looks like the query.
    pub forced: bool,
}

/// Decides whether `query` is a calculation and prepares it for fend.
pub fn prepare(query: &str) -> Option<Prepared> {
    let query = query.trim();
    let (forced, query) = match query.strip_prefix('=') {
        Some(rest) => (true, rest.trim()),
        None => (false, query),
    };
    // "2+2=" is a calculation too; any other '=' is an assignment or a
    // comparison, which we don't do.
    let query = query.trim_end_matches('=').trim_end();
    if query.is_empty() || query.contains('=') || query.chars().count() > MAX_CHARS {
        return None;
    }
    if !forced && (starts_with_lone_letter(query) || looks_like_an_identifier(query)) {
        return None;
    }
    let mut tokens = tokenize(query)?;
    if depth(&tokens) > MAX_DEPTH || (!forced && starts_hyphenated(&tokens)) {
        return None;
    }

    let mut currency = rewrite_money(&mut tokens);
    merge_compound_units(&mut tokens);
    rewrite_times(&mut tokens);
    rewrite_bytes(&mut tokens);
    let split = conversion_keyword(&tokens);
    rewrite_inches(&mut tokens, split);
    let mut base = None;
    let mut conversion = false;
    // Counts as typing a number: "usd to eur", "ff in decimal".
    let mut implied_number = false;
    if let Some(k) = split {
        conversion = true;
        // "2 in 1", "1 to 10": nothing to convert to.
        if !tokens[k + 1..]
            .iter()
            .any(|t| matches!(t.kind, Kind::Word | Kind::Money(_)))
        {
            return None;
        }
        base = target_base(&tokens[k + 1..]);
        implied_number |= rewrite_hex_word(&mut tokens, k);
        let k = rewrite_temperatures(&mut tokens, k);
        // "usd to eur": the rate for one.
        if k == 1 && is_money_word(&tokens[0]) && tokens[k + 1..].iter().all(is_money_word) {
            tokens.insert(0, Token::new("1", Kind::Number));
            implied_number = true;
        }
    } else if tokens.len() == 1 && is_base_literal(&tokens[0]) {
        // "0xff": show it in decimal.
        tokens.push(Token::new("to", Kind::Word));
        tokens.push(Token::new("decimal", Kind::Word));
        conversion = true;
    }
    currency |= tokens.iter().any(is_money_word);

    if !forced {
        let has_digit = query.chars().any(|c| c.is_ascii_digit());
        if !(has_digit || implied_number) || !(conversion || has_operation(&tokens)) {
            return None;
        }
    }

    let shown = join(&tokens);
    let mut expr_tokens = rewrite_percentages(tokens);
    // Hex or binary in, no base asked for: answer in decimal.
    if !conversion && expr_tokens.iter().any(is_base_literal) {
        expr_tokens.push(Token::new("to", Kind::Word));
        expr_tokens.push(Token::new("decimal", Kind::Word));
    }
    Some(Prepared {
        expr: join(&expr_tokens),
        shown,
        currency,
        base,
        conversion,
        forced,
    })
}

/// "c 2+2", "g 5 km": a one-letter prefix (Claude, ChatGPT, files) or a
/// quicklink-style keyword, not math.
fn starts_with_lone_letter(query: &str) -> bool {
    let mut chars = query.chars();
    matches!(
        (chars.next(), chars.next()),
        (Some(a), Some(b)) if a.is_alphabetic() && b.is_whitespace()
    )
}

/// Dates, versions, phone numbers and the like: "2024-01-01", "1.2.3",
/// "555-1234", "2024-01". Each would evaluate (as subtraction and so on),
/// but you typed it to find a file named that way.
fn looks_like_an_identifier(query: &str) -> bool {
    let seps: Vec<char> = query
        .chars()
        .filter(|c| matches!(c, '-' | '.' | '/'))
        .collect();
    let Some(&sep) = seps.first() else {
        return false;
    };
    if seps.iter().any(|&c| c != sep) {
        return false;
    }
    let groups: Vec<&str> = query.split(sep).collect();
    if !groups
        .iter()
        .all(|g| !g.is_empty() && g.chars().all(|c| c.is_ascii_digit()))
    {
        return false;
    }
    let padded = groups[1..]
        .iter()
        .any(|g| g.len() >= 2 && g.starts_with('0'));
    let phone = sep == '-' && groups.len() == 2 && groups[0].len() == 3 && groups[1].len() == 4;
    groups.len() >= 3 || padded || phone
}

/// "k-9", "t-1000": a name with a hyphen, not "k minus 9".
fn starts_hyphenated(tokens: &[Token]) -> bool {
    matches!(tokens, [word, dash, next, ..]
        if word.kind == Kind::Word
            && dash.is_op("-")
            && dash.glued
            && next.glued
            && !FUNCTIONS.iter().any(|f| word.is(f)))
}

fn is_base_literal(t: &Token) -> bool {
    t.kind == Kind::Number && t.text.len() > 2 && {
        let p = &t.text[..2];
        p.eq_ignore_ascii_case("0x") || p.eq_ignore_ascii_case("0b") || p.eq_ignore_ascii_case("0o")
    }
}

/// Function names that make "name 16" or "name(2)" a calculation.
const FUNCTIONS: &[&str] = &[
    "sqrt", "cbrt", "sin", "cos", "tan", "asin", "acos", "atan", "sinh", "cosh", "tanh", "ln",
    "log", "log2", "log10", "exp", "abs", "floor", "ceil", "round",
];

/// An operator between operands, `mod`, `of` (percent of), or a function
/// applied to something. A leading minus, brackets and a percent sign
/// stuck to a number ("50%") don't count.
fn has_operation(tokens: &[Token]) -> bool {
    tokens.iter().enumerate().any(|(i, t)| match t.kind {
        Kind::Op => {
            i > 0
                && match t.text.as_str() {
                    "(" | ")" | "," => false,
                    "%" => !t.glued,
                    _ => true,
                }
        }
        Kind::Word => {
            t.is("mod")
                || (t.is("of") && i > 0)
                || (FUNCTIONS.iter().any(|f| t.is(f))
                    && tokens
                        .get(i + 1)
                        .is_some_and(|n| n.kind == Kind::Number || n.is_op("(")))
        }
        _ => false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expr(query: &str) -> Option<String> {
        prepare(query).map(|p| p.expr)
    }

    #[test]
    fn plain_searches_are_not_calculations() {
        for query in [
            "chrome",
            "code",
            "notes",
            "tax 2024",
            "e",
            "a",
            "pi",
            "km",
            "5",
            "-5",
            "3.14",
            "1e10",
            "4k",
            "24h",
            "1080p",
            "2fa",
            "50%",
            "100%",
            "5 km",
            "$100",
            "€50",
            "5 usd",
            "3 feet",
            "12 in",
            "photoshop 2024",
            "python 3.12",
            "1password",
            "paint 3d",
            "win 11",
            "x64",
            "mp3",
            "the 1975",
            "8 ball",
            "12 angry men",
            "hello world",
            "",
            "   ",
        ] {
            assert_eq!(prepare(query), None, "{query:?}");
        }
    }

    #[test]
    fn dates_versions_and_phone_numbers_are_left_alone() {
        for query in [
            "2024-01-01",
            "2024-01",
            "10-05",
            "1.2.3",
            "192.168.1.1",
            "1/2/2024",
            "555-1234",
            "555-123-4567",
        ] {
            assert_eq!(prepare(query), None, "{query:?}");
        }
        // Arithmetic that only looks a bit like them still works.
        assert_eq!(expr("10-5").as_deref(), Some("10-5"));
        assert_eq!(expr("4/20").as_deref(), Some("4/20"));
        assert_eq!(expr("3.14-1").as_deref(), Some("3.14-1"));
        assert_eq!(expr("2024-1").as_deref(), Some("2024-1"));
    }

    #[test]
    fn one_letter_prefixes_belong_to_other_features() {
        for query in [
            "c 2+2", "g 5 + 5", "f 2024-1", "d 10 * 3", "x 2*3", "k-9", "t-1000",
        ] {
            assert_eq!(prepare(query), None, "{query:?}");
        }
    }

    #[test]
    fn arithmetic_and_functions() {
        for (query, want) in [
            ("2^10", "2^10"),
            ("2 + 2", "2 + 2"),
            ("1,000 + 1", "1,000 + 1"),
            ("10!", "10!"),
            ("2**8", "2**8"),
            ("6 ÷ 2", "6 ÷ 2"),
            ("10 mod 3", "10 mod 3"),
            ("10 % 3", "10 % 3"),
            ("sqrt(2)", "sqrt(2)"),
            ("sqrt 16", "sqrt 16"),
            ("ln 2", "ln 2"),
            ("sin(pi/2)", "sin(pi/2)"),
            ("(1+2)*3", "(1+2)*3"),
            ("2+2=", "2+2"),
            ("5 − 3", "5 - 3"),
            ("max(1,2) + 1", "max(1,2) + 1"),
        ] {
            assert_eq!(expr(query).as_deref(), Some(want), "{query:?}");
        }
    }

    #[test]
    fn x_means_times_between_numbers() {
        assert_eq!(expr("3 x 4").as_deref(), Some("3 × 4"));
        assert_eq!(expr("3x4").as_deref(), Some("3 × 4"));
        assert_eq!(expr("0x1f + 1").as_deref(), Some("0x1f + 1 to decimal"));
    }

    #[test]
    fn percentages_work_like_a_desk_calculator() {
        let p = prepare("200 + 10%").unwrap();
        assert_eq!(p.expr, "(200) * (1 + 10 / 100)");
        assert_eq!(p.shown, "200 + 10%");
        assert_eq!(expr("200 - 10%").as_deref(), Some("(200) * (1 - 10 / 100)"));
        assert_eq!(
            expr("(1 + 1) + 50%").as_deref(),
            Some("((1 + 1)) * (1 + 50 / 100)")
        );
        assert_eq!(expr("80 * 15%").as_deref(), Some("80 * (15 / 100)"));
        assert_eq!(expr("15% of 80").as_deref(), Some("15% of 80"));
    }

    #[test]
    fn unit_conversions() {
        for (query, want) in [
            ("5 mi to km", "5 mi to km"),
            ("3 cups in ml", "3 cups in ml"),
            ("1 in to cm", "1 inch to cm"),
            ("5 in in cm", "5 inch in cm"),
            ("5 ft 11 in to cm", "5 ft 11 inch to cm"),
            ("1 mb in kb", "1 MB in kB"),
            ("2 gib to mb", "2 GiB to MB"),
            ("1 sq ft in m^2", "1 ft^2 in m^2"),
            ("1 fl oz in ml", "1 floz in ml"),
            ("1 c to mph", "1 c to mph"),
            ("90 min to hours", "90 min to hours"),
        ] {
            let p = prepare(query).unwrap_or_else(|| panic!("{query:?}"));
            assert_eq!(p.expr, want, "{query:?}");
            assert!(p.conversion);
        }
        // Nothing to convert to.
        assert_eq!(prepare("2 in 1"), None);
        assert_eq!(prepare("1 to 10"), None);
    }

    #[test]
    fn temperatures() {
        for (query, want) in [
            ("72 f to c", "72 °F to °C"),
            ("72f to c", "72 °F to °C"),
            ("100 c in f", "100 °C to °F"),
            ("-40 c to f", "-40 °C to °F"),
            ("72 degrees f to celsius", "72 °F to °C"),
            ("300 k to c", "300 K to °C"),
            ("72 °F to °C", "72 °F to °C"),
        ] {
            assert_eq!(expr(query).as_deref(), Some(want), "{query:?}");
        }
    }

    #[test]
    fn currencies() {
        for (query, want) in [
            ("100 usd to eur", "100 USD to EUR"),
            ("€50 in $", "50 EUR in USD"),
            ("50€ to £", "50 EUR to GBP"),
            ("$5k to eur", "5k USD to EUR"),
            ("C$100 to usd", "100 CAD to USD"),
            ("100 dollars in euros", "100 dollars in euros"),
            ("1000 yen to usd", "1000 JPY to USD"),
            ("usd to eur", "1 USD to EUR"),
            ("$20 * 3", "20 USD * 3"),
        ] {
            let p = prepare(query).unwrap_or_else(|| panic!("{query:?}"));
            assert_eq!(p.expr, want, "{query:?}");
            assert!(p.currency, "{query:?}");
        }
        // Lower-case words that are also currency codes stay words.
        let p = prepare("3 cup to ml").unwrap();
        assert!(!p.currency);
        assert_eq!(p.expr, "3 cup to ml");
    }

    #[test]
    fn bases() {
        let p = prepare("255 to hex").unwrap();
        assert_eq!((p.expr.as_str(), p.base), ("255 to hex", Some(Base::Hex)));
        assert_eq!(prepare("10 in binary").unwrap().base, Some(Base::Binary));
        assert_eq!(prepare("8 as oct").unwrap().base, Some(Base::Octal));
        assert_eq!(expr("0xff").as_deref(), Some("0xff to decimal"));
        assert_eq!(expr("0b1010").as_deref(), Some("0b1010 to decimal"));
        assert_eq!(expr("ff in decimal").as_deref(), Some("0xff in decimal"));
        assert_eq!(prepare("0xff to decimal").unwrap().base, None);
    }

    #[test]
    fn equals_forces_an_answer() {
        let p = prepare("=pi").unwrap();
        assert!(p.forced);
        assert_eq!(p.expr, "pi");
        assert!(prepare("=2024-01-01").is_some());
        assert_eq!(prepare("="), None);
    }

    #[test]
    fn rejects_what_fend_shouldnt_see() {
        for query in [
            "x = 5",
            "a == b",
            "1 + 1; 2",
            "\\x.x",
            "x:x+1",
            "#1 + 1",
            "5 > 3",
            "\"1\" + 1",
        ] {
            assert_eq!(prepare(query), None, "{query:?}");
        }
        let long = "1+".repeat(60) + "1";
        assert_eq!(prepare(&long), None);
        let deep = "(".repeat(20) + "1" + &")".repeat(20) + "+1";
        assert_eq!(prepare(&deep), None);
    }
}
