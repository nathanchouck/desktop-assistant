//! Rewrites that turn how people type calculations into what fend reads:
//! currency symbols, "x" for times, "in" for inches, temperatures, file
//! sizes, desk-calculator percentages.

use super::parse::Base;
use super::tokens::{balanced, join, Kind, Token};

/// Currency codes recognized in lower case. Codes that are also English
/// words or units ("cup", "all", "try", "top") only count in capitals.
const LOWERCASE_CODES: &[&str] = &[
    "aed", "ars", "aud", "bgn", "brl", "cad", "chf", "clp", "cny", "cop", "czk", "dkk", "egp",
    "eur", "gbp", "hkd", "huf", "idr", "ils", "inr", "isk", "jpy", "krw", "kwd", "kzt", "mxn",
    "myr", "ngn", "nok", "nzd", "php", "pkr", "pln", "qar", "ron", "rub", "sar", "sek", "sgd",
    "thb", "twd", "uah", "usd", "vnd", "zar",
];
/// More codes, recognized only in capitals.
const UPPERCASE_CODES: &[&str] = &["TRY", "CUP", "ALL", "TOP", "BOB", "MOP", "PEN", "MAD"];
/// Currency names fend doesn't know, and the codes they stand for.
const CURRENCY_NAMES: &[(&str, &str)] = &[
    ("yen", "JPY"),
    ("yuan", "CNY"),
    ("rupee", "INR"),
    ("rupees", "INR"),
    ("franc", "CHF"),
    ("francs", "CHF"),
    ("bitcoin", "BTC"),
];
/// Currency words fend knows itself.
const FEND_CURRENCY_WORDS: &[&str] = &["dollar", "dollars", "euro", "euros", "cent", "cents"];

/// Turns currency symbols, codes and names into ISO codes after the number
/// ("€50" → "50 EUR"). Returns whether there was any.
pub(super) fn rewrite_money(tokens: &mut Vec<Token>) -> bool {
    let mut found = false;
    let mut out: Vec<Token> = Vec::with_capacity(tokens.len());
    let mut i = 0;
    while i < tokens.len() {
        let t = &tokens[i];
        if let Kind::Money(code) = t.kind {
            found = true;
            let code = Token::new(code, Kind::Word);
            if tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Number) {
                // "$50" → "50 USD"; "$5k" → "5k USD".
                let mut number = tokens[i + 1].clone();
                number.glued = t.glued;
                out.push(number);
                i += 2;
                if tokens.get(i).is_some_and(|s| s.glued && s.is("k")) {
                    out.push(tokens[i].clone());
                    i += 1;
                }
            } else {
                i += 1;
            }
            out.push(code);
            continue;
        }
        let mut t = t.clone();
        if t.kind == Kind::Word {
            let lower = t.text.to_ascii_lowercase();
            if LOWERCASE_CODES.contains(&lower.as_str())
                || UPPERCASE_CODES.contains(&t.text.as_str())
            {
                t.text = t.text.to_ascii_uppercase();
                found = true;
            } else if let Some((_, code)) = CURRENCY_NAMES.iter().find(|(n, _)| *n == lower) {
                t.text = (*code).into();
                found = true;
            } else if FEND_CURRENCY_WORDS.contains(&lower.as_str()) {
                found = true;
            }
        }
        out.push(t);
        i += 1;
    }
    *tokens = out;
    found
}

pub(super) fn is_money_word(t: &Token) -> bool {
    if t.kind != Kind::Word {
        return false;
    }
    let lower = t.text.to_ascii_lowercase();
    UPPERCASE_CODES.contains(&t.text.as_str())
        || LOWERCASE_CODES.contains(&lower.as_str())
        || CURRENCY_NAMES.iter().any(|(_, code)| *code == t.text)
        || FEND_CURRENCY_WORDS.contains(&lower.as_str())
}

/// "sq ft" → "ft^2", "cubic m" → "m^3", "fl oz" → "floz".
pub(super) fn merge_compound_units(tokens: &mut Vec<Token>) {
    let mut i = 0;
    while i + 1 < tokens.len() {
        let next_is_word = tokens[i + 1].kind == Kind::Word;
        let power = if tokens[i].is("sq") || tokens[i].is("square") {
            Some("2")
        } else if tokens[i].is("cu") || tokens[i].is("cubic") {
            Some("3")
        } else {
            None
        };
        if let (Some(power), true) = (power, next_is_word) {
            let unit = tokens.remove(i + 1);
            tokens[i].text = format!("{}^{power}", unit.text);
        } else if tokens[i].is("fl") && tokens[i + 1].is("oz") {
            tokens.remove(i + 1);
            tokens[i].text = "floz".into();
        }
        i += 1;
    }
}

/// Where the conversion keyword is ("5 mi to km"): the last `to`, `as`,
/// `into` or `in` with something on both sides, unless that `in` means
/// inches.
pub(super) fn conversion_keyword(tokens: &[Token]) -> Option<usize> {
    (1..tokens.len().saturating_sub(1))
        .rev()
        .find(|&i| is_keyword(&tokens[i]) && !is_inch(tokens, i))
}

fn is_keyword(t: &Token) -> bool {
    t.is("to") || t.is("as") || t.is("into") || t.is("in")
}

/// An `in` that means inches: after a number, and at the end or before the
/// keyword ("12 in", "1 in to cm"). "10 in binary" is a conversion.
fn is_inch(tokens: &[Token], i: usize) -> bool {
    tokens[i].is("in")
        && i > 0
        && tokens[i - 1].kind == Kind::Number
        && tokens.get(i + 1).is_none_or(is_keyword)
}

/// "12 in" → "12 inch" (fend reads `in` as the keyword).
pub(super) fn rewrite_inches(tokens: &mut [Token], split: Option<usize>) {
    for i in 1..tokens.len() {
        if Some(i) != split && is_inch(tokens, i) {
            tokens[i].text = "inch".into();
        }
    }
}

/// "mb" → "MB": typed in lower case, file sizes mean bytes (fend would
/// read millibits).
pub(super) fn rewrite_bytes(tokens: &mut [Token]) {
    const BYTES: &[(&str, &str)] = &[
        ("kb", "kB"),
        ("mb", "MB"),
        ("gb", "GB"),
        ("tb", "TB"),
        ("pb", "PB"),
        ("kib", "KiB"),
        ("mib", "MiB"),
        ("gib", "GiB"),
        ("tib", "TiB"),
    ];
    for t in tokens.iter_mut().filter(|t| t.kind == Kind::Word) {
        if let Some((_, unit)) = BYTES.iter().find(|(lower, _)| t.text == *lower) {
            t.text = (*unit).into();
        }
    }
}

/// "3 x 4" and "3x4" → "3 × 4". Hex literals ("0x4") are numbers already.
pub(super) fn rewrite_times(tokens: &mut Vec<Token>) {
    let mut i = 1;
    while i < tokens.len() {
        let after_number = tokens[i - 1].kind == Kind::Number;
        if after_number
            && tokens[i].is("x")
            && tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Number)
        {
            tokens[i] = Token::new("×", Kind::Op);
        } else if after_number
            && tokens[i].kind == Kind::Word
            && tokens[i].text.len() > 1
            && tokens[i].text.starts_with(['x', 'X'])
            && tokens[i].text[1..].chars().all(|c| c.is_ascii_digit())
        {
            let number = tokens[i].text[1..].to_owned();
            tokens[i] = Token::new("×", Kind::Op);
            tokens.insert(i + 1, Token::new(number, Kind::Number));
        }
        i += 1;
    }
}

/// The temperature scale `tokens` names, if they're exactly one.
fn temperature(tokens: &[Token]) -> Option<&'static str> {
    let scale = |t: &Token| -> Option<&'static str> {
        let word = t.text.to_lowercase();
        let word = word.trim_start_matches('°');
        match word {
            "f" | "fahrenheit" | "degf" => Some("°F"),
            "c" | "celsius" | "centigrade" | "degc" => Some("°C"),
            "k" | "kelvin" | "kelvins" => Some("K"),
            _ => None,
        }
    };
    match tokens {
        [unit] if unit.kind == Kind::Word => scale(unit),
        [deg, unit]
            if deg.is("deg") || deg.is("degree") || deg.is("degrees") || deg.text == "°" =>
        {
            scale(unit)
        }
        _ => None,
    }
}

/// "72 f to c" → "72 °F to °C": only when both sides are temperatures, so
/// "1 c to mph" (the speed of light) still works. Returns where the
/// keyword is afterwards.
pub(super) fn rewrite_temperatures(tokens: &mut Vec<Token>, k: usize) -> usize {
    let Some(to) = temperature(&tokens[k + 1..]) else {
        return k;
    };
    for len in [2, 1] {
        if k < len + 1 {
            continue;
        }
        let unit_at = k - len;
        if tokens[unit_at - 1].kind != Kind::Number {
            continue;
        }
        if let Some(from) = temperature(&tokens[unit_at..k]) {
            tokens.truncate(unit_at);
            tokens.push(Token::new(from, Kind::Word));
            tokens.push(Token::new("to", Kind::Word));
            tokens.push(Token::new(to, Kind::Word));
            return unit_at + 1;
        }
    }
    k
}

/// "ff in decimal" → "0xff in decimal". Returns whether it did.
pub(super) fn rewrite_hex_word(tokens: &mut [Token], k: usize) -> bool {
    let to_decimal =
        tokens.len() == k + 2 && (tokens[k + 1].is("decimal") || tokens[k + 1].is("dec"));
    let word = &tokens[0];
    if k == 1
        && to_decimal
        && word.kind == Kind::Word
        && word.text.chars().all(|c| c.is_ascii_hexdigit())
    {
        tokens[0] = Token::new(format!("0x{}", word.text), Kind::Number);
        return true;
    }
    false
}

pub(super) fn target_base(rhs: &[Token]) -> Option<Base> {
    match rhs {
        [t] if t.is("hex") || t.is("hexadecimal") => Some(Base::Hex),
        [t] if t.is("bin") || t.is("binary") => Some(Base::Binary),
        [t] if t.is("oct") || t.is("octal") => Some(Base::Octal),
        _ => None,
    }
}

/// Percentages the way a desk calculator does them: "200 + 10%" is 220,
/// "80 * 15%" is 12. fend would add a tenth of a unit and print "1200%".
/// "15% of 80" is fend's own and stays.
pub(super) fn rewrite_percentages(tokens: Vec<Token>) -> Vec<Token> {
    let n = tokens.len();
    let is_percent = |i: usize| -> bool {
        tokens[i].is_op("%")
            && tokens[i].glued
            && i > 0
            && tokens[i - 1].kind == Kind::Number
            && !tokens.get(i + 1).is_some_and(|t| t.is("of"))
    };
    // "<lhs> + N%" at the end, with the + outside any brackets.
    if n >= 4 && is_percent(n - 1) {
        let op = &tokens[n - 3];
        let lhs = &tokens[..n - 3];
        let operand = lhs.last().is_some_and(|t| {
            matches!(t.kind, Kind::Number | Kind::Word) || t.is_op(")") || t.is_op("!")
        });
        if (op.is_op("+") || op.is_op("-")) && operand && balanced(lhs) {
            let expr = format!(
                "({}) * (1 {} {} / 100)",
                join(lhs),
                op.text,
                tokens[n - 2].text
            );
            return vec![Token::new(expr, Kind::Word)];
        }
    }
    let mut out = Vec::with_capacity(n);
    for (i, t) in tokens.iter().enumerate() {
        if i + 1 < n && is_percent(i + 1) {
            let mut fraction = Token::new(format!("({} / 100)", t.text), Kind::Word);
            fraction.glued = t.glued;
            out.push(fraction);
        } else if !is_percent(i) {
            out.push(t.clone());
        }
    }
    out
}
