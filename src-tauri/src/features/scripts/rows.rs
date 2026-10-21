//! The rows a query finds: scripts by title or keyword ("say hello"),
//! with the text after a title as the argument ("say hello Ana"), plus
//! "Open Scripts Folder" and "Reload Scripts". Pure, so matching and ids
//! are tested here.

use super::header::{Header, ScriptMode};
use super::index::{Index, Script};
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, text_icon, Action, ItemKind, ResultItem};

/// A script: `script:<file>`, or `script:<file>:<text>` with an argument.
pub const PREFIX: &str = "script:";
pub const OPEN_FOLDER_ID: &str = "scripts:open-folder";
pub const RELOAD_ID: &str = "scripts:reload";
/// "say hello Ana": you typed the title on purpose, so above apps.
const ARGUMENT_SCORE: f32 = 1.5;
/// The same through a keyword ("hi Ana").
const KEYWORD_ARGUMENT_SCORE: f32 = 1.4;
/// Keyword matches count a little less than the title.
const KEYWORD_WEIGHT: f32 = 0.9;
/// "scripts" lists every script, below anything named like that.
const LIST_SCORE: f32 = 0.4;

/// A row's icon: `@icon` as a glyph name (`script`, `globe`) or as an
/// emoji; the script glyph without one.
pub fn icon(header: &Header) -> String {
    match header.icon.as_deref() {
        Some(name)
            if name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') =>
        {
            glyph(name)
        }
        Some(text) => text_icon(&text.chars().take(8).collect::<String>()),
        None => glyph("script"),
    }
}

/// The script id for `file`, with `argument` if given.
pub fn script_id(file: &str, argument: Option<&str>) -> String {
    match argument {
        Some(text) => format!("{PREFIX}{file}:{text}"),
        None => format!("{PREFIX}{file}"),
    }
}

/// (file, argument) from a script id.
pub fn parse_id(id: &str) -> Option<(&str, Option<&str>)> {
    let rest = id.strip_prefix(PREFIX)?;
    Some(match rest.split_once(':') {
        Some((file, text)) => (file, Some(text)),
        None => (rest, None),
    })
}

/// The text after `head` in `query` ("say hello Ana" after "Say Hello"),
/// ignoring case; `None` unless a space and some text follow.
pub fn text_after<'q>(query: &'q str, head: &str) -> Option<&'q str> {
    let mut rest = query.char_indices();
    for h in head.trim().chars() {
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

fn row(script: &Script, score: f32, argument: Option<&str>) -> ResultItem {
    let header = &script.header;
    let subtitle = match argument {
        Some(text) => format!("Run with “{text}”"),
        None if header.description.is_empty() => script.file.clone(),
        None => header.description.clone(),
    };
    let first = match header.mode {
        ScriptMode::Copy => Action::Copy,
        ScriptMode::Paste => Action::Paste,
        ScriptMode::Silent | ScriptMode::Output => Action::Run,
    };
    let item = ResultItem::new(
        script_id(&script.file, argument),
        ItemKind::Script,
        &header.title,
    )
    .with_subtitle(subtitle)
    .with_icon(icon(header))
    .with_score(score)
    .with_actions([first, Action::RevealInExplorer, Action::CopyPath]);
    match (&header.argument, argument) {
        (Some(name), None) => item.with_accessory(format!("‹{name}›")),
        _ => item,
    }
}

/// "Open Scripts Folder" and "Reload Scripts", if the query finds them.
fn commands(fuzzy: &mut FuzzyQuery, folder: &str) -> Vec<ResultItem> {
    let commands = [
        (
            OPEN_FOLDER_ID,
            "Open Scripts Folder",
            folder.to_owned(),
            "folder",
            Action::Open,
            &["script commands", "edit scripts", "add script"][..],
        ),
        (
            RELOAD_ID,
            "Reload Scripts",
            "Read the scripts folder again".to_owned(),
            "restart",
            Action::Run,
            &["refresh scripts", "rescan scripts"][..],
        ),
    ];
    commands
        .into_iter()
        .filter_map(|(id, title, subtitle, icon, action, keywords)| {
            let by_keyword = keywords
                .iter()
                .filter_map(|k| fuzzy.score(k))
                .map(|s| s * KEYWORD_WEIGHT)
                .reduce(f32::max);
            let score = fuzzy
                .score(title)
                .into_iter()
                .chain(by_keyword)
                .reduce(f32::max)?;
            Some(
                ResultItem::new(id, ItemKind::Command, title)
                    .with_subtitle(subtitle)
                    .with_icon(glyph(icon))
                    .with_score(score)
                    .with_actions([action]),
            )
        })
        .collect()
}

/// Every row `query` finds, best first.
pub fn rows(index: &Index, query: &str) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(query);
    let listing = ["script", "scripts"]
        .iter()
        .any(|w| query.trim().eq_ignore_ascii_case(w));
    let mut found: Vec<ResultItem> = index
        .scripts
        .iter()
        .filter_map(|script| {
            let header = &script.header;
            if header.argument.is_some() {
                let typed = text_after(query, &header.title)
                    .map(|text| (text, ARGUMENT_SCORE))
                    .or_else(|| {
                        header.keywords.iter().find_map(|k| {
                            text_after(query, k).map(|text| (text, KEYWORD_ARGUMENT_SCORE))
                        })
                    });
                if let Some((text, score)) = typed {
                    return Some(row(script, score, Some(text)));
                }
            }
            let by_keyword = header
                .keywords
                .iter()
                .filter_map(|k| fuzzy.score(k))
                .map(|s| s * KEYWORD_WEIGHT)
                .reduce(f32::max);
            let score = fuzzy
                .score(&header.title)
                .into_iter()
                .chain(by_keyword)
                .chain(listing.then_some(LIST_SCORE))
                .reduce(f32::max)?;
            Some(row(script, score, None))
        })
        .collect();
    found.extend(commands(&mut fuzzy, &index.folder.display().to_string()));
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    found
}

/// "Found 3 script commands", and whether some files aren't commands.
pub fn reload_message(index: &Index) -> String {
    if let Some(error) = &index.folder_error {
        return format!("Scripts folder: {error}");
    }
    let found = match index.scripts.len() {
        1 => "Found 1 script command".to_owned(),
        n => format!("Found {n} script commands"),
    };
    match index.problems.len() {
        0 => format!("{found}."),
        1 => format!("{found}; 1 file isn't one (see Settings)."),
        n => format!("{found}; {n} files aren't (see Settings)."),
    }
}

#[cfg(test)]
mod tests;
