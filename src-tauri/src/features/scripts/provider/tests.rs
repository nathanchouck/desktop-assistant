use std::path::Path;
use std::sync::RwLock;

use super::*;
use crate::features::scripts::header::{Header, ScriptMode, WorkDir};
use crate::features::scripts::index::Problem;
use crate::features::scripts::language::{Language, Runner};
use crate::paths::AppPaths;

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

/// A provider over a temporary scripts folder.
fn provider(dir: &Path) -> ScriptProvider {
    let mut settings = crate::settings::Settings::default();
    settings.scripts.folder = dir.join("scripts").display().to_string();
    settings.scripts.timeout_secs = 10;
    let paths = AppPaths::with_roots(&dir.join("config"), &dir.join("data"));
    let library = Arc::new(Library::new(paths, Arc::new(RwLock::new(settings))));
    ScriptProvider::new(
        library,
        Arc::new(Shell::new()),
        Arc::new(DesktopTracker::new()),
        Arc::new(Store::in_memory()),
    )
}

#[tokio::test]
async fn unknown_or_removed_scripts_are_errors() {
    let dir = tempfile::tempdir().unwrap();
    let p = provider(dir.path());
    assert!(p.owns("script:x.ps1") && p.owns(RELOAD_ID) && !p.owns("app:x"));
    assert!(p.activate("script:gone.ps1", Action::Run).await.is_err());
    // The folder doesn't exist (it isn't the default one, so isn't made).
    assert_eq!(
        p.activate(RELOAD_ID, Action::Run).await.unwrap(),
        notice("Scripts folder: The folder doesn't exist.")
    );
}

#[cfg(unix)]
mod running {
    use super::*;

    fn add(dir: &Path, file: &str, text: &str) {
        let folder = dir.join("scripts");
        std::fs::create_dir_all(&folder).unwrap();
        std::fs::write(folder.join(file), text).unwrap();
    }

    async fn ready(dir: &Path, scripts: &[(&str, &str)]) -> ScriptProvider {
        for (file, text) in scripts {
            add(dir, file, text);
        }
        let p = provider(dir);
        p.activate(RELOAD_ID, Action::Run).await.unwrap();
        p
    }

    #[tokio::test]
    async fn copy_paste_and_output_modes() {
        let dir = tempfile::tempdir().unwrap();
        let p = ready(
            dir.path(),
            &[
                (
                    "copy.sh",
                    "# @title Copy It\n# @mode copy\n# @argument\necho \"got $1\"\n",
                ),
                (
                    "paste.sh",
                    "# @title Paste It\n# @mode paste\nprintf 'a\\nb\\n'\n",
                ),
                (
                    "out.sh",
                    "# @title Show It\n# @mode output\necho one; echo two\n",
                ),
                ("quiet.sh", "# @title Quiet\n# @mode copy\ntrue\n"),
            ],
        )
        .await;
        assert_eq!(p.library.index().scripts.len(), 4);
        let found = p.query("copy it Ana", 8).await.unwrap();
        assert_eq!(found[0].id, "script:copy.sh:Ana");
        assert_eq!(
            p.activate(&found[0].id, Action::Copy).await.unwrap(),
            Activation::Copy {
                text: "got Ana".into()
            }
        );
        assert_eq!(
            p.activate("script:paste.sh", Action::Paste).await.unwrap(),
            Activation::Paste {
                text: Some("a\nb".into())
            }
        );
        assert_eq!(
            p.activate("script:out.sh", Action::Run).await.unwrap(),
            notice("one · two")
        );
        // A notice keeps the launcher open, so the run is counted here.
        assert!(p.store.frecency("script:out.sh") > 0.0);
        assert_eq!(
            p.activate("script:quiet.sh", Action::Copy).await.unwrap(),
            notice("Quiet didn't print anything.")
        );
    }

    #[tokio::test]
    async fn failures_and_paths() {
        let dir = tempfile::tempdir().unwrap();
        let p = ready(
            dir.path(),
            &[
                (
                    "fail.sh",
                    "# @title Fail\n# @mode output\necho 'disk full' >&2; exit 1\n",
                ),
                ("here.sh", "# @title Here\n# @mode copy\npwd\n"),
                (
                    "there.sh",
                    "# @title There\n# @mode copy\n# @cwd explorer\npwd\n",
                ),
            ],
        )
        .await;
        assert_eq!(
            p.activate("script:fail.sh", Action::Run).await.unwrap(),
            notice("Fail failed: disk full")
        );
        // Runs in its own folder.
        let Activation::Copy { text } = p.activate("script:here.sh", Action::Copy).await.unwrap()
        else {
            panic!("expected a copy");
        };
        let folder = dir.path().join("scripts").canonicalize().unwrap();
        assert_eq!(Path::new(&text).canonicalize().unwrap(), folder);
        // No File Explorer window was captured.
        assert_eq!(
            p.activate("script:there.sh", Action::Copy).await.unwrap(),
            notice("There runs in File Explorer's folder: open the launcher from File Explorer.")
        );
        assert_eq!(
            p.activate("script:here.sh", Action::CopyPath)
                .await
                .unwrap(),
            Activation::Copy {
                text: dir
                    .path()
                    .join("scripts")
                    .join("here.sh")
                    .display()
                    .to_string()
            }
        );
    }

    #[tokio::test]
    async fn silent_scripts_run_in_the_background() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("ran");
        let script = format!("# @title Touch\nsleep 0.3; touch '{}'\n", marker.display());
        let p = ready(dir.path(), &[("touch.sh", &script)]).await;
        assert_eq!(
            p.activate("script:touch.sh", Action::Run).await.unwrap(),
            Activation::Hide
        );
        // Still running: Enter again doesn't start it twice.
        assert_eq!(
            p.activate("script:touch.sh", Action::Run).await.unwrap(),
            notice("Touch is still running.")
        );
        for _ in 0..50 {
            if marker.exists() && p.library.claim("touch.sh").is_some() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("the silent script didn't finish");
    }
}
