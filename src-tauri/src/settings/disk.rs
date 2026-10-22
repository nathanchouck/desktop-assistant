//! Reading and writing the two settings files: `settings.toml` in the
//! settings folder (maybe a synced one) and `settings.local.toml` in the
//! local data folder (this PC only).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml::Table;

use super::{default_table, layers, migrate, Settings, SettingsError};
use crate::paths::AppPaths;

const SYNCED_HEADER: &str = "\
# Desktop Assistant settings. Missing keys use their defaults.
# This PC's own settings (index folders, program paths, start at login...)
# are in settings.local.toml in %LOCALAPPDATA%\\desktop-assistant.

";

const LOCAL_HEADER: &str = "\
# Settings for this PC only. They win over settings.toml, which may be
# shared with your other PCs. A key you add here stays per-PC.

";

/// Where the two files are.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Files {
    pub synced: PathBuf,
    pub local: PathBuf,
}

impl Files {
    /// The files in use now (the settings folder can change while running).
    pub fn of(paths: &AppPaths) -> Self {
        Self {
            synced: paths.settings_file(),
            local: paths.local_settings_file(),
        }
    }

    /// Both files' text. A missing file is `None`; any other error (a
    /// cloud file that can't be fetched, no permission) is an error.
    pub fn read(&self) -> io::Result<Texts> {
        Ok(Texts {
            synced: read_text(&self.synced)?,
            local: read_text(&self.local)?,
        })
    }
}

/// The two files' text (`None`: no file), as read or written by us.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Texts {
    pub synced: Option<String>,
    pub local: Option<String>,
}

/// What a change seen on disk means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Seen {
    /// The files hold what we last read or wrote: our own write, or a
    /// touch that changed nothing.
    Unchanged,
    /// The settings file is gone, probably for a moment (a sync replacing
    /// it). Keep the current settings; the next save writes it again.
    Gone,
    /// Changed elsewhere (another PC, a hand edit): load it.
    Changed,
}

pub fn compare(known: &Texts, now: &Texts) -> Seen {
    if now == known {
        Seen::Unchanged
    } else if now.synced.is_none() && known.synced.is_some() {
        Seen::Gone
    } else {
        Seen::Changed
    }
}

/// Which file a problem is in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Which {
    Synced,
    Local,
}

/// Settings from the two files' text: defaults, then the synced file, then
/// the local one, each migrated first. Also says whether a migration
/// changed either file, so it can be written back.
pub fn settings_from(texts: &Texts) -> Result<(Settings, bool), (Which, SettingsError)> {
    let (synced, synced_migrated) =
        table_from(texts.synced.as_deref()).map_err(|e| (Which::Synced, e))?;
    let (local, local_migrated) =
        table_from(texts.local.as_deref()).map_err(|e| (Which::Local, e))?;
    let defaults = default_table().map_err(|e| (Which::Synced, e))?;
    // A value of the wrong type is the synced file's fault if that file
    // fails on its own, else the local file's.
    let with_synced = layers::layer(defaults, synced, Table::new());
    with_synced
        .clone()
        .try_into::<Settings>()
        .map_err(|e| (Which::Synced, SettingsError::Parse(e)))?;
    let settings = layers::layer(with_synced, Table::new(), local)
        .try_into()
        .map_err(|e| (Which::Local, SettingsError::Parse(e)))?;
    Ok((settings, synced_migrated || local_migrated))
}

/// A file's table after migration (empty for no file), and whether the
/// migration changed it.
fn table_from(text: Option<&str>) -> Result<(Table, bool), SettingsError> {
    let Some(text) = text else {
        return Ok((Table::new(), false));
    };
    let mut table: Table = toml::from_str(text)?;
    let migrated = migrate(&mut table);
    Ok((table, migrated))
}

/// A file's table for merging into on save: whatever parses, else nothing.
fn lenient_table(text: Option<&str>) -> Table {
    table_from(text).map(|(t, _)| t).unwrap_or_default()
}

/// The two files' text for `settings`, given what they hold now (`on_disk`):
/// this PC's keys in the local file, keys neither file should lose kept.
pub fn texts_for(settings: &Settings, on_disk: &Texts) -> Result<Texts, SettingsError> {
    let full = Table::try_from(settings).map_err(|e| SettingsError::Shape(e.to_string()))?;
    let (synced, local) = layers::split_for_save(
        full,
        lenient_table(on_disk.synced.as_deref()),
        lenient_table(on_disk.local.as_deref()),
    );
    Ok(Texts {
        synced: Some(format!("{SYNCED_HEADER}{}", toml::to_string_pretty(&synced)?)),
        local: Some(format!("{LOCAL_HEADER}{}", toml::to_string_pretty(&local)?)),
    })
}

/// Which files to write.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    Both,
    /// Only this PC's file (the synced one belongs to another folder now,
    /// or can't be read).
    LocalOnly,
}

/// Saves `settings` and returns what the files hold afterwards. A file
/// whose text wouldn't change isn't touched, so changing a per-PC setting
/// leaves the synced file (and the other PCs) alone.
pub fn write(files: &Files, settings: &Settings, scope: Scope) -> Result<Texts, SettingsError> {
    // What's there now, for keys to keep and to skip unchanged files. A
    // file that can't be read is simply written.
    let on_disk = Texts {
        synced: read_text(&files.synced).ok().flatten(),
        local: read_text(&files.local).ok().flatten(),
    };
    let wanted = texts_for(settings, &on_disk)?;
    let mut now = on_disk;
    if scope == Scope::Both && wanted.synced != now.synced {
        if let Some(text) = &wanted.synced {
            write_atomic(&files.synced, text)?;
            now.synced = wanted.synced;
        }
    }
    if wanted.local != now.local {
        if let Some(text) = &wanted.local {
            write_atomic(&files.local, text)?;
            now.local = wanted.local;
        }
    }
    Ok(now)
}

/// Reads a file; `None` if there's no file.
pub fn read_text(path: &Path) -> io::Result<Option<String>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(Some(text)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e),
    }
}

/// Writes a file whole (temp file, then rename), so a reader (or a sync
/// client) never sees half of it.
pub fn write_atomic(path: &Path, text: &str) -> Result<(), SettingsError> {
    let io_err = |source| SettingsError::Io {
        path: path.to_owned(),
        source,
    };
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir).map_err(io_err)?;
    }
    let tmp = path.with_extension("toml.tmp");
    fs::write(&tmp, text).map_err(io_err)?;
    fs::rename(&tmp, path).map_err(io_err)?;
    Ok(())
}

/// Moves a file that doesn't parse to `<name>.bad` (replacing an older
/// one), so it can be fixed by hand and isn't read again.
pub fn set_aside(path: &Path) -> io::Result<PathBuf> {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".bad");
    let bad = path.with_file_name(name);
    fs::rename(path, &bad)?;
    Ok(bad)
}

/// What startup found.
#[derive(Debug)]
pub struct Loaded {
    pub settings: Settings,
    /// What the files hold now.
    pub texts: Texts,
    /// The settings file couldn't be read (not missing: unreadable, e.g. a
    /// cloud file while OneDrive isn't running). Defaults (and this PC's
    /// file) are used, and nothing is saved until it can be read.
    pub unreadable: Option<String>,
}

/// Loads settings at startup. A missing file is created with defaults; a
/// file that doesn't parse is moved aside (`settings.toml.bad`) and
/// defaults are used, so a typo never stops the launcher from starting.
/// Never fails: the worst case is defaults.
pub fn load(files: &Files) -> Loaded {
    let mut unreadable = None;
    let mut texts = Texts {
        synced: read_text(&files.synced).unwrap_or_else(|e| {
            tracing::error!("can't read {}: {e}", files.synced.display());
            unreadable = Some(e.to_string());
            None
        }),
        local: read_text(&files.local).unwrap_or_else(|e| {
            tracing::error!("can't read {}: {e}", files.local.display());
            None
        }),
    };
    let (settings, migrated) = loop {
        match settings_from(&texts) {
            Ok(found) => break found,
            Err((which, e)) => {
                let (path, text) = match which {
                    Which::Synced => (&files.synced, &mut texts.synced),
                    Which::Local => (&files.local, &mut texts.local),
                };
                match set_aside(path) {
                    Ok(bad) => tracing::error!("{e}; moved it to {}", bad.display()),
                    Err(move_err) => tracing::error!("{e}; could not move it aside: {move_err}"),
                }
                // Each pass drops a broken file, so this ends.
                if text.take().is_none() {
                    break (Settings::default(), false);
                }
            }
        }
    };
    let created = texts.synced.is_none() && unreadable.is_none();
    if created || migrated {
        let scope = if unreadable.is_some() {
            Scope::LocalOnly
        } else {
            Scope::Both
        };
        match write(files, &settings, scope) {
            Ok(written) => texts = written,
            Err(e) => tracing::error!("could not save settings: {e}"),
        }
    }
    Loaded {
        settings,
        texts,
        unreadable,
    }
}

#[cfg(test)]
mod tests {
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
    fn an_old_single_file_loads_unchanged_and_splits_on_save() {
        let dir = tempfile::tempdir().unwrap();
        let f = files(dir.path());
        let mut old = Settings::default();
        old.search.index_folders = vec![r"D:\Work".into()];
        old.general.launch_at_login = true;
        old.ai.default_provider = ProviderId::Chatgpt;
        super::super::save(&f.synced, &old).unwrap();
        let before = fs::read_to_string(&f.synced).unwrap();

        let loaded = load(&f);
        assert_eq!(loaded.settings, old, "nothing lost");
        assert_eq!(fs::read_to_string(&f.synced).unwrap(), before, "not rewritten");
        assert!(!f.local.exists());

        let texts = write(&f, &loaded.settings, Scope::Both).unwrap();
        assert!(!texts.synced.as_deref().unwrap().contains("D:\\\\Work"));
        assert!(texts.local.as_deref().unwrap().contains("D:\\\\Work"));
        assert_eq!(load(&f).settings, old);
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
        assert_eq!(fs::metadata(&f.synced).unwrap().modified().unwrap(), long_ago);
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
        assert_eq!(loaded.settings.search.index_folders, ["~/src"], "local file still used");
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
        assert!(matches!(settings_from(&bad_synced), Err((Which::Synced, _))));
    }
}
