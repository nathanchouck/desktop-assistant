//! Snippet placeholders: `{clipboard}`, `{date}`, `{time}`, `{datetime}`.
//! Anything else in braces is left as typed.

use chrono::NaiveDateTime;

const CLIPBOARD: &str = "{clipboard}";

/// Whether `text` uses `{clipboard}`, so the clipboard is only read when
/// it's needed.
pub fn needs_clipboard(text: &str) -> bool {
    text.contains(CLIPBOARD)
}

/// Fills in the placeholders. `clipboard` is the clipboard's text (empty
/// when it holds none); `now` is local time.
pub fn expand(text: &str, clipboard: &str, now: NaiveDateTime) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            rest = after;
            break;
        };
        let token = &after[..=close];
        match token {
            CLIPBOARD => out.push_str(clipboard),
            "{date}" => out.push_str(&now.format("%Y-%m-%d").to_string()),
            "{time}" => out.push_str(&now.format("%H:%M").to_string()),
            "{datetime}" => out.push_str(&now.format("%Y-%m-%d %H:%M").to_string()),
            _ => {
                // Not ours: keep the brace and look again just after it,
                // so "{{date}" still finds "{date}".
                out.push('{');
                rest = &after[1..];
                continue;
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn now() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 3, 7)
            .and_then(|d| d.and_hms_opt(9, 5, 0))
            .unwrap()
    }

    #[test]
    fn fills_in_placeholders() {
        assert_eq!(
            expand("Today is {date} at {time} ({datetime})", "", now()),
            "Today is 2026-03-07 at 09:05 (2026-03-07 09:05)"
        );
        assert_eq!(expand("> {clipboard}!", "copied", now()), "> copied!");
        assert_eq!(expand("{clipboard}", "", now()), "");
    }

    #[test]
    fn leaves_everything_else_alone() {
        let code = "fn main() { println!(\"{}\", x); }";
        assert_eq!(expand(code, "c", now()), code);
        assert_eq!(expand("{cursor} {Date} {", "", now()), "{cursor} {Date} {");
        assert_eq!(expand("{{date}}", "", now()), "{2026-03-07}");
        assert_eq!(expand("✓ {date} ✓", "", now()), "✓ 2026-03-07 ✓");
        assert_eq!(expand("", "", now()), "");
    }

    #[test]
    fn knows_when_the_clipboard_is_needed() {
        assert!(needs_clipboard("Re: {clipboard}"));
        assert!(!needs_clipboard("{date}"));
    }
}
