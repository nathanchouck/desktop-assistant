//! Reading and writing the two settings files: `settings.toml` in the
//! settings folder (maybe a synced one) and `settings.local.toml` in the
//! local data folder (this PC only).

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use toml::Table;

use super::{default_table, layers, migrate, Settings, SettingsError};
use crate::paths::{AppPaths, SETTINGS_FILE};

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
        Self::in_folder(&paths.settings_dir().folder, paths)
    }

    /// The files with `settings.toml` in `folder`.
    pub fn in_folder(folder: &Path, paths: &AppPaths) -> Self {
        Self {
            synced: folder.join(SETTINGS_FILE),
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
        synced: Some(format!(
            "{SYNCED_HEADER}{}",
            toml::to_string_pretty(&synced)?
        )),
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
    let on_disk = on_disk(files);
    let wanted = texts_for(settings, &on_disk)?;
    replace(files, on_disk, wanted, scope)
}

/// Writes `settings` to `target`, another settings folder, keeping the
/// keys that `source` (the files in use) has and this version doesn't
/// know, so a move carries the whole file.
pub fn copy_to(
    target: &Files,
    settings: &Settings,
    source: &Texts,
) -> Result<Texts, SettingsError> {
    let wanted = texts_for(settings, source)?;
    replace(target, on_disk(target), wanted, Scope::Both)
}

fn on_disk(files: &Files) -> Texts {
    Texts {
        synced: read_text(&files.synced).ok().flatten(),
        local: read_text(&files.local).ok().flatten(),
    }
}

/// Writes each file whose text changes from `now` to `wanted`.
fn replace(
    files: &Files,
    mut now: Texts,
    wanted: Texts,
    scope: Scope,
) -> Result<Texts, SettingsError> {
    if scope == Scope::Both && wanted.synced != now.synced {
        if let Some(text) = &wanted.synced {
            keep_if_broken(&files.synced, now.synced.as_deref());
            write_atomic(&files.synced, text)?;
            now.synced = wanted.synced;
        }
    }
    if wanted.local != now.local {
        if let Some(text) = &wanted.local {
            keep_if_broken(&files.local, now.local.as_deref());
            write_atomic(&files.local, text)?;
            now.local = wanted.local;
        }
    }
    Ok(now)
}

/// Copies a file that doesn't parse to `<name>.bad` before a save replaces
/// it: a hand edit with a typo shouldn't just vanish.
fn keep_if_broken(path: &Path, text: Option<&str>) {
    let Some(text) = text else { return };
    if toml::from_str::<Table>(text).is_ok() {
        return;
    }
    let bad = bad_name(path);
    match fs::copy(path, &bad) {
        Ok(_) => tracing::warn!(
            "{} had an error; kept a copy as {}",
            path.display(),
            bad.display()
        ),
        Err(e) => tracing::warn!("could not keep a copy of {}: {e}", path.display()),
    }
}

fn bad_name(path: &Path) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_owned();
    name.push(".bad");
    path.with_file_name(name)
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
    let bad = bad_name(path);
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
    // No file for this PC yet: the first start since settings were split
    // (or it was deleted). Writing now moves this PC's values out of the
    // one old file at once, so a later move to a synced folder can't
    // leave them behind.
    let first_split = texts.local.is_none();
    if created || migrated || first_split {
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
mod tests;
