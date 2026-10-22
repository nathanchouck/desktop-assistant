use std::path::Path;
use std::sync::RwLock;

use super::*;
use crate::paths::AppPaths;

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
