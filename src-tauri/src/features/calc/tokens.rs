//! Splitting a query into numbers, words, operators and currency symbols,
//! and joining them back with the spacing they had.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Number,
    Word,
    Op,
    /// A currency symbol, with the ISO code it stands for.
    Money(&'static str),
}

#[derive(Debug, Clone)]
pub(super) struct Token {
    pub text: String,
    pub kind: Kind,
    /// No space before it ("50€", "3x4"): joined back the same way.
    pub glued: bool,
}

impl Token {
    pub fn new(text: impl Into<String>, kind: Kind) -> Self {
        Self {
            text: text.into(),
            kind,
            glued: false,
        }
    }

    pub fn is(&self, word: &str) -> bool {
        self.kind == Kind::Word && self.text.eq_ignore_ascii_case(word)
    }

    pub fn is_op(&self, op: &str) -> bool {
        self.kind == Kind::Op && self.text == op
    }
}

/// Splits the query into numbers, words, operators and currency symbols.
/// `None` for characters that never appear in a calculation.
pub(super) fn tokenize(query: &str) -> Option<Vec<Token>> {
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
            while i < chars.len() && (chars[i].is_alphanumeric() || matches!(chars[i], '_' | '°'))
            {
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

/// How deep the brackets go.
pub(super) fn depth(tokens: &[Token]) -> usize {
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

/// Brackets close as often as they open, never going below zero.
pub(super) fn balanced(tokens: &[Token]) -> bool {
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

pub(super) fn join(tokens: &[Token]) -> String {
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

    fn kinds(query: &str) -> Vec<(String, Kind)> {
        tokenize(query)
            .unwrap()
            .into_iter()
            .map(|t| (t.text, t.kind))
            .collect()
    }

    #[test]
    fn splits_numbers_words_and_operators() {
        assert_eq!(
            kinds("1,000.5e-3 km+0x1F"),
            [
                ("1,000.5e-3".into(), Kind::Number),
                ("km".into(), Kind::Word),
                ("+".into(), Kind::Op),
                ("0x1F".into(), Kind::Number),
            ]
        );
        assert_eq!(
            kinds("C$5 €"),
            [
                ("C$".into(), Kind::Money("CAD")),
                ("5".into(), Kind::Number),
                ("€".into(), Kind::Money("EUR")),
            ]
        );
        assert_eq!(kinds("max(1,2)").len(), 6);
        assert!(tokenize("a;b").is_none());
    }

    #[test]
    fn joins_with_the_original_spacing() {
        for query in ["2^10", "3 × 4", "50€ to $", "sqrt(2) + 1", "1,000 + 1"] {
            assert_eq!(join(&tokenize(query).unwrap()), query);
        }
    }

    #[test]
    fn bracket_depth_and_balance() {
        let tokens = tokenize("((1)+(2))").unwrap();
        assert_eq!(depth(&tokens), 2);
        assert!(balanced(&tokens));
        assert!(!balanced(&tokenize(")1(").unwrap()));
    }
}
