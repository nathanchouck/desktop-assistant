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
    Reloaded(Settings),
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
    Adopted(Settings),
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
            tracing::warn!(
                "settings folder {} isn't there; using {} for now",
                missing.display(),
                dir.folder.display()
            );
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
        let files = Files::in_folder(&dir.folder, &self.paths);
        let moved = files != known.files;
        if moved {
            tracing::info!("settings folder is now {}", dir.folder.display());
        }
        known.missing = dir.missing;
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
            Ok((settings, _)) => Change::Reloaded(settings),
            Err((which, e)) => Change::Broken(format!("{} has a mistake: {e}", file_name(which))),
        }
    }

    /// Makes `folder` the settings folder. A folder that already has
    /// settings (another PC put them there) is adopted unless `replace`;
    /// otherwise `current` is written there, the file it replaces copied
    /// to `settings.toml.bak`. This PC's own settings stay where they are.
    pub fn move_to(&self, folder: &Path, current: &Settings, replace: bool) -> Result<Moved, MoveError> {
        let probe = folder::probe(folder)?;
        let mut known = self.lock();
        // This PC's values go to its own file first, so they're kept
        // whichever settings are used from now on.
        disk::write(&known.files, current, Scope::LocalOnly)?;
        let target = Files::in_folder(folder, &self.paths);
        let in_use = target == known.files;
        let (texts, moved) = match folder::switch_for(&probe, replace) {
            _ if in_use => (known.texts.clone(), Moved::Wrote { backup: None }),
            Switch::Adopt => {
                let texts = target.read().map_err(|e| MoveError::Unusable(e.to_string()))?;
                let (settings, _) =
                    disk::settings_from(&texts).map_err(|(_, e)| MoveError::Unusable(e.to_string()))?;
                (texts, Moved::Adopted(settings))
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
        self.known.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }
}

fn file_name(which: Which) -> &'static str {
    match which {
        Which::Synced => "settings.toml",
        Which::Local => "settings.local.toml",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{LOCAL_SETTINGS_FILE, SETTINGS_FILE};

    struct Pc {
        _dir: tempfile::TempDir,
        root: PathBuf,
        paths: AppPaths,
    }

    /// A PC with its own config and data folders, and a "OneDrive" folder
    /// under `cloud` (shared between PCs in a test).
    fn pc(cloud: Option<&Path>) -> (Pc, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_owned();
        let paths = AppPaths::with_roots(&root.join("cfg"), &root.join("data"));
        let onedrive = cloud.map(Path::to_owned).unwrap_or_else(|| root.join("OneDrive"));
        fs::create_dir_all(&onedrive).unwrap();
        let synced = onedrive.join("desktop-assistant");
        (
            Pc {
                _dir: dir,
                root,
                paths,
            },
            synced,
        )
    }

    fn edit(path: &Path, from: &str, to: &str) {
        let text = fs::read_to_string(path).unwrap();
        assert!(text.contains(from), "{from} not in {text}");
        fs::write(path, text.replace(from, to)).unwrap();
    }

    #[test]
    fn our_own_saves_are_not_changes() {
        let (pc, _) = pc(None);
        let (files, mut s) = SettingsFiles::load(pc.paths.clone());
        assert_eq!(files.check(&s), Change::None);
        s.appearance.tint = "teal".into();
        s.search.index_folders = vec!["~/src".into()];
        files.save(&s).unwrap();
        assert_eq!(files.check(&s), Change::None);
    }

    #[test]
    fn a_change_made_elsewhere_is_reloaded_once() {
        let (pc, _) = pc(None);
        let (files, s) = SettingsFiles::load(pc.paths.clone());
        let synced = files.files().synced;
        edit(&synced, "toggle = \"Ctrl+Space\"", "toggle = \"Alt+Space\"");
        let Change::Reloaded(new) = files.check(&s) else {
            panic!("not reloaded");
        };
        assert_eq!(new.hotkeys.toggle, "Alt+Space");
        assert_eq!(files.check(&new), Change::None);

        // This PC's file edited by hand counts too.
        edit(&pc.paths.local_settings_file(), "launch_at_login = false", "launch_at_login = true");
        let Change::Reloaded(newer) = files.check(&new) else {
            panic!("not reloaded");
        };
        assert!(newer.general.launch_at_login);
        assert_eq!(newer.hotkeys.toggle, "Alt+Space");
    }

    #[test]
    fn a_broken_edit_is_reported_once_and_kept_on_disk() {
        let (pc, _) = pc(None);
        let (files, s) = SettingsFiles::load(pc.paths.clone());
        let synced = files.files().synced;
        fs::write(&synced, "[hotkeys\n").unwrap();
        let Change::Broken(message) = files.check(&s) else {
            panic!("not reported");
        };
        assert!(message.starts_with("settings.toml has a mistake"), "{message}");
        assert_eq!(files.check(&s), Change::None);
        assert_eq!(fs::read_to_string(&synced).unwrap(), "[hotkeys\n");
        // Fixed: picked up.
        fs::write(&synced, "[search]\nmax_results = 4\n").unwrap();
        let Change::Reloaded(fixed) = files.check(&s) else {
            panic!("not reloaded");
        };
        assert_eq!(fixed.search.max_results, 4);
    }

    #[test]
    fn a_file_briefly_gone_is_not_a_change() {
        let (pc, _) = pc(None);
        let (files, s) = SettingsFiles::load(pc.paths.clone());
        let synced = files.files().synced;
        let text = fs::read_to_string(&synced).unwrap();
        fs::remove_file(&synced).unwrap();
        assert_eq!(files.check(&s), Change::None);
        fs::write(&synced, text).unwrap();
        assert_eq!(files.check(&s), Change::None);
    }

    #[test]
    fn moving_writes_there_and_keeps_this_pcs_settings_local() {
        let (pc, synced) = pc(None);
        let (files, mut s) = SettingsFiles::load(pc.paths.clone());
        s.appearance.tint = "pink".into();
        s.search.index_folders = vec![r"D:\Work".into()];
        files.save(&s).unwrap();
        let old_file = files.files().synced;

        let moved = files.move_to(&synced, &s, false).unwrap();
        assert_eq!(moved, Moved::Wrote { backup: None });
        assert_eq!(pc.paths.settings_file(), synced.join(SETTINGS_FILE));
        assert_eq!(files.files().synced, synced.join(SETTINGS_FILE));
        let text = fs::read_to_string(synced.join(SETTINGS_FILE)).unwrap();
        assert!(text.contains("tint = \"pink\""), "{text}");
        assert!(!text.contains(r"D:\Work"), "{text}");
        assert!(old_file.exists(), "the old file stays where it was");
        assert!(!files.location().is_default);
        assert_eq!(files.check(&s), Change::None);

        // A restart reads the same settings from the new folder.
        let (_, again) = SettingsFiles::load(pc.paths.clone());
        assert_eq!(again, s);
    }

    #[test]
    fn a_second_pc_adopts_the_shared_settings_and_keeps_its_own() {
        let cloud = tempfile::tempdir().unwrap();
        let (first, synced) = pc(Some(cloud.path()));
        let (files_a, mut a) = SettingsFiles::load(first.paths.clone());
        a.appearance.tint = "green".into();
        a.search.index_folders = vec![r"C:\A".into()];
        files_a.move_to(&synced, &a, false).unwrap();

        // The second PC has an old single settings.toml of its own.
        let (second, _) = pc(Some(cloud.path()));
        let mut b = Settings::default();
        b.appearance.tint = "red".into();
        b.search.index_folders = vec![r"E:\B".into()];
        b.hotkeys.win_key_override = false;
        super::super::save(&second.paths.settings_file(), &b).unwrap();
        let (files_b, loaded_b) = SettingsFiles::load(second.paths.clone());
        assert_eq!(loaded_b, b);

        let Moved::Adopted(adopted) = files_b.move_to(&synced, &loaded_b, false).unwrap() else {
            panic!("not adopted");
        };
        assert_eq!(adopted.appearance.tint, "green", "the shared look");
        assert_eq!(adopted.search.index_folders, [r"E:\B"], "this PC's folders");
        assert!(!adopted.hotkeys.win_key_override, "this PC's Windows key");

        // A change on the second PC reaches the first; machine keys don't.
        let mut b2 = adopted.clone();
        b2.appearance.tint = "orange".into();
        b2.search.index_folders = vec![r"E:\B2".into()];
        files_b.save(&b2).unwrap();
        let Change::Reloaded(a2) = files_a.check(&a) else {
            panic!("first PC didn't see it");
        };
        assert_eq!(a2.appearance.tint, "orange");
        assert_eq!(a2.search.index_folders, [r"C:\A"]);
        assert!(a2.hotkeys.win_key_override);
        // And the first PC saving a per-PC value doesn't touch the shared file.
        let shared_before = fs::read_to_string(synced.join(SETTINGS_FILE)).unwrap();
        let mut a3 = a2.clone();
        a3.general.launch_at_login = true;
        files_a.save(&a3).unwrap();
        assert_eq!(fs::read_to_string(synced.join(SETTINGS_FILE)).unwrap(), shared_before);
        assert_eq!(files_b.check(&b2), Change::None);
    }

    #[test]
    fn replacing_backs_up_the_file_there() {
        let cloud = tempfile::tempdir().unwrap();
        let (pc, synced) = pc(Some(cloud.path()));
        fs::create_dir_all(&synced).unwrap();
        fs::write(synced.join(SETTINGS_FILE), "[appearance]\ntint = 'red'\n").unwrap();
        let (files, mut s) = SettingsFiles::load(pc.paths.clone());
        s.appearance.tint = "teal".into();
        let moved = files.move_to(&synced, &s, true).unwrap();
        let backup = synced.join("settings.toml.bak");
        assert_eq!(moved, Moved::Wrote { backup: Some(backup.clone()) });
        assert_eq!(fs::read_to_string(backup).unwrap(), "[appearance]\ntint = 'red'\n");
        let (_, again) = SettingsFiles::load(pc.paths.clone());
        assert_eq!(again.appearance.tint, "teal");
    }

    #[test]
    fn unusable_settings_there_are_not_adopted() {
        let cloud = tempfile::tempdir().unwrap();
        let (pc, synced) = pc(Some(cloud.path()));
        fs::create_dir_all(&synced).unwrap();
        fs::write(synced.join(SETTINGS_FILE), "[[[").unwrap();
        let (files, s) = SettingsFiles::load(pc.paths.clone());
        assert!(matches!(files.move_to(&synced, &s, false), Err(MoveError::Unusable(_))));
        assert!(files.location().is_default, "still the default folder");
        assert!(!pc.paths.settings_pointer_file().exists());
    }

    #[test]
    fn back_to_the_default_folder() {
        let (pc, synced) = pc(None);
        let (files, mut s) = SettingsFiles::load(pc.paths.clone());
        files.move_to(&synced, &s, false).unwrap();
        s.quicklinks.enabled = false;
        files.save(&s).unwrap();
        let default = pc.paths.default_settings_dir().to_owned();
        let moved = files.move_to(&default, &s, true).unwrap();
        assert!(matches!(moved, Moved::Wrote { backup: Some(_) }));
        assert!(files.location().is_default);
        assert!(!pc.paths.settings_pointer_file().exists());
        let (_, again) = SettingsFiles::load(pc.paths.clone());
        assert_eq!(again, s, "the settings in use came along");
    }

    #[test]
    fn a_missing_folder_falls_back_and_is_followed_when_it_returns() {
        let (pc, synced) = pc(None);
        let (files, mut s) = SettingsFiles::load(pc.paths.clone());
        s.appearance.tint = "purple".into();
        files.move_to(&synced, &s, false).unwrap();
        files.save(&s).unwrap();

        // OneDrive isn't there at the next start (another drive letter,
        // signed out): the default folder is used meanwhile.
        let parked = pc.root.join("parked");
        fs::rename(&synced, &parked).unwrap();
        let (files, fallback) = SettingsFiles::load(pc.paths.clone());
        let location = files.location();
        assert!(location.is_default);
        assert_eq!(location.missing.as_deref(), Some(synced.as_path()));
        assert_eq!(fallback.appearance.tint, "blue", "the default folder's file");

        // It comes back: its settings are used.
        fs::rename(&parked, &synced).unwrap();
        let Change::Reloaded(back) = files.check(&fallback) else {
            panic!("not followed");
        };
        assert_eq!(back.appearance.tint, "purple");
        assert_eq!(files.location().missing, None);
        assert_eq!(files.files().synced, synced.join(SETTINGS_FILE));
    }

    #[test]
    fn a_chosen_folder_back_without_a_file_gets_the_current_settings() {
        let (pc, synced) = pc(None);
        let (files, s) = SettingsFiles::load(pc.paths.clone());
        files.move_to(&synced, &s, false).unwrap();
        fs::remove_dir_all(&synced).unwrap();
        let (files, mut fallback) = SettingsFiles::load(pc.paths.clone());
        fallback.appearance.tint = "teal".into();
        fs::create_dir_all(&synced).unwrap();
        assert_eq!(files.check(&fallback), Change::None);
        let text = fs::read_to_string(synced.join(SETTINGS_FILE)).unwrap();
        assert!(text.contains("tint = \"teal\""), "{text}");
        assert!(pc.paths.data_dir.join(LOCAL_SETTINGS_FILE).exists());
    }
}
