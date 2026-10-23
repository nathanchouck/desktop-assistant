//! Saved AI commands: a name, a prompt template (see `template.rs`), which
//! AI answers, and an optional keyword. They're found by name or keyword;
//! text typed after either fills `{text}` ("fix grammar teh cat"). Pure,
//! so matching and ids are tested here.

use serde::{Deserialize, Serialize};

use super::template;
use crate::ai::ProviderId;
use crate::features::system::matching;
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, text_icon, Action, ItemKind, ResultItem};

/// `ai-cmd:<name>`, or `ai-cmd:<name>\t<text>` with typed text (a tab can't
/// be typed in the search box: it asks the AI).
pub const PREFIX: &str = "ai-cmd:";
const TEXT_SEPARATOR: char = '\t';
/// "fix grammar teh cat": you typed the name on purpose, so above apps.
const TEXT_SCORE: f32 = 1.5;
/// The same through a keyword ("fix teh cat").
const KEYWORD_TEXT_SCORE: f32 = 1.4;
/// Subtitles show this much of the prompt or the typed text.
const PREVIEW_CHARS: usize = 60;

/// Which AI answers a saved command.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CommandAi {
    /// The default AI (Settings → ChatGPT → Default AI).
    #[default]
    Default,
    Claude,
    Chatgpt,
}

impl CommandAi {
    /// For `Activation::Ask`: `None` asks the default AI.
    pub fn provider(self) -> Option<ProviderId> {
        match self {
            CommandAi::Default => None,
            CommandAi::Claude => Some(ProviderId::Claude),
            CommandAi::Chatgpt => Some(ProviderId::Chatgpt),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct AiCommand {
    /// What you type to find it ("Fix Grammar"); also its id, so keep
    /// names unique.
    pub name: String,
    /// A short word that finds it too ("fix"). May be empty.
    pub keyword: String,
    /// What's sent, with placeholders: {selection}, {clipboard}, {text},
    /// {date}, {time}; {text|clipboard} takes the first that isn't empty.
    pub prompt: String,
    pub provider: CommandAi,
    /// A glyph name (sparkle, calendar, mail, drive, ...) or an emoji.
    /// Empty: the AI's own symbol.
    pub icon: String,
}

fn command(name: &str, keyword: &str, provider: CommandAi, icon: &str, prompt: &str) -> AiCommand {
    AiCommand {
        name: name.into(),
        keyword: keyword.into(),
        prompt: prompt.into(),
        provider,
        icon: icon.into(),
    }
}

/// The commands a new install starts with.
pub fn defaults() -> Vec<AiCommand> {
    use CommandAi::{Claude, Default};
    vec![
        command(
            "Fix Grammar",
            "fix",
            Default,
            "",
            "Fix the spelling and grammar of this text. Keep its meaning, tone and formatting, \
             and reply with only the corrected text.\n\n{text|clipboard}",
        ),
        command(
            "Summarize Clipboard",
            "",
            Default,
            "",
            "Summarize this in a few short bullet points:\n\n{clipboard}",
        ),
        command(
            "Explain Clipboard",
            "",
            Default,
            "",
            "Explain this in plain words:\n\n{clipboard}",
        ),
        command(
            "Translate to English",
            "",
            Default,
            "",
            "Translate this to English. Reply with only the translation.\n\n{text|clipboard}",
        ),
        command(
            "Make It Shorter",
            "",
            Default,
            "",
            "Make this shorter without losing anything important. Keep its tone, and reply \
             with only the new text.\n\n{text|clipboard}",
        ),
        command(
            "Today's Calendar",
            "",
            Claude,
            "calendar",
            "What's on my Google Calendar today? Use the Google Calendar connector.",
        ),
        command(
            "Unread Email",
            "",
            Claude,
            "mail",
            "What unread emails do I have in Gmail? List the important ones first, with who \
             sent each and what they want. Use the Gmail connector.",
        ),
        command(
            "Search Drive",
            "drive",
            Claude,
            "drive",
            "Search my Google Drive for: {text}\nList the best matches with their links. Use \
             the Google Drive connector.",
        ),
    ]
}

/// The id of a command's row, with the text typed after its name.
pub fn command_id(name: &str, text: Option<&str>) -> String {
    match text {
        Some(text) => format!("{PREFIX}{name}{TEXT_SEPARATOR}{text}"),
        None => format!("{PREFIX}{name}"),
    }
}

/// (name, typed text) from a row id.
pub fn parse_id(id: &str) -> Option<(&str, Option<&str>)> {
    let rest = id.strip_prefix(PREFIX)?;
    Some(match rest.split_once(TEXT_SEPARATOR) {
        Some((name, text)) => (name, Some(text)),
        None => (rest, None),
    })
}

/// The text after `head` in `query` ("fix grammar teh cat" after "Fix
/// Grammar"), ignoring case; `None` unless a space and some text follow.
pub fn text_after<'q>(query: &'q str, head: &str) -> Option<&'q str> {
    let head = head.trim();
    if head.is_empty() {
        return None;
    }
    let mut rest = query.char_indices();
    for h in head.chars() {
        let (_, q) = rest.next()?;
        if !q.to_lowercase().eq(h.to_lowercase()) {
            return None;
        }
    }
    let (i, sep) = rest.next()?;
    if !sep.is_whitespace() {
        return None;
    }
    let text = query[i..].trim();
    (!text.is_empty()).then_some(text)
}

/// A row's icon: the command's glyph name or emoji, else the AI's symbol.
pub fn icon(command: &AiCommand, default: ProviderId) -> String {
    let name = command.icon.trim();
    if name.is_empty() {
        return match command.provider.provider().unwrap_or(default) {
            ProviderId::Claude => glyph("sparkle"),
            ProviderId::Chatgpt => glyph("chatgpt"),
        };
    }
    if name
        .chars()
        .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
    {
        glyph(name)
    } else {
        text_icon(&name.chars().take(8).collect::<String>())
    }
}

/// "Search Drive {text}" with nothing typed: what Enter puts in the search
/// box so you can type it (the keyword, else the name, and a space).
pub fn typing_prompt(command: &AiCommand) -> String {
    let head = match command.keyword.trim() {
        "" => command.name.trim(),
        keyword => keyword,
    };
    format!("{head} ")
}

fn preview(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    if line.chars().count() <= PREVIEW_CHARS {
        return line.to_owned();
    }
    let cut: String = line.chars().take(PREVIEW_CHARS - 1).collect();
    format!("{cut}…")
}

fn row(command: &AiCommand, default: ProviderId, score: f32, text: Option<&str>) -> ResultItem {
    let ai = command.provider.provider().unwrap_or(default).name();
    let subtitle = match text {
        Some(text) => format!("Ask {ai} · “{}”", preview(text)),
        None => format!("Ask {ai} · {}", preview(&command.prompt)),
    };
    let item = ResultItem::new(command_id(&command.name, text), ItemKind::Command, &command.name)
        .with_subtitle(subtitle)
        .with_icon(icon(command, default))
        .with_score(score)
        .with_actions([Action::AskAi]);
    if text.is_none() && template::requires_text(&command.prompt) {
        item.with_accessory("‹text›")
    } else {
        item
    }
}

/// The rows `query` finds among `commands`, best first. `default` is the
/// default AI, named for commands that use it.
pub fn rows(commands: &[AiCommand], query: &str, default: ProviderId) -> Vec<ResultItem> {
    let query = query.trim();
    let mut fuzzy = FuzzyQuery::new(query);
    let mut found: Vec<ResultItem> = commands
        .iter()
        .filter(|c| !c.name.trim().is_empty())
        .filter_map(|command| {
            if template::needs(&command.prompt, true).text {
                let typed = text_after(query, &command.name)
                    .map(|text| (text, TEXT_SCORE))
                    .or_else(|| {
                        text_after(query, &command.keyword).map(|t| (t, KEYWORD_TEXT_SCORE))
                    });
                if let Some((text, score)) = typed {
                    return Some(row(command, default, score, Some(text)));
                }
            }
            if !matching::long_enough(query) {
                return None;
            }
            let keywords: Vec<&str> = [command.keyword.trim()]
                .into_iter()
                .filter(|k| !k.is_empty())
                .collect();
            let score = matching::score(&mut fuzzy, &command.name, &keywords)?;
            Some(row(command, default, score, None))
        })
        .collect();
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[ResultItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn ids_round_trip() {
        assert_eq!(command_id("Fix Grammar", None), "ai-cmd:Fix Grammar");
        assert_eq!(parse_id("ai-cmd:Fix Grammar"), Some(("Fix Grammar", None)));
        let id = command_id("Fix: Grammar", Some("a: b"));
        assert_eq!(parse_id(&id), Some(("Fix: Grammar", Some("a: b"))));
        assert_eq!(parse_id("script:x"), None);
    }

    #[test]
    fn defaults_are_complete() {
        let all = defaults();
        let names: Vec<&str> = all.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "Fix Grammar",
                "Summarize Clipboard",
                "Explain Clipboard",
                "Translate to English",
                "Make It Shorter",
                "Today's Calendar",
                "Unread Email",
                "Search Drive"
            ]
        );
        for command in &all {
            if !command.icon.is_empty() {
                assert!(matching::glyph_exists(&command.icon), "{}", command.icon);
            }
        }
        let calendar = &all[5];
        assert_eq!(calendar.provider, CommandAi::Claude);
        assert!(calendar.prompt.contains("Google Calendar connector"));
        assert!(template::requires_text(&all[7].prompt));
        assert!(!template::requires_text(&all[0].prompt));
    }

    #[test]
    fn text_after_the_name_or_keyword_fills_text() {
        let all = defaults();
        let found = rows(&all, "fix grammar teh cat sat", ProviderId::Claude);
        assert_eq!(ids(&found)[0], "ai-cmd:Fix Grammar\tteh cat sat");
        assert_eq!(found[0].score, TEXT_SCORE);
        assert_eq!(found[0].subtitle, "Ask Claude · “teh cat sat”");
        let found = rows(&all, "drive budget 2026", ProviderId::Claude);
        assert_eq!(ids(&found), ["ai-cmd:Search Drive\tbudget 2026"]);
        // A prompt without {text} doesn't take text after its name.
        let found = rows(&all, "summarize clipboard please", ProviderId::Claude);
        assert!(found.is_empty(), "{:?}", ids(&found));
    }

    #[test]
    fn names_and_keywords_match_without_text() {
        let all = defaults();
        let found = rows(&all, "calendar", ProviderId::Chatgpt);
        assert_eq!(ids(&found), ["ai-cmd:Today's Calendar"]);
        assert_eq!(found[0].icon.as_deref(), Some("glyph:calendar"));
        assert!(found[0].subtitle.starts_with("Ask Claude · What's on my"));
        let found = rows(&all, "summ", ProviderId::Chatgpt);
        assert_eq!(ids(&found)[0], "ai-cmd:Summarize Clipboard");
        assert_eq!(found[0].subtitle, "Ask ChatGPT · Summarize this in a few short bullet points:");
        assert_eq!(found[0].icon.as_deref(), Some("glyph:chatgpt"));
        let found = rows(&all, "drive", ProviderId::Claude);
        assert_eq!(found[0].id, "ai-cmd:Search Drive");
        assert_eq!(found[0].accessory.as_deref(), Some("‹text›"));
        assert_eq!(found[0].actions, [Action::AskAi]);
        assert!(rows(&all, "f", ProviderId::Claude).is_empty(), "one letter is too short");
        assert!(rows(&all, "notepad", ProviderId::Claude).is_empty());
    }

    #[test]
    fn icons_and_typing_prompts() {
        let mut c = defaults().remove(0);
        assert_eq!(icon(&c, ProviderId::Claude), "glyph:sparkle");
        c.provider = CommandAi::Chatgpt;
        assert_eq!(icon(&c, ProviderId::Claude), "glyph:chatgpt");
        c.icon = "📝".into();
        assert_eq!(icon(&c, ProviderId::Claude), "text:📝");
        assert_eq!(typing_prompt(&c), "fix ");
        c.keyword.clear();
        assert_eq!(typing_prompt(&c), "Fix Grammar ");
    }

    #[test]
    fn text_after_needs_a_space() {
        assert_eq!(text_after("fix grammar x", "Fix Grammar"), Some("x"));
        assert_eq!(text_after("fix grammarx", "Fix Grammar"), None);
        assert_eq!(text_after("fix grammar ", "Fix Grammar"), None);
        assert_eq!(text_after("anything", ""), None);
    }

    #[test]
    fn providers_map_to_ask() {
        assert_eq!(CommandAi::Default.provider(), None);
        assert_eq!(CommandAi::Chatgpt.provider(), Some(ProviderId::Chatgpt));
    }
}
