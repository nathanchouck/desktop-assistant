//! Whether a query is a calculation, and rewriting it into something fend
//! understands ("72 f to c" → "72 °F to °C", "€50 in $" → "50 EUR in USD").
//!
//! The answer row always ranks first, so a false positive steals Enter from
//! the app or file you meant. The rules lean towards silence: a query needs a
//! digit and an operation (an operator, a conversion, a function), and things
//! shaped like dates, versions and phone numbers are left alone. Starting the
//! query with `=` skips those checks.

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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Number,
    Word,
    Op,
    /// A currency symbol, with the ISO code it stands for.
    Money(&'static str),
}

#[derive(Debug, Clone)]
struct Token {
    text: String,
    kind: Kind,
    /// No space before it ("50€", "3x4"): joined back the same way.
    glued: bool,
}

impl Token {
    fn new(text: impl Into<String>, kind: Kind) -> Self {
        Self {
            text: text.into(),
            kind,
            glued: false,
        }
    }

    fn is(&self, word: &str) -> bool {
        self.kind == Kind::Word && self.text.eq_ignore_ascii_case(word)
    }

    fn is_op(&self, op: &str) -> bool {
        self.kind == Kind::Op && self.text == op
    }
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
    let seps: Vec<char> = query.chars().filter(|c| matches!(c, '-' | '.' | '/')).collect();
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
    let padded = groups[1..].iter().any(|g| g.len() >= 2 && g.starts_with('0'));
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

/// Splits the query into numbers, words, operators and currency symbols.
/// `None` for characters that never appear in a calculation.
fn tokenize(query: &str) -> Option<Vec<Token>> {
    let chars: Vec<char> = query.chars().collect();
    let mut tokens = Vec::new();
    let mut glued = false;
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c.is_whitespace() {
            glued = false;
            i += 1;
            continue;
        }
        let start = i;
        let kind = if c.is_ascii_digit() || (c == '.' && next_is_digit(&chars, i)) {
            i = scan_number(&chars, i);
            Kind::Number
        } else if c.is_alphabetic() || c == '°' || c == '_' {
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '°')) {
                i += 1;
            }
            // "C$", "US$": letters glued to a dollar sign.
            let word: String = chars[start..i].iter().collect();
            match (chars.get(i), dollar_code(&word)) {
                (Some('$'), Some(code)) => {
                    i += 1;
                    Kind::Money(code)
                }
                _ => Kind::Word,
            }
        } else if let Some(code) = symbol_code(c) {
            i += 1;
            Kind::Money(code)
        } else if c == '*' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            Kind::Op
        } else if "+-*/^×÷!%(),".contains(c) {
            i += 1;
            Kind::Op
        } else if matches!(c, '−' | '–') {
            // A typographic minus.
            tokens.push(Token {
                text: "-".into(),
                kind: Kind::Op,
                glued,
            });
            glued = true;
            i += 1;
            continue;
        } else {
            return None;
        };
        tokens.push(Token {
            text: chars[start..i].iter().collect(),
            kind,
            glued,
        });
        glued = true;
    }
    Some(tokens)
}

fn next_is_digit(chars: &[char], i: usize) -> bool {
    chars.get(i + 1).is_some_and(char::is_ascii_digit)
}

/// The end of the number starting at `i`: `0x1f`/`0b101`/`0o17`, or
/// `1,000.5e-3`.
fn scan_number(chars: &[char], mut i: usize) -> usize {
    let radix = match (chars[i], chars.get(i + 1)) {
        ('0', Some('x' | 'X')) => 16,
        ('0', Some('b' | 'B')) => 2,
        ('0', Some('o' | 'O')) => 8,
        _ => 10,
    };
    if radix != 10 && chars.get(i + 2).is_some_and(|c| c.is_digit(radix)) {
        i += 2;
        while i < chars.len() && chars[i].is_digit(radix) {
            i += 1;
        }
        return i;
    }
    while i < chars.len() {
        let c = chars[i];
        let thousands = c == ',' && thousands_group(chars, i + 1);
        if c.is_ascii_digit() || thousands || (c == '.' && next_is_digit(chars, i)) {
            i += 1;
        } else if matches!(c, 'e' | 'E') {
            let sign = matches!(chars.get(i + 1), Some('+' | '-'));
            let digit_at = i + 1 + usize::from(sign);
            if chars.get(digit_at).is_some_and(char::is_ascii_digit) {
                i = digit_at;
            } else {
                break;
            }
        } else {
            break;
        }
    }
    i
}

/// Three digits and then no more: "1,000" (not the "1,2" of `max(1,2)`).
fn thousands_group(chars: &[char], at: usize) -> bool {
    (at..at + 3).all(|j| chars.get(j).is_some_and(char::is_ascii_digit))
        && !chars.get(at + 3).is_some_and(char::is_ascii_digit)
}

fn symbol_code(c: char) -> Option<&'static str> {
    Some(match c {
        '$' => "USD",
        '€' => "EUR",
        '£' => "GBP",
        '¥' => "JPY",
        '₹' => "INR",
        '₩' => "KRW",
        '₽' => "RUB",
        '₪' => "ILS",
        '₺' => "TRY",
        '₱' => "PHP",
        '฿' => "THB",
        '₴' => "UAH",
        '₫' => "VND",
        _ => return None,
    })
}

fn dollar_code(prefix: &str) -> Option<&'static str> {
    Some(match prefix.to_ascii_uppercase().as_str() {
        "US" => "USD",
        "C" | "CA" => "CAD",
        "A" | "AU" => "AUD",
        "NZ" => "NZD",
        "HK" => "HKD",
        "S" | "SG" => "SGD",
        "R" => "BRL",
        "MX" => "MXN",
        _ => return None,
    })
}

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
fn rewrite_money(tokens: &mut Vec<Token>) -> bool {
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
            if LOWERCASE_CODES.contains(&lower.as_str()) || UPPERCASE_CODES.contains(&t.text.as_str())
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

fn is_money_word(t: &Token) -> bool {
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
fn merge_compound_units(tokens: &mut Vec<Token>) {
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
fn conversion_keyword(tokens: &[Token]) -> Option<usize> {
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
fn rewrite_inches(tokens: &mut [Token], split: Option<usize>) {
    for i in 1..tokens.len() {
        if Some(i) != split && is_inch(tokens, i) {
            tokens[i].text = "inch".into();
        }
    }
}

/// "mb" → "MB": typed in lower case, file sizes mean bytes (fend would
/// read millibits).
fn rewrite_bytes(tokens: &mut [Token]) {
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
fn rewrite_times(tokens: &mut Vec<Token>) {
    let mut i = 1;
    while i < tokens.len() {
        let after_number = tokens[i - 1].kind == Kind::Number;
        if after_number && tokens[i].is("x") && tokens.get(i + 1).is_some_and(|n| n.kind == Kind::Number)
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
fn rewrite_temperatures(tokens: &mut Vec<Token>, k: usize) -> usize {
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
fn rewrite_hex_word(tokens: &mut [Token], k: usize) -> bool {
    let to_decimal =
        tokens.len() == k + 2 && (tokens[k + 1].is("decimal") || tokens[k + 1].is("dec"));
    let word = &tokens[0];
    if k == 1 && to_decimal && word.kind == Kind::Word && word.text.chars().all(|c| c.is_ascii_hexdigit())
    {
        tokens[0] = Token::new(format!("0x{}", word.text), Kind::Number);
        return true;
    }
    false
}

fn target_base(rhs: &[Token]) -> Option<Base> {
    match rhs {
        [t] if t.is("hex") || t.is("hexadecimal") => Some(Base::Hex),
        [t] if t.is("bin") || t.is("binary") => Some(Base::Binary),
        [t] if t.is("oct") || t.is("octal") => Some(Base::Octal),
        _ => None,
    }
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

/// How deep the brackets go.
fn depth(tokens: &[Token]) -> usize {
    let mut level = 0usize;
    let mut deepest = 0;
    for t in tokens {
        if t.is_op("(") {
            level += 1;
            deepest = deepest.max(level);
        } else if t.is_op(")") {
            level = level.saturating_sub(1);
        }
    }
    deepest
}

/// Percentages the way a desk calculator does them: "200 + 10%" is 220,
/// "80 * 15%" is 12. fend would add a tenth of a unit and print "1200%".
/// "15% of 80" is fend's own and stays.
fn rewrite_percentages(tokens: Vec<Token>) -> Vec<Token> {
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
        let operand = lhs
            .last()
            .is_some_and(|t| matches!(t.kind, Kind::Number | Kind::Word) || t.is_op(")") || t.is_op("!"));
        if (op.is_op("+") || op.is_op("-")) && operand && balanced(lhs) {
            let expr = format!("({}) * (1 {} {} / 100)", join(lhs), op.text, tokens[n - 2].text);
            return vec![Token::new(expr, Kind::Word)];
        }
    }
    let mut out = Vec::with_capacity(n);
    for i in 0..n {
        if i + 1 < n && is_percent(i + 1) {
            let mut t = Token::new(format!("({} / 100)", tokens[i].text), Kind::Word);
            t.glued = tokens[i].glued;
            out.push(t);
        } else if is_percent(i) {
            continue;
        } else {
            out.push(tokens[i].clone());
        }
    }
    out
}

/// Brackets close as often as they open, never going below zero.
fn balanced(tokens: &[Token]) -> bool {
    let mut level = 0i32;
    for t in tokens {
        if t.is_op("(") {
            level += 1;
        } else if t.is_op(")") {
            level -= 1;
            if level < 0 {
                return false;
            }
        }
    }
    level == 0
}

fn join(tokens: &[Token]) -> String {
    let mut text = String::new();
    for (i, t) in tokens.iter().enumerate() {
        if i > 0 && !t.glued {
            text.push(' ');
        }
        text.push_str(&t.text);
    }
    text
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
        for query in ["c 2+2", "g 5 + 5", "f 2024-1", "d 10 * 3", "x 2*3", "k-9", "t-1000"] {
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
        assert_eq!(expr("(1 + 1) + 50%").as_deref(), Some("((1 + 1)) * (1 + 50 / 100)"));
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
