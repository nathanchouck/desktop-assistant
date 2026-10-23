//! The settings files in use, and what they held when we last read or
//! wrote them. Every save goes through here, so a change seen on disk can
//! be told apart from our own write: the files then hold exactly what we
//! wrote. Moving to another settings folder is done here too.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use super::disk::{self, Files, Scope, Seen, Texts, Which};
use super::folder::{self, FolderError, Switch};
use super::{Settings, SettingsError};
use crate::paths::AppPaths;

pub struct SettingsFiles {
    paths: AppPaths,
    known: Mutex<Known>,
}

#[derive(Debug, Clone)]
struct Known {
    files: Files,
    /// What the files held when we last read or wrote them.
    texts: Texts,
    /// `settings.toml` couldn't be read at startup (a cloud file while
    /// OneDrive isn't running): only this PC's file is written until it
    /// can be.
    unreadable: Option<String>,
    /// The chosen folder isn't there; the default one is used meanwhile.
    missing: Option<PathBuf>,
}

/// What a look at the files found.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    /// Nothing new: our own write, a touch, or a file briefly gone.
    None,
    /// Changed elsewhere (another PC, a hand edit): these are the settings
    /// now.
    Reloaded(Box<Settings>),
    /// Changed elsewhere but not usable (a typo, a half-synced file). The
    /// current settings stay; reported once per version of the file.
    Broken(String),
}

/// How a move to another folder went.
#[derive(Debug, Clone, PartialEq)]
pub enum Moved {
    /// This PC's settings were written there.
    Wrote {
        /// The file that was there before, copied aside.
        backup: Option<PathBuf>,
    },
    /// The folder had settings (from another PC), and these are now in use.
    Adopted(Box<Settings>),
}

#[derive(Debug, thiserror::Error)]
pub enum MoveError {
    #[error(transparent)]
    Folder(#[from] FolderError),
    #[error(transparent)]
    Settings(#[from] SettingsError),
    #[error("The settings there can't be used ({0}). Replace them with this PC's instead?")]
    Unusable(String),
}

/// Where settings are, for the settings menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    /// The folder `settings.toml` is read from now.
    pub folder: PathBuf,
    pub is_default: bool,
    /// A chosen folder that isn't there.
    pub missing: Option<PathBuf>,
    /// Why `settings.toml` couldn't be read, if it couldn't.
    pub unreadable: Option<String>,
}

impl SettingsFiles {
    /// Reads the settings at startup (see `disk::load`). Never fails: the
    /// worst case is defaults.
    pub fn load(paths: AppPaths) -> (Self, Settings) {
        let dir = paths.settings_dir();
        if let Some(missing) = &dir.missing {
            crate::toast::report(format!(
                "Your settings folder {} isn't there; using {} for now",
                missing.display(),
                dir.folder.display()
            ));
        }
        let files = Files::in_folder(&dir.folder, &paths);
        let loaded = disk::load(&files);
        let known = Known {
            files,
            texts: loaded.texts,
            unreadable: loaded.unreadable,
            missing: dir.missing,
        };
        let this = Self {
            paths,
            known: Mutex::new(known),
        };
        (this, loaded.settings)
    }

    /// Saves settings from the menu: this PC's keys to its own file, the
    /// rest to `settings.toml`.
    pub fn save(&self, settings: &Settings) -> Result<(), SettingsError> {
        let mut known = self.lock();
        let scope = if known.unreadable.is_some() {
            Scope::LocalOnly
        } else {
            Scope::Both
        };
        known.texts = disk::write(&known.files, settings, scope)?;
        Ok(())
    }

    /// The files in use, to watch.
    pub fn files(&self) -> Files {
        self.lock().files.clone()
    }

    pub fn location(&self) -> Location {
        let known = self.lock();
        let folder = known
            .files
            .synced
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_default();
        Location {
            is_default: folder::same_folder(&folder, self.paths.default_settings_dir()),
            folder,
            missing: known.missing.clone(),
            unreadable: known.unreadable.clone(),
        }
    }

    /// Looks at the files again (they or the folder may have changed while
    /// running). `current` is what's in use now, written to a chosen folder
    /// that has come back without a settings file.
    pub fn check(&self, current: &Settings) -> Change {
        let mut known = self.lock();
        let dir = self.paths.settings_dir();
        known.missing = dir.missing.clone();
        if let Some(missing) = &dir.missing {
            if known.files.synced.parent() == Some(missing.as_path()) {
                // Gone while running (renamed, a drive unplugged): keep the
                // settings in use rather than switch to the default folder's
                // older ones. A save makes the folder again if it can.
                return Change::None;
            }
        }
        let files = Files::in_folder(&dir.folder, &self.paths);
        let moved = files != known.files;
        if moved {
            tracing::info!("settings folder is now {}", dir.folder.display());
        }
        // A file that can't be read now (locked mid-sync, a cloud file
        // being fetched) is tried again on the next look.
        let texts = match files.read() {
            Ok(texts) => texts,
            Err(e) => {
                tracing::debug!("can't read the settings files yet: {e}");
                return Change::None;
            }
        };
        if moved && texts.synced.is_none() {
            // The chosen folder is back but empty: it gets these settings.
            known.files = files;
            known.unreadable = None;
            return match disk::write(&known.files, current, Scope::Both) {
                Ok(written) => {
                    known.texts = written;
                    Change::None
                }
                Err(e) => Change::Broken(e.to_string()),
            };
        }
        if !moved {
            match disk::compare(&known.texts, &texts) {
                Seen::Unchanged => return Change::None,
                // A sync replacing the file: wait for the new one. The next
                // save writes it again if it never comes.
                Seen::Gone => return Change::None,
                Seen::Changed => {}
            }
        }
        known.files = files;
        known.unreadable = None;
        let found = disk::settings_from(&texts);
        known.texts = texts;
        match found {
            Ok((settings, _)) => Change::Reloaded(Box::new(settings)),
            Err((which, e)) => Change::Broken(format!("{} has a mistake: {e}", file_name(which))),
        }
    }

    /// Makes `folder` the settings folder. A folder that already has
    /// settings (another PC put them there) is adopted unless `replace`;
    /// otherwise `current` is written there, the file it replaces copied
    /// to `settings.toml.bak`. This PC's own settings stay where they are.
    pub fn move_to(
        &self,
        folder: &Path,
        current: &Settings,
        replace: bool,
    ) -> Result<Moved, MoveError> {
        let probe = folder::probe(folder)?;
        let mut known = self.lock();
        // This PC's values go to its own file first, so they're kept
        // whichever settings are used from now on.
        known.texts.local = disk::write(&known.files, current, Scope::LocalOnly)?.local;
        let target = Files::in_folder(folder, &self.paths);
        let in_use = target == known.files;
        let (texts, moved) = match folder::switch_for(&probe, replace) {
            _ if in_use => (known.texts.clone(), Moved::Wrote { backup: None }),
            Switch::Adopt => {
                let texts = target
                    .read()
                    .map_err(|e| MoveError::Unusable(e.to_string()))?;
                let (settings, _) = disk::settings_from(&texts)
                    .map_err(|(_, e)| MoveError::Unusable(e.to_string()))?;
                (texts, Moved::Adopted(Box::new(settings)))
            }
            Switch::Write => {
                fs::create_dir_all(folder)
                    .map_err(|e| FolderError::io("could not create", folder, e))?;
                let backup = folder::back_up(&target.synced)?;
                let texts = disk::copy_to(&target, current, &known.texts)?;
                (texts, Moved::Wrote { backup })
            }
        };
        folder::remember(&self.paths, folder)?;
        tracing::info!("settings folder set to {}", folder.display());
        known.files = target;
        known.texts = texts;
        known.unreadable = None;
        known.missing = None;
        Ok(moved)
    }

    fn lock(&self) -> MutexGuard<'_, Known> {
        // A panic while holding the lock leaves the last state, still usable.
        self.known
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn file_name(which: Which) -> &'static str {
    match which {
        Which::Synced => "settings.toml",
        Which::Local => "settings.local.toml",
    }
}

#[cfg(test)]
mod tests;
