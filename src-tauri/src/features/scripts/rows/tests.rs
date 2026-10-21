use std::path::Path;

use super::*;
use crate::features::scripts::header::{Header, ScriptMode, WorkDir};
use crate::features::scripts::index::Problem;
use crate::features::scripts::language::{Language, Runner};

fn header(title: &str) -> Header {
    Header {
        title: title.into(),
        description: String::new(),
        icon: None,
        mode: ScriptMode::Silent,
        argument: None,
        keywords: Vec::new(),
        cwd: WorkDir::Script,
    }
}

fn script(file: &str, header: Header) -> Script {
    Script {
        file: file.into(),
        path: Path::new("/s").join(file),
        language: Language::PowerShell,
        header,
        runner: Runner::Itself,
    }
}

fn index(scripts: Vec<Script>) -> Index {
    Index {
        folder: "/s".into(),
        folder_error: None,
        scripts,
        problems: Vec::new(),
    }
}

fn ids(rows: &[ResultItem]) -> Vec<&str> {
    rows.iter().map(|r| r.id.as_str()).collect()
}

#[test]
fn icons() {
    let mut h = header("X");
    assert_eq!(icon(&h), "glyph:script");
    h.icon = Some("globe".into());
    assert_eq!(icon(&h), "glyph:globe");
    h.icon = Some("layout-left".into());
    assert_eq!(icon(&h), "glyph:layout-left");
    h.icon = Some("👋".into());
    assert_eq!(icon(&h), "text:👋");
    h.icon = Some("Globe".into());
    assert_eq!(icon(&h), "text:Globe");
}

#[test]
fn ids_round_trip() {
    assert_eq!(script_id("a.ps1", None), "script:a.ps1");
    assert_eq!(parse_id("script:a.ps1"), Some(("a.ps1", None)));
    let id = script_id("a.ps1", Some("http://x:80"));
    assert_eq!(parse_id(&id), Some(("a.ps1", Some("http://x:80"))));
    assert_eq!(parse_id("app:a.ps1"), None);
}

#[test]
fn text_after_a_title() {
    assert_eq!(text_after("say hello Ana", "Say Hello"), Some("Ana"));
    assert_eq!(
        text_after("SAY HELLO   Ana Lee ", "Say Hello"),
        Some("Ana Lee")
    );
    assert_eq!(text_after("Café crème", "café"), Some("crème"));
    assert_eq!(text_after("say hello", "Say Hello"), None);
    assert_eq!(text_after("say hello ", "Say Hello"), None);
    assert_eq!(text_after("say helloAna", "Say Hello"), None);
    assert_eq!(text_after("say", "Say Hello"), None);
    assert_eq!(text_after("x y", ""), None);
}

#[test]
fn found_by_title_and_keywords() {
    let mut flush = header("Flush DNS");
    flush.keywords = vec!["ipconfig".into()];
    flush.description = "Clears the DNS cache".into();
    let idx = index(vec![
        script("flush.cmd", flush),
        script("lock.ps1", header("Lock Screen")),
    ]);
    let found = rows(&idx, "flush");
    assert_eq!(ids(&found), ["script:flush.cmd"]);
    assert_eq!(found[0].subtitle, "Clears the DNS cache");
    assert_eq!(found[0].kind, ItemKind::Script);
    assert_eq!(
        found[0].actions,
        [Action::Run, Action::RevealInExplorer, Action::CopyPath]
    );
    assert_eq!(ids(&rows(&idx, "ipconfig")), ["script:flush.cmd"]);
    let lock = rows(&idx, "lock");
    assert_eq!(
        lock[0].subtitle, "lock.ps1",
        "the file when there's no description"
    );
    assert!(rows(&idx, "zzz").is_empty());
}

#[test]
fn text_after_the_title_is_the_argument() {
    let mut hello = header("Say Hello");
    hello.argument = Some("name".into());
    hello.keywords = vec!["hi".into()];
    let idx = index(vec![script("hello.ps1", hello)]);

    let bare = rows(&idx, "say hello");
    assert_eq!(ids(&bare), ["script:hello.ps1"]);
    assert_eq!(bare[0].accessory.as_deref(), Some("‹name›"));

    let with_text = rows(&idx, "say hello Ana Lee");
    assert_eq!(ids(&with_text), ["script:hello.ps1:Ana Lee"]);
    assert_eq!(with_text[0].subtitle, "Run with “Ana Lee”");
    assert_eq!(with_text[0].title, "Say Hello");
    assert_eq!(with_text[0].score, ARGUMENT_SCORE);
    assert_eq!(with_text[0].accessory, None);

    let by_keyword = rows(&idx, "hi Bo");
    assert_eq!(ids(&by_keyword), ["script:hello.ps1:Bo"]);
    assert_eq!(by_keyword[0].score, KEYWORD_ARGUMENT_SCORE);

    // A script without @argument ignores text after its title.
    let idx = index(vec![script("lock.ps1", header("Lock"))]);
    assert!(rows(&idx, "lock now").is_empty());
}

#[test]
fn the_mode_picks_what_enter_says() {
    let mut copy = header("Uuid");
    copy.mode = ScriptMode::Copy;
    let mut paste = header("Date");
    paste.mode = ScriptMode::Paste;
    let idx = index(vec![script("u.py", copy), script("d.py", paste)]);
    assert_eq!(rows(&idx, "uuid")[0].actions[0], Action::Copy);
    assert_eq!(rows(&idx, "date")[0].actions[0], Action::Paste);
}

#[test]
fn scripts_lists_them_all_with_the_commands() {
    let idx = index(vec![
        script("a.ps1", header("Alpha")),
        script("b.ps1", header("Beta")),
    ]);
    let found = rows(&idx, "scripts");
    let mut found_ids = ids(&found);
    found_ids.sort_unstable();
    assert_eq!(
        found_ids,
        [
            "script:a.ps1",
            "script:b.ps1",
            "scripts:open-folder",
            "scripts:reload"
        ]
    );
    // The commands come first: they're named like the query.
    assert!(found[0].id.starts_with("scripts:"));
    let open = rows(&idx, "open scripts folder");
    assert_eq!(open[0].id, OPEN_FOLDER_ID);
    assert_eq!(open[0].subtitle, Path::new("/s").display().to_string());
    assert_eq!(rows(&idx, "reload scripts")[0].id, RELOAD_ID);
}

#[test]
fn reload_messages() {
    let mut idx = index(vec![script("a.ps1", header("A"))]);
    assert_eq!(reload_message(&idx), "Found 1 script command.");
    idx.problems = vec![Problem {
        file: "b.py".into(),
        message: "x".into(),
    }];
    assert_eq!(
        reload_message(&idx),
        "Found 1 script command; 1 file isn't one (see Settings)."
    );
    idx.folder_error = Some("The folder doesn't exist.".into());
    assert_eq!(
        reload_message(&idx),
        "Scripts folder: The folder doesn't exist."
    );
}
