use super::*;
use crate::ai::ProviderId;

fn files(dir: &Path) -> Files {
    Files {
        synced: dir.join("cfg").join("settings.toml"),
        local: dir.join("data").join("settings.local.toml"),
    }
}

#[test]
fn first_start_creates_both_files() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let loaded = load(&f);
    assert_eq!(loaded.settings, Settings::default());
    assert!(loaded.unreadable.is_none());
    assert_eq!(f.read().unwrap(), loaded.texts);
    let local = loaded.texts.local.unwrap();
    assert!(local.contains("index_folders"), "{local}");
    assert!(!loaded.texts.synced.unwrap().contains("index_folders"));
}

#[test]
fn an_old_single_file_is_split_at_the_first_start() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let mut old = Settings::default();
    old.search.index_folders = vec![r"D:\Work".into()];
    old.general.launch_at_login = true;
    old.ai.default_provider = ProviderId::Chatgpt;
    old.appearance.tint = "purple".into();
    super::super::save(&f.synced, &old).unwrap();

    let loaded = load(&f);
    assert_eq!(loaded.settings, old, "nothing lost");
    assert_eq!(f.read().unwrap(), loaded.texts);
    let synced = loaded.texts.synced.as_deref().unwrap();
    let local = loaded.texts.local.as_deref().unwrap();
    assert!(!synced.contains(r"D:\Work"), "{synced}");
    assert!(
        synced.contains("default_provider = \"chatgpt\""),
        "{synced}"
    );
    assert!(local.contains(r"D:\Work"), "{local}");
    assert!(local.contains("launch_at_login = true"), "{local}");
    assert_eq!(load(&f).settings, old, "the same after the split");
}

#[test]
fn a_save_keeps_a_copy_of_a_file_that_does_not_parse() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    load(&f);
    fs::write(&f.synced, "[hotkeys\ntoggle = 'Alt+Space'\n").unwrap();
    write(&f, &Settings::default(), Scope::Both).unwrap();
    let bad = f.synced.with_file_name("settings.toml.bad");
    assert_eq!(
        fs::read_to_string(bad).unwrap(),
        "[hotkeys\ntoggle = 'Alt+Space'\n"
    );
    assert!(load(&f).settings == Settings::default());
}

#[test]
fn copying_to_another_folder_keeps_unknown_keys() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let source = Texts {
        synced: Some("future_key = 7\n[search]\nmax_results = 5\n".into()),
        local: None,
    };
    let (settings, _) = settings_from(&source).unwrap();
    let target = Files {
        synced: dir.path().join("OneDrive").join("settings.toml"),
        local: f.local.clone(),
    };
    let texts = copy_to(&target, &settings, &source).unwrap();
    assert_eq!(target.read().unwrap(), texts);
    let synced = texts.synced.unwrap();
    assert!(synced.contains("future_key = 7"), "{synced}");
    assert!(synced.contains("max_results = 5"), "{synced}");
}

#[test]
fn unchanged_files_are_not_rewritten() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let loaded = load(&f);
    // An old time on the synced file shows whether it was rewritten.
    let long_ago = std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1 << 30);
    let file = fs::File::options().write(true).open(&f.synced).unwrap();
    file.set_modified(long_ago).unwrap();
    drop(file);
    let mut s = loaded.settings.clone();
    s.general.launch_at_login = true;
    let texts = write(&f, &s, Scope::Both).unwrap();
    assert_eq!(
        fs::metadata(&f.synced).unwrap().modified().unwrap(),
        long_ago
    );
    assert!(texts.local.unwrap().contains("launch_at_login = true"));
    assert_eq!(load(&f).settings, s);
}

#[test]
fn local_only_leaves_the_synced_file_alone() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let mut s = Settings::default();
    s.hotkeys.toggle = "Alt+Space".into();
    s.search.max_results = 5;
    let texts = write(&f, &s, Scope::LocalOnly).unwrap();
    assert!(texts.synced.is_none());
    assert!(!f.synced.exists());
    assert!(texts.local.is_some());
}

#[test]
fn our_own_write_is_recognized() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let known = load(&f).texts;
    assert_eq!(compare(&known, &f.read().unwrap()), Seen::Unchanged);

    let mut s = Settings::default();
    s.appearance.tint = "teal".into();
    let known = write(&f, &s, Scope::Both).unwrap();
    assert_eq!(compare(&known, &f.read().unwrap()), Seen::Unchanged);

    // Another PC (or a hand edit) changes the synced file.
    let edited = known.synced.as_deref().unwrap().replace("teal", "pink");
    fs::write(&f.synced, edited).unwrap();
    let now = f.read().unwrap();
    assert_eq!(compare(&known, &now), Seen::Changed);
    assert_eq!(settings_from(&now).unwrap().0.appearance.tint, "pink");

    // A sync replacing the file: it's briefly not there.
    fs::remove_file(&f.synced).unwrap();
    assert_eq!(compare(&known, &f.read().unwrap()), Seen::Gone);
    // Only this PC's file changed (a hand edit): that counts.
    let local_edit = Texts {
        synced: known.synced.clone(),
        local: Some("[search]\nmax_results = 3\n".into()),
    };
    assert_eq!(compare(&known, &local_edit), Seen::Changed);
}

#[test]
fn a_broken_file_is_set_aside_and_the_other_kept() {
    let dir = tempfile::tempdir().unwrap();
    let f = files(dir.path());
    let mut s = Settings::default();
    s.search.index_folders = vec!["~/src".into()];
    write(&f, &s, Scope::Both).unwrap();
    fs::write(&f.synced, "this is = = not toml").unwrap();
    let loaded = load(&f);
    assert_eq!(
        loaded.settings.search.index_folders,
        ["~/src"],
        "local file still used"
    );
    assert!(f.synced.with_file_name("settings.toml.bad").exists());
    assert!(loaded.texts.synced.is_some(), "recreated with defaults");

    fs::write(&f.local, "[search\n").unwrap();
    let loaded = load(&f);
    assert_eq!(loaded.settings, Settings::default());
    assert!(f.local.with_file_name("settings.local.toml.bad").exists());
}

#[test]
fn a_reload_error_names_the_file() {
    let bad_local = Texts {
        synced: Some("version = 1\n".into()),
        local: Some("[hotkeys]\nwin_key_override = \"yes\"\n".into()),
    };
    assert!(matches!(settings_from(&bad_local), Err((Which::Local, _))));
    let bad_synced = Texts {
        synced: Some("[[".into()),
        local: None,
    };
    assert!(matches!(
        settings_from(&bad_synced),
        Err((Which::Synced, _))
    ));
}
