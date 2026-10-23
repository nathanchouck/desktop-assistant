//! Prompt templates for saved AI commands. Placeholders:
//! - `{selection}`: what's selected in the window you came from (copied
//!   with Ctrl+C, the clipboard put back after);
//! - `{clipboard}`: the clipboard's text;
//! - `{text}`: what you type after the command's name or keyword;
//! - `{date}` (2026-10-23), `{time}` (14:05), `{datetime}`.
//!
//! `{text|clipboard}` takes the first one that isn't empty. Anything else
//! in braces is left as typed, so prompts can hold code.

use chrono::NaiveDateTime;

/// Where a placeholder's text comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    Selection,
    Clipboard,
    Text,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Part {
    From(Source),
    Date,
    Time,
    DateTime,
}

/// What has to be fetched before a template can be filled.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct Needs {
    pub selection: bool,
    pub clipboard: bool,
    /// The template uses `{text}` somewhere.
    pub text: bool,
}

/// The values to fill in. `None` (or only spaces) counts as empty.
#[derive(Debug, Clone, Copy)]
pub struct Values<'a> {
    pub selection: Option<&'a str>,
    pub clipboard: Option<&'a str>,
    pub text: Option<&'a str>,
    /// Local time.
    pub now: NaiveDateTime,
}

impl Values<'_> {
    fn get(&self, source: Source) -> Option<&str> {
        let value = match source {
            Source::Selection => self.selection,
            Source::Clipboard => self.clipboard,
            Source::Text => self.text,
        };
        value.filter(|v| !v.trim().is_empty())
    }
}

/// A placeholder had nothing to fill it: the sources it would take, in
/// order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Missing(pub Vec<Source>);

/// The parts of a placeholder's inside (`text|clipboard`), or `None` if
/// it isn't one of ours.
fn parse(inside: &str) -> Option<Vec<Part>> {
    inside
        .split('|')
        .map(|name| match name.trim() {
            "selection" => Some(Part::From(Source::Selection)),
            "clipboard" => Some(Part::From(Source::Clipboard)),
            "text" => Some(Part::From(Source::Text)),
            "date" => Some(Part::Date),
            "time" => Some(Part::Time),
            "datetime" => Some(Part::DateTime),
            _ => None,
        })
        .collect()
}

/// Splits `template` into literal text and placeholders.
fn pieces(template: &str) -> Vec<Result<&str, Vec<Part>>> {
    let mut out = Vec::new();
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        let after = &rest[open..];
        let Some(close) = after.find('}') else {
            break;
        };
        match parse(&after[1..close]) {
            Some(parts) => {
                out.push(Ok(&rest[..open]));
                out.push(Err(parts));
                rest = &after[close + 1..];
            }
            None => {
                // Not ours: keep the brace and look again just after it,
                // so "{{text}" still finds "{text}".
                out.push(Ok(&rest[..=open]));
                rest = &after[1..];
            }
        }
    }
    out.push(Ok(rest));
    out
}

/// What filling `template` will ask for, when you typed text after the
/// name (`typed`). A source after one that's sure to be there (the typed
/// text) isn't needed; the selection is only grabbed when it might be used.
pub fn needs(template: &str, typed: bool) -> Needs {
    let mut needs = Needs::default();
    for piece in pieces(template) {
        let Err(parts) = piece else { continue };
        for part in parts {
            match part {
                Part::From(Source::Text) => {
                    needs.text = true;
                    if typed {
                        break;
                    }
                }
                Part::From(Source::Selection) => needs.selection = true,
                Part::From(Source::Clipboard) => needs.clipboard = true,
                Part::Date | Part::Time | Part::DateTime => break,
            }
        }
    }
    needs
}

/// Whether the template can't do without typed text: a placeholder takes
/// nothing but `{text}` ("Search Drive {text}").
pub fn requires_text(template: &str) -> bool {
    pieces(template).into_iter().any(
        |piece| matches!(piece, Err(parts) if parts.iter().all(|p| *p == Part::From(Source::Text))),
    )
}

/// Fills in the placeholders, or says which one had nothing to use.
pub fn fill(template: &str, values: &Values) -> Result<String, Missing> {
    let mut out = String::with_capacity(template.len());
    for piece in pieces(template) {
        match piece {
            Ok(text) => out.push_str(text),
            Err(parts) => {
                let value = parts.iter().find_map(|part| match part {
                    Part::From(source) => values.get(*source).map(str::to_owned),
                    Part::Date => Some(values.now.format("%Y-%m-%d").to_string()),
                    Part::Time => Some(values.now.format("%H:%M").to_string()),
                    Part::DateTime => Some(values.now.format("%Y-%m-%d %H:%M").to_string()),
                });
                match value {
                    Some(value) => out.push_str(value.trim()),
                    None => {
                        return Err(Missing(
                            parts
                                .iter()
                                .filter_map(|p| match p {
                                    Part::From(source) => Some(*source),
                                    _ => None,
                                })
                                .collect(),
                        ))
                    }
                }
            }
        }
    }
    Ok(out)
}

/// Placeholders a prompt can use, for the settings menu.
pub const PLACEHOLDERS: &[&str] = &[
    "{selection}",
    "{clipboard}",
    "{text}",
    "{date}",
    "{time}",
    "{text|clipboard}",
];

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::NaiveDate;

    fn values<'a>(
        selection: Option<&'a str>,
        clipboard: Option<&'a str>,
        text: Option<&'a str>,
    ) -> Values<'a> {
        Values {
            selection,
            clipboard,
            text,
            now: NaiveDate::from_ymd_opt(2026, 10, 23)
                .and_then(|d| d.and_hms_opt(9, 5, 0))
                .unwrap(),
        }
    }

    #[test]
    fn fills_each_placeholder() {
        let v = values(Some("sel"), Some("clip"), Some("typed"));
        assert_eq!(
            fill(
                "{selection} {clipboard} {text} {date} {time} {datetime}",
                &v
            ),
            Ok("sel clip typed 2026-10-23 09:05 2026-10-23 09:05".into())
        );
        // Values are trimmed: a copied line keeps no trailing newline.
        let v = values(None, Some("  line\n"), None);
        assert_eq!(
            fill("Explain: {clipboard}.", &v),
            Ok("Explain: line.".into())
        );
    }

    #[test]
    fn alternatives_take_the_first_non_empty() {
        let typed = values(None, Some("clip"), Some("typed"));
        assert_eq!(
            fill("Fix: {text|clipboard}", &typed),
            Ok("Fix: typed".into())
        );
        let not_typed = values(None, Some("clip"), Some("  "));
        assert_eq!(
            fill("Fix: {text | clipboard}", &not_typed),
            Ok("Fix: clip".into())
        );
        let nothing = values(None, Some(""), None);
        assert_eq!(
            fill("Fix: {text|clipboard}", &nothing),
            Err(Missing(vec![Source::Text, Source::Clipboard]))
        );
        assert_eq!(
            fill("{selection}", &nothing),
            Err(Missing(vec![Source::Selection]))
        );
    }

    #[test]
    fn leaves_other_braces_alone() {
        let v = values(None, None, Some("x"));
        let code = "fn main() { println!(\"{}\", y); } {Text} {text|nope} {";
        assert_eq!(fill(code, &v), Ok(code.into()));
        assert_eq!(fill("{{text}}", &v), Ok("{x}".into()));
        assert_eq!(fill("✓ {text} ✓", &v), Ok("✓ x ✓".into()));
        assert_eq!(fill("", &v), Ok(String::new()));
    }

    #[test]
    fn knows_when_text_must_be_typed() {
        assert!(requires_text("Search Drive for {text}"));
        assert!(!requires_text("Fix {text|clipboard}"));
        assert!(!requires_text("Summarize {clipboard}"));
        assert!(!requires_text("{Text}"));
    }

    #[test]
    fn knows_what_to_fetch() {
        assert!(needs("Summarize {clipboard}", false).clipboard);
        assert!(!needs("Summarize {clipboard}", false).selection);
        // Typed text comes first, so the clipboard isn't needed then.
        assert_eq!(
            needs("Fix {text|clipboard}", true),
            Needs {
                text: true,
                ..Needs::default()
            }
        );
        assert_eq!(
            needs("Fix {text|clipboard}", false),
            Needs {
                text: true,
                clipboard: true,
                ..Needs::default()
            }
        );
        assert!(needs("Explain {selection|clipboard}", false).selection);
        assert_eq!(needs("Today {date}", false), Needs::default());
        assert_eq!(needs("{date|selection}", false), Needs::default());
    }
}
