//! Turning fend's answer into the row's title and the text Enter copies.

use super::parse::{Base, Prepared};

/// Longer answers (huge factorials) aren't worth a row.
const MAX_CHARS: usize = 5000;
/// Integer parts longer than this get thousands separators in the title.
const GROUP_FROM: usize = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Answer {
    /// "≈ 22.2222 °C", "65,536".
    pub title: String,
    /// What Enter copies: no "≈", no separators ("22.2222 °C", "65536").
    pub text: String,
}

/// Formats fend's `raw` answer to `prepared`. `None` when there's nothing
/// worth showing (a function, the query echoed back, a huge number).
pub fn answer(raw: &str, prepared: &Prepared) -> Option<Answer> {
    let (approx, value) = match raw.strip_prefix("approx. ") {
        Some(v) => (true, v.trim()),
        None => (false, raw.trim()),
    };
    // Functions print as lambdas ("\x.(x^(1/2))"); multi-line output is
    // for fend's own terminal.
    if value.is_empty() || value.contains(['\\', '\n']) || value.chars().count() > MAX_CHARS {
        return None;
    }

    let (text, title_number, rest) = match prepared.base {
        Some(base) => (with_prefix(value, base), None, ""),
        None => match split_number(value) {
            Some((number, rest)) => {
                let unit = rest.trim();
                let number = if prepared.currency && !unit.is_empty() {
                    round(number, Rounding::Cents)
                } else if approx && !unit.is_empty() {
                    round(number, Rounding::Readable)
                } else {
                    number.to_owned()
                };
                (format!("{number}{rest}"), Some(number), rest)
            }
            None => (value.to_owned(), None, ""),
        },
    };
    if !prepared.forced && squash(&text) == squash(&prepared.shown) {
        return None;
    }
    let shown = match title_number {
        Some(number) => format!("{}{rest}", group(&number)),
        None => text.clone(),
    };
    let title = if approx { format!("≈ {shown}") } else { shown };
    Some(Answer { title, text })
}

/// "ff" → "0xff" when the answer is in that base.
fn with_prefix(value: &str, base: Base) -> String {
    let (prefix, radix) = match base {
        Base::Hex => ("0x", 16),
        Base::Binary => ("0b", 2),
        Base::Octal => ("0o", 8),
    };
    let digits = value.trim_start_matches('-');
    if !digits.is_empty() && digits.chars().all(|c| c.is_digit(radix)) {
        let sign = if value.starts_with('-') { "-" } else { "" };
        format!("{sign}{prefix}{digits}")
    } else {
        value.to_owned()
    }
}

/// Splits "-8.04672 km" into ("-8.04672", " km"). `None` if it doesn't
/// start with a plain decimal number.
fn split_number(value: &str) -> Option<(&str, &str)> {
    let digits_from = usize::from(value.starts_with('-'));
    let mut end = digits_from;
    let mut seen_digit = false;
    let mut seen_point = false;
    for (i, c) in value.char_indices().skip(digits_from) {
        if c.is_ascii_digit() {
            seen_digit = true;
        } else if c == '.' && !seen_point {
            seen_point = true;
        } else {
            end = i;
            break;
        }
        end = i + c.len_utf8();
    }
    let rest = &value[end..];
    // "0x100", "12a": not a number followed by a unit.
    let unit_starts_cleanly = rest.is_empty() || rest.starts_with(char::is_whitespace);
    (seen_digit && unit_starts_cleanly && !value[..end].ends_with('.')).then(|| (&value[..end], rest))
}

#[derive(Clone, Copy)]
enum Rounding {
    /// Money: two decimals.
    Cents,
    /// Approximate conversions: four decimals, or four significant digits
    /// below 1 ("22.2222 °C", "0.0003861 mi^2").
    Readable,
}

fn round(number: &str, rounding: Rounding) -> String {
    let Ok(x) = number.parse::<f64>() else {
        return number.to_owned();
    };
    if !x.is_finite() || x.abs() >= 1e15 {
        return number.to_owned();
    }
    let decimals = match rounding {
        Rounding::Cents => 2,
        Rounding::Readable if x.abs() >= 1.0 || x == 0.0 => 4,
        Rounding::Readable => {
            // Leading zeros after the point, plus four significant digits.
            let zeros = (-x.abs().log10()).floor() as usize;
            (zeros + 4).min(12)
        }
    };
    let fixed = format!("{x:.decimals$}");
    match rounding {
        Rounding::Cents => fixed,
        Rounding::Readable => trim_zeros(&fixed),
    }
}

fn trim_zeros(fixed: &str) -> String {
    if !fixed.contains('.') {
        return fixed.to_owned();
    }
    fixed.trim_end_matches('0').trim_end_matches('.').to_owned()
}

/// "1234567.5" → "1,234,567.5"; four digits or fewer stay as they are.
fn group(number: &str) -> String {
    let (sign, unsigned) = match number.strip_prefix('-') {
        Some(rest) => ("-", rest),
        None => ("", number),
    };
    let (int, frac) = match unsigned.split_once('.') {
        Some((i, f)) => (i, Some(f)),
        None => (unsigned, None),
    };
    if int.len() < GROUP_FROM || !int.chars().all(|c| c.is_ascii_digit()) {
        return number.to_owned();
    }
    let mut grouped = String::with_capacity(int.len() + int.len() / 3);
    for (i, c) in int.chars().enumerate() {
        if i > 0 && (int.len() - i) % 3 == 0 {
            grouped.push(',');
        }
        grouped.push(c);
    }
    match frac {
        Some(f) => format!("{sign}{grouped}.{f}"),
        None => format!("{sign}{grouped}"),
    }
}

/// For comparing an answer with the query: no spaces, commas or case.
fn squash(s: &str) -> String {
    s.chars()
        .filter(|c| !c.is_whitespace() && *c != ',')
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::features::calc::parse::prepare;

    fn fmt(query: &str, raw: &str) -> Option<(String, String)> {
        let prepared = prepare(query).expect("a calculation");
        answer(raw, &prepared).map(|a| (a.title, a.text))
    }

    fn pair(title: &str, text: &str) -> Option<(String, String)> {
        Some((title.into(), text.into()))
    }

    #[test]
    fn exact_answers_as_fend_prints_them() {
        assert_eq!(fmt("2^10", "1024"), pair("1024", "1024"));
        assert_eq!(fmt("2^16", "65536"), pair("65,536", "65536"));
        assert_eq!(fmt("5 mi to km", "8.04672 km"), pair("8.04672 km", "8.04672 km"));
        assert_eq!(fmt("10 - 20", "-10"), pair("-10", "-10"));
        assert_eq!(
            fmt("1000000 * 3", "3000000"),
            pair("3,000,000", "3000000")
        );
    }

    #[test]
    fn approximate_answers_are_marked_and_conversions_rounded() {
        assert_eq!(
            fmt("sqrt(2)", "approx. 1.4142135624"),
            pair("≈ 1.4142135624", "1.4142135624")
        );
        assert_eq!(
            fmt("72 f to c", "approx. 22.2222222222 °C"),
            pair("≈ 22.2222 °C", "22.2222 °C")
        );
        assert_eq!(
            fmt("1 m^2 in mi^2", "approx. 0.0000003861021585 mi^2"),
            pair("≈ 0.0000003861 mi^2", "0.0000003861 mi^2")
        );
        assert_eq!(
            fmt("1 c to mph", "approx. 670616629.3843951324 mph"),
            pair("≈ 670,616,629.3844 mph", "670616629.3844 mph")
        );
    }

    #[test]
    fn money_is_rounded_to_cents() {
        assert_eq!(
            fmt("50 eur to usd", "approx. 54.3478260870 USD"),
            pair("≈ 54.35 USD", "54.35 USD")
        );
        assert_eq!(fmt("100 usd to eur", "92 EUR"), pair("92.00 EUR", "92.00 EUR"));
        assert_eq!(
            fmt("1000000 jpy to usd", "approx. 6666.6666666667 USD"),
            pair("≈ 6666.67 USD", "6666.67 USD")
        );
    }

    #[test]
    fn bases_get_their_prefix() {
        assert_eq!(fmt("255 to hex", "ff"), pair("0xff", "0xff"));
        assert_eq!(fmt("10 in binary", "1010"), pair("0b1010", "0b1010"));
        assert_eq!(fmt("8 as octal", "10"), pair("0o10", "0o10"));
        assert_eq!(fmt("0xff", "255"), pair("255", "255"));
    }

    #[test]
    fn nothing_worth_showing() {
        assert_eq!(fmt("1 km to km", "1 km"), pair("1 km", "1 km"));
        let echo = prepare("=5").unwrap();
        assert!(answer("5", &echo).is_some(), "= always answers");
        let p = prepare("2 + 2").unwrap();
        assert_eq!(answer("\\x.x", &p), None);
        assert_eq!(answer("", &p), None);
        assert_eq!(answer(&"9".repeat(MAX_CHARS + 1), &p), None);
        let echoed = Prepared {
            expr: "5 km".into(),
            shown: "5 km".into(),
            currency: false,
            base: None,
            conversion: false,
            forced: false,
        };
        assert_eq!(answer("5 km", &echoed), None, "the query echoed back");
    }

    #[test]
    fn number_splitting() {
        assert_eq!(split_number("8.04672 km"), Some(("8.04672", " km")));
        assert_eq!(split_number("-3"), Some(("-3", "")));
        assert_eq!(split_number("0x100"), None);
        assert_eq!(split_number("0 + i"), Some(("0", " + i")));
        assert_eq!(split_number("km"), None);
        assert_eq!(split_number("1."), None);
    }

    #[test]
    fn grouping() {
        assert_eq!(group("1024"), "1024");
        assert_eq!(group("12345"), "12,345");
        assert_eq!(group("-1234567.891"), "-1,234,567.891");
    }
}
