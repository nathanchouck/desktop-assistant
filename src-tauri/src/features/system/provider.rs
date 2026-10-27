//! System rows: lock, sleep, restart and the like, and Windows Settings
//! pages. What ends your session or deletes things asks first (unless
//! `system.confirm` is off).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use async_trait::async_trait;

use super::commands::{
    theme_subtitle, BinContents, Exit, SettingsPage, SystemCommand, SETTINGS_PAGES,
};
use super::{matching, os};
use crate::features::media::{self, MediaKey};
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::settings::{self, SharedSettings};

const NAME: &str = "system";
const ID_PREFIX: &str = "sys:";
/// Recycle Bin contents older than this are looked up again (in the
/// background; the row shows what's known until then).
const BIN_MAX_AGE: Duration = Duration::from_secs(30);
/// Between hiding the launcher and sleeping, so it isn't still on screen
/// when the PC wakes up.
const SLEEP_DELAY: Duration = Duration::from_millis(400);

pub struct SystemProvider {
    settings: SharedSettings,
    /// Whether hibernation is on (checked once; it rarely changes).
    hibernate: bool,
    bin: Arc<BinCache>,
}

impl SystemProvider {
    pub fn new(settings: SharedSettings) -> Self {
        Self {
            settings,
            hibernate: os::hibernate_available(),
            bin: Arc::new(BinCache::default()),
        }
    }

    fn command_row(&self, cmd: SystemCommand, confirm: bool) -> ResultItem {
        let mut subtitle = cmd.subtitle().to_owned();
        // Always ask when asking is on, even when the bin last looked
        // empty: that look can be stale, and emptying can't be undone. An
        // empty bin just answers the second Enter with a notice.
        let ask = confirm.then(|| cmd.confirm()).flatten();
        match cmd {
            SystemCommand::EmptyRecycleBin => {
                if let Some(contents) = self.bin.get() {
                    subtitle = contents.summary();
                }
            }
            SystemCommand::ToggleDarkMode => {
                subtitle = theme_subtitle(os::apps_use_light_theme()).to_owned();
            }
            _ => {}
        }
        let row = ResultItem::new(cmd.id(), ItemKind::Command, cmd.title())
            .with_subtitle(subtitle)
            .with_icon(glyph(cmd.glyph()))
            .with_actions([Action::Run]);
        match ask {
            Some(question) => row.with_confirm(question),
            None => row,
        }
    }

    async fn run(&self, cmd: SystemCommand) -> Result<Activation, SearchError> {
        tracing::info!("system command: {}", cmd.id());
        match cmd {
            SystemCommand::Lock => blocking(os::lock).await?,
            SystemCommand::Sleep => after_hide(|| os::suspend(false))?,
            SystemCommand::Hibernate => after_hide(|| os::suspend(true))?,
            SystemCommand::Restart => blocking(|| os::exit_windows(Exit::Restart)).await?,
            SystemCommand::ShutDown => blocking(|| os::exit_windows(Exit::ShutDown)).await?,
            SystemCommand::SignOut => blocking(|| os::exit_windows(Exit::SignOut)).await?,
            SystemCommand::EmptyRecycleBin => return self.empty_recycle_bin().await,
            SystemCommand::ToggleDarkMode => {
                let light = blocking(os::toggle_theme).await?;
                tracing::info!("switched to {} mode", if light { "light" } else { "dark" });
            }
            SystemCommand::Mute => media::press_after_hide(MediaKey::Mute).map_err(failed)?,
        }
        Ok(Activation::Hide)
    }

    async fn empty_recycle_bin(&self) -> Result<Activation, SearchError> {
        let before = blocking(os::recycle_bin).await.ok();
        if before.is_some_and(BinContents::is_empty) {
            self.bin.set(BinContents::default());
            return Ok(notice("The Recycle Bin is already empty."));
        }
        blocking(os::empty_recycle_bin).await?;
        self.bin.set(BinContents::default());
        Ok(notice(match before {
            Some(contents) => format!("Emptied the Recycle Bin ({}).", contents.summary()),
            None => "Emptied the Recycle Bin.".into(),
        }))
    }
}

/// Rows for `query`: system commands and Settings pages that match.
fn matching_rows(
    provider: &SystemProvider,
    query: &str,
    confirm: bool,
    limit: usize,
) -> Vec<ResultItem> {
    if !matching::long_enough(query) {
        return Vec::new();
    }
    let mut fuzzy = FuzzyQuery::new(query);
    let commands = SystemCommand::ALL
        .into_iter()
        .filter(|&cmd| cmd != SystemCommand::Hibernate || provider.hibernate)
        .filter_map(|cmd| {
            let score = matching::score(&mut fuzzy, cmd.title(), cmd.keywords())?;
            Some(provider.command_row(cmd, confirm).with_score(score))
        })
        .collect::<Vec<_>>();
    let pages = SETTINGS_PAGES.iter().filter_map(|page| {
        let score = matching::score(&mut fuzzy, page.title, page.keywords)?;
        Some(
            ResultItem::new(page.id(), ItemKind::Command, page.title)
                .with_subtitle("Windows Settings")
                .with_icon(glyph(page.glyph))
                .with_score(score)
                .with_actions([Action::Open]),
        )
    });
    let mut rows: Vec<ResultItem> = commands.into_iter().chain(pages).collect();
    rows.sort_by(|a, b| b.score.total_cmp(&a.score));
    rows.truncate(limit);
    rows
}

#[async_trait]
impl SearchProvider for SystemProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let system = settings::current(&self.settings).system;
        if !system.enabled {
            return Ok(Vec::new());
        }
        Ok(matching_rows(self, query, system.confirm, limit))
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(ID_PREFIX)
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        if !os::SUPPORTED {
            return Ok(notice("System commands work only on Windows."));
        }
        if let Some(page) = SettingsPage::from_id(id) {
            return Ok(Activation::OpenUrl { url: page.uri() });
        }
        let cmd = SystemCommand::from_id(id).ok_or_else(|| failed(format!("{id} is unknown")))?;
        self.run(cmd).await
    }
}

/// The Recycle Bin's contents as last seen. Looking can take a moment with
/// many files or drives, so it happens in the background.
#[derive(Default)]
struct BinCache {
    seen: Mutex<Option<(Instant, BinContents)>>,
    looking: AtomicBool,
}

impl BinCache {
    /// What was seen last; looks again in the background when that's old.
    fn get(self: &Arc<Self>) -> Option<BinContents> {
        let seen = self.seen.lock().ok().and_then(|s| *s);
        if seen.is_none_or(|(at, _)| at.elapsed() > BIN_MAX_AGE) {
            self.look();
        }
        seen.map(|(_, contents)| contents)
    }

    fn set(&self, contents: BinContents) {
        if let Ok(mut seen) = self.seen.lock() {
            *seen = Some((Instant::now(), contents));
        }
    }

    fn look(self: &Arc<Self>) {
        if !os::SUPPORTED || self.looking.swap(true, Ordering::AcqRel) {
            return;
        }
        let cache = Arc::clone(self);
        let started = std::thread::Builder::new()
            .name("recycle-bin".into())
            .spawn(move || {
                match os::recycle_bin() {
                    Ok(contents) => cache.set(contents),
                    Err(e) => tracing::debug!("could not look in the Recycle Bin: {e}"),
                }
                cache.looking.store(false, Ordering::Release);
            });
        if started.is_err() {
            self.looking.store(false, Ordering::Release);
        }
    }
}

fn notice(message: impl Into<String>) -> Activation {
    Activation::Notice {
        message: message.into(),
    }
}

fn failed(message: impl ToString) -> SearchError {
    SearchError::Provider {
        provider: NAME,
        message: message.to_string(),
    }
}

/// Runs a Windows call off the async runtime.
async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, SearchError> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(failed)?
        .map_err(failed)
}

/// Runs `work` shortly after the launcher hides. Sleep doesn't return
/// until the PC wakes, so a failure is a toast, seen on the next open.
fn after_hide(
    work: impl FnOnce() -> Result<(), String> + Send + 'static,
) -> Result<(), SearchError> {
    std::thread::Builder::new()
        .name("system-command".into())
        .spawn(move || {
            std::thread::sleep(SLEEP_DELAY);
            if let Err(e) = work() {
                crate::toast::report(format!("That didn't work: {e}"));
            }
        })
        .map(|_| ())
        .map_err(failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::RwLock;

    fn provider() -> SystemProvider {
        SystemProvider::new(Arc::new(RwLock::new(Settings::default())))
    }

    async fn ids(p: &SystemProvider, query: &str) -> Vec<String> {
        p.query(query, 8)
            .await
            .unwrap()
            .into_iter()
            .map(|r| r.id)
            .collect()
    }

    #[tokio::test]
    async fn finds_commands_by_name_and_keyword() {
        let p = provider();
        assert_eq!(ids(&p, "lock").await[0], "sys:lock");
        assert_eq!(ids(&p, "sleep").await[0], "sys:sleep");
        assert_eq!(ids(&p, "reboot").await[0], "sys:restart");
        assert_eq!(ids(&p, "shutdown").await[0], "sys:shut-down");
        assert_eq!(ids(&p, "log out").await[0], "sys:sign-out");
        assert_eq!(ids(&p, "empty trash").await[0], "sys:empty-recycle-bin");
        assert_eq!(ids(&p, "dark mode").await[0], "sys:dark-mode");
        assert_eq!(ids(&p, "unmute").await[0], "sys:mute");
        assert_eq!(ids(&p, "bluetooth").await[0], "sys:page:bluetooth");
        assert_eq!(ids(&p, "wifi").await[0], "sys:page:network-wifi");
        assert_eq!(ids(&p, "windows update").await[0], "sys:page:windowsupdate");
        assert_eq!(ids(&p, "uninstall").await[0], "sys:page:appsfeatures");
        assert!(ids(&p, "s").await.is_empty());
        assert!(ids(&p, "photoshop").await.is_empty());
    }

    #[tokio::test]
    async fn risky_rows_ask_unless_turned_off() {
        let p = provider();
        let rows = p.query("restart", 8).await.unwrap();
        assert_eq!(rows[0].confirm.as_deref(), Some("Restart your PC?"));
        let lock = &p.query("lock", 8).await.unwrap()[0];
        assert_eq!(lock.confirm, None);

        if let Ok(mut s) = p.settings.write() {
            s.system.confirm = false;
        }
        assert_eq!(p.query("restart", 8).await.unwrap()[0].confirm, None);
        if let Ok(mut s) = p.settings.write() {
            s.system.enabled = false;
        }
        assert!(p.query("restart", 8).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn the_bin_always_asks_even_when_it_looked_empty() {
        // The last look can be stale: files may have been deleted since.
        let p = provider();
        p.bin.set(BinContents::default());
        let row = &p.query("empty recycle bin", 8).await.unwrap()[0];
        assert_eq!(row.subtitle, "Already empty");
        assert!(row.confirm.is_some());
        p.bin.set(BinContents {
            items: 3,
            bytes: 4096,
        });
        let row = &p.query("empty recycle bin", 8).await.unwrap()[0];
        assert_eq!(row.subtitle, "3 items · 4 KB");
        assert!(row.confirm.is_some());
    }

    #[test]
    fn hibernate_shows_only_when_available() {
        let mut p = provider();
        p.hibernate = false;
        let rows = matching_rows(&p, "hibernate", true, 8);
        assert!(rows.iter().all(|r| r.id != "sys:hibernate"));
        p.hibernate = true;
        let rows = matching_rows(&p, "hibernate", true, 8);
        assert_eq!(rows[0].id, "sys:hibernate");
        assert_eq!(rows[0].confirm.as_deref(), Some("Hibernate the PC?"));
    }

    #[tokio::test]
    async fn owns_its_ids_only() {
        let p = provider();
        assert!(p.owns("sys:lock") && p.owns("sys:page:display"));
        assert!(!p.owns("cmd:restart"));
    }
}
