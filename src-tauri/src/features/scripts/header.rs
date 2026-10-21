//! A script's header: the comment lines at the very top of the file that
//! turn it into a command (`# @title Say Hello`). Pure, so every comment
//! style and file encoding is tested here.

use serde::{Deserialize, Serialize};

/// Only this much of a file is read for its header.
pub const HEADER_BYTES: usize = 16 * 1024;
/// The header ends after this many lines even if comments go on.
const HEADER_LINES: usize = 80;
/// Longer titles and descriptions are cut, so a stray paragraph can't
/// flood a result row.
const MAX_TEXT: usize = 120;

/// What happens to what a script prints.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ScriptMode {
    /// Runs in the background; a toast shows the last line it printed.
    Silent,
    /// Shows what it printed: in the footer, or a text file when long.
    Output,
    /// Copies what it printed.
    Copy,
    /// Pastes what it printed into the window you came from.
    Paste,
}

/// The folder a script runs in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkDir {
    /// The script's own folder.
    Script,
    /// The folder File Explorer showed when the launcher opened.
    Explorer,
}

/// How a language writes line comments.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommentStyle {
    /// `# …` (PowerShell, Python, Bash).
    Hash,
    /// `:: …`, `REM …` or `@REM …` (batch files).
    Batch,
    /// `// …` (JavaScript).
    Slashes,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Header {
    pub title: String,
    pub description: String,
    /// An emoji, or a glyph name (see `ui/src/lib/glyphs.ts`).
    pub icon: Option<String>,
    pub mode: ScriptMode,
    /// What its one optional text argument is called ("name"), if it
    /// takes one.
    pub argument: Option<String>,
    pub keywords: Vec<String>,
    pub cwd: WorkDir,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum HeaderError {
    #[error("No @title in the comment at the top, so it isn't a command.")]
    NoTitle,
    #[error("@mode “{0}” isn't one of silent, output, copy or paste.")]
    BadMode(String),
    #[error("@cwd “{0}” isn't script or explorer.")]
    BadCwd(String),
}

/// A file's text: UTF-8 (with or without a byte order mark) or UTF-16
/// with one (what Windows PowerShell's `Out-File` writes).
pub fn decode(bytes: &[u8]) -> String {
    if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        return String::from_utf8_lossy(rest).into_owned();
    }
    let utf16 = |rest: &[u8], from: fn([u8; 2]) -> u16| {
        let units: Vec<u16> = rest
            .chunks_exact(2)
            .map(|pair| from([pair[0], pair[1]]))
            .collect();
        String::from_utf16_lossy(&units)
    };
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return utf16(rest, u16::from_le_bytes);
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return utf16(rest, u16::from_be_bytes);
    }
    String::from_utf8_lossy(bytes).into_owned()
}

/// Reads the header from the top of `text`: comment lines, with blank
/// lines, a `#!` line and (in batch files) `@echo off`-style lines
/// allowed in between. The first other line ends it.
pub fn parse(text: &str, style: CommentStyle) -> Result<Header, HeaderError> {
    let mut found = Found::default();
    for (i, line) in text.lines().take(HEADER_LINES).enumerate() {
        let line = line.trim();
        if line.is_empty() || (i == 0 && line.starts_with("#!")) {
            continue;
        }
        match comment_body(line, style) {
            Some(body) => {
                if let Some((key, value)) = directive(body) {
                    found.add(&key, value);
                }
            }
            // `@echo off`, `@setlocal`, `@chcp 65001 >nul`: batch files
            // start with these, before or after their comments.
            None if style == CommentStyle::Batch && line.starts_with('@') => {}
            None => break,
        }
    }
    found.finish()
}

/// The text of a comment line, without its marker; `None` if the line
/// isn't a comment.
fn comment_body(line: &str, style: CommentStyle) -> Option<&str> {
    match style {
        CommentStyle::Hash => line.strip_prefix('#'),
        CommentStyle::Slashes => line.strip_prefix("//"),
        CommentStyle::Batch => {
            if let Some(rest) = line.strip_prefix("::") {
                return Some(rest);
            }
            let rest = line.strip_prefix('@').unwrap_or(line);
            let head = rest.get(..3)?;
            let after = &rest[3..];
            // `REM` must be a word of its own: `REMOVE` isn't a comment.
            (head.eq_ignore_ascii_case("rem")
                && (after.is_empty() || after.starts_with(char::is_whitespace)))
            .then_some(after)
        }
    }
}

/// `@key value` or `@key: value` → (lowercase key, trimmed value).
fn directive(body: &str) -> Option<(String, &str)> {
    let rest = body.trim_start().strip_prefix('@')?;
    let end = rest
        .find(|c: char| c.is_whitespace() || c == ':')
        .unwrap_or(rest.len());
    let key = &rest[..end];
    if key.is_empty() {
        return None;
    }
    let value = rest[end..].trim_start();
    let value = value.strip_prefix(':').unwrap_or(value).trim();
    Some((key.to_ascii_lowercase(), value))
}

/// Directives seen so far. The first of each counts, except keywords,
/// which add up.
#[derive(Default)]
struct Found {
    title: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    mode: Option<String>,
    argument: Option<String>,
    keywords: Vec<String>,
    cwd: Option<String>,
}

impl Found {
    fn add(&mut self, key: &str, value: &str) {
        let slot = match key {
            "title" => &mut self.title,
            "description" => &mut self.description,
            "icon" => &mut self.icon,
            "mode" => &mut self.mode,
            "argument" => &mut self.argument,
            "cwd" => &mut self.cwd,
            "keywords" => {
                self.keywords.extend(
                    value
                        .split(',')
                        .map(str::trim)
                        .filter(|k| !k.is_empty())
                        .map(|k| cut(k, MAX_TEXT)),
                );
                return;
            }
            // Anything else (@author, @version) is the script's own business.
            _ => return,
        };
        if slot.is_none() {
            *slot = Some(value.to_owned());
        }
    }

    fn finish(self) -> Result<Header, HeaderError> {
        let title = self
            .title
            .filter(|t| !t.is_empty())
            .ok_or(HeaderError::NoTitle)?;
        let mode = match self.mode.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("silent") => ScriptMode::Silent,
            Some("output") => ScriptMode::Output,
            Some("copy") => ScriptMode::Copy,
            Some("paste") => ScriptMode::Paste,
            Some(_) => return Err(HeaderError::BadMode(self.mode.unwrap_or_default())),
        };
        let cwd = match self.cwd.as_deref().map(str::to_ascii_lowercase).as_deref() {
            None | Some("") | Some("script") => WorkDir::Script,
            Some("explorer") => WorkDir::Explorer,
            Some(_) => return Err(HeaderError::BadCwd(self.cwd.unwrap_or_default())),
        };
        Ok(Header {
            title: cut(&title, MAX_TEXT),
            description: cut(self.description.as_deref().unwrap_or(""), MAX_TEXT),
            icon: self.icon.filter(|i| !i.is_empty()),
            mode,
            // `@argument` alone still takes text; it just has no name.
            argument: self.argument.map(|name| {
                if name.is_empty() {
                    "text".to_owned()
                } else {
                    cut(&name, 40)
                }
            }),
            keywords: self.keywords,
            cwd,
        })
    }
}

fn cut(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let kept: String = text.chars().take(max_chars - 1).collect();
    format!("{kept}…")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hash(text: &str) -> Result<Header, HeaderError> {
        parse(text, CommentStyle::Hash)
    }

    #[test]
    fn reads_every_directive() {
        let h = hash(
            "# @title Say Hello\n# @description Greets you\n# @icon 👋\n# @mode copy\n\
             # @argument name\n# @keywords greet, hi \n# @keywords wave\n# @cwd explorer\n\
             # @author me\nWrite-Output hi\n",
        )
        .unwrap();
        assert_eq!(
            h,
            Header {
                title: "Say Hello".into(),
                description: "Greets you".into(),
                icon: Some("👋".into()),
                mode: ScriptMode::Copy,
                argument: Some("name".into()),
                keywords: vec!["greet".into(), "hi".into(), "wave".into()],
                cwd: WorkDir::Explorer,
            }
        );
    }

    #[test]
    fn defaults_for_what_isnt_there() {
        let h = hash("# @title Lock\n").unwrap();
        assert_eq!(h.mode, ScriptMode::Silent);
        assert_eq!(h.cwd, WorkDir::Script);
        assert_eq!(
            (h.description.as_str(), h.icon, h.argument),
            ("", None, None)
        );
        assert!(h.keywords.is_empty());
        // `@argument` without a name still takes text.
        assert_eq!(
            hash("# @title X\n# @argument\n")
                .unwrap()
                .argument
                .as_deref(),
            Some("text")
        );
    }

    #[test]
    fn a_missing_title_is_not_a_command() {
        assert_eq!(hash("# @description no title\n"), Err(HeaderError::NoTitle));
        assert_eq!(hash("# @title   \n"), Err(HeaderError::NoTitle));
        assert_eq!(hash("print('no header')\n"), Err(HeaderError::NoTitle));
        assert_eq!(hash(""), Err(HeaderError::NoTitle));
    }

    #[test]
    fn the_header_ends_at_the_first_line_of_code() {
        let text = "# @title Real\nparam($x)\n# @title Later\n# @mode copy\n";
        let h = hash(text).unwrap();
        assert_eq!(h.title, "Real");
        assert_eq!(h.mode, ScriptMode::Silent);
        assert_eq!(
            hash("$x = 1\n# @title Too late\n"),
            Err(HeaderError::NoTitle)
        );
    }

    #[test]
    fn blank_lines_shebangs_and_plain_comments_are_skipped() {
        let text =
            "#!/usr/bin/env python3\n\n# -*- coding: utf-8 -*-\n#\n#   @title   Spaced Out  \n";
        assert_eq!(hash(text).unwrap().title, "Spaced Out");
        // A colon after the key works too, and keys ignore case.
        assert_eq!(hash("#@Title: Tight\n").unwrap().title, "Tight");
    }

    #[test]
    fn bad_values_say_what_is_wrong() {
        assert_eq!(
            hash("# @title X\n# @mode loud\n"),
            Err(HeaderError::BadMode("loud".into()))
        );
        assert_eq!(
            hash("# @title X\n# @cwd home\n"),
            Err(HeaderError::BadCwd("home".into()))
        );
        assert_eq!(
            HeaderError::BadMode("loud".into()).to_string(),
            "@mode “loud” isn't one of silent, output, copy or paste."
        );
        // Modes ignore case.
        assert_eq!(
            hash("# @title X\n# @mode PASTE\n").unwrap().mode,
            ScriptMode::Paste
        );
    }

    #[test]
    fn batch_comments() {
        let text = "@echo off\r\n:: @title Flush DNS\r\nREM @description Clears the cache\r\n\
                    @REM @mode output\r\nrem @keywords dns\r\n@chcp 65001 >nul\r\n:: @icon process\r\n\
                    ipconfig /flushdns\r\n:: @title Too late\r\n";
        let h = parse(text, CommentStyle::Batch).unwrap();
        assert_eq!(h.title, "Flush DNS");
        assert_eq!(h.description, "Clears the cache");
        assert_eq!(h.mode, ScriptMode::Output);
        assert_eq!(h.keywords, ["dns"]);
        assert_eq!(h.icon.as_deref(), Some("process"));
        // REMOVE isn't REM, and `#` isn't a batch comment.
        assert_eq!(
            parse("REMOVE @title X\n", CommentStyle::Batch),
            Err(HeaderError::NoTitle)
        );
        assert_eq!(
            parse("# @title X\n", CommentStyle::Batch),
            Err(HeaderError::NoTitle)
        );
    }

    #[test]
    fn javascript_comments() {
        let text = "#!/usr/bin/env node\n// @title Uuid\n// @mode paste\n'use strict';\n";
        let h = parse(text, CommentStyle::Slashes).unwrap();
        assert_eq!((h.title.as_str(), h.mode), ("Uuid", ScriptMode::Paste));
        assert_eq!(
            parse("# @title X\n", CommentStyle::Slashes),
            Err(HeaderError::NoTitle)
        );
    }

    #[test]
    fn crlf_files() {
        let h = hash("# @title Windows\r\n# @description Saved in Notepad\r\n").unwrap();
        assert_eq!(h.title, "Windows");
        assert_eq!(h.description, "Saved in Notepad");
    }

    #[test]
    fn byte_order_marks_and_utf16() {
        let utf8 = [&[0xEF, 0xBB, 0xBF][..], "# @title Café\n".as_bytes()].concat();
        assert_eq!(hash(&decode(&utf8)).unwrap().title, "Café");

        let text = "# @title Wide 👋\r\n";
        let le: Vec<u8> = [0xFF, 0xFE]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_le_bytes))
            .collect();
        assert_eq!(hash(&decode(&le)).unwrap().title, "Wide 👋");
        let be: Vec<u8> = [0xFE, 0xFF]
            .into_iter()
            .chain(text.encode_utf16().flat_map(u16::to_be_bytes))
            .collect();
        assert_eq!(hash(&decode(&be)).unwrap().title, "Wide 👋");
        // No mark: UTF-8, with anything else replaced rather than failing.
        assert_eq!(decode(b"# @title A\xFFB"), "# @title A\u{FFFD}B");
    }

    #[test]
    fn long_text_is_cut() {
        let long = "x".repeat(500);
        let h = hash(&format!("# @title {long}\n# @description {long}\n")).unwrap();
        assert_eq!(h.title.chars().count(), MAX_TEXT);
        assert!(h.title.ends_with('…'));
        assert_eq!(h.description.chars().count(), MAX_TEXT);
    }

    #[test]
    fn the_first_of_a_directive_counts() {
        let h = hash("# @title One\n# @title Two\n# @mode copy\n# @mode paste\n").unwrap();
        assert_eq!((h.title.as_str(), h.mode), ("One", ScriptMode::Copy));
    }
}
