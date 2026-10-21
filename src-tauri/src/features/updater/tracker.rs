//! Where an update stands (checking, downloading, ready, ...) and the
//! wording shown for it. Pure: the manager feeds it what happened.

use serde::Serialize;

/// What the updater is doing. Shown in the settings menu and search.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum UpdatePhase {
    /// This build has no public key to check updates with.
    NotSetUp,
    /// Not checked yet since the app started.
    Idle,
    Checking,
    UpToDate,
    /// `percent` once the download's size is known.
    Downloading {
        version: String,
        percent: Option<u32>,
    },
    /// Downloaded and its signature checked: installs on restart.
    Ready { version: String },
    /// The installer is starting; the app is about to close.
    Installing { version: String },
    /// A check or download failed; `message` says which and why.
    Failed { message: String },
}

impl UpdatePhase {
    /// One line for the settings menu and the search row.
    pub fn summary(&self) -> String {
        match self {
            Self::NotSetUp => "Updates aren't set up for this build".into(),
            Self::Idle => "Not checked yet".into(),
            Self::Checking => "Checking for updates…".into(),
            Self::UpToDate => "Up to date".into(),
            Self::Downloading {
                version,
                percent: Some(p),
            } => format!("Downloading v{version}… {p}%"),
            Self::Downloading { version, .. } => format!("Downloading v{version}…"),
            Self::Ready { version } => format!("v{version} is ready: restart to install"),
            Self::Installing { version } => format!("Installing v{version}…"),
            Self::Failed { message } => message.clone(),
        }
    }
}

/// Moves `UpdatePhase` along as checks, downloads and installs happen.
/// Events that don't fit the current phase (a late check result after an
/// install started) are ignored.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tracker {
    phase: UpdatePhase,
    /// The version downloaded and verified, kept while a later check runs.
    ready: Option<String>,
}

impl Tracker {
    /// Starts "not set up" until `enable` says this build can update.
    pub fn new() -> Self {
        Self {
            phase: UpdatePhase::NotSetUp,
            ready: None,
        }
    }

    pub fn enable(&mut self) {
        if self.phase == UpdatePhase::NotSetUp {
            self.phase = UpdatePhase::Idle;
        }
    }

    pub fn phase(&self) -> &UpdatePhase {
        &self.phase
    }

    /// The version that's downloaded and can be installed now.
    pub fn ready(&self) -> Option<&str> {
        self.ready.as_deref()
    }

    /// Marks a check as running. False when one can't start: not set up,
    /// or already checking, downloading or installing.
    pub fn start_check(&mut self) -> bool {
        match self.phase {
            UpdatePhase::NotSetUp
            | UpdatePhase::Checking
            | UpdatePhase::Downloading { .. }
            | UpdatePhase::Installing { .. } => false,
            _ => {
                self.phase = UpdatePhase::Checking;
                true
            }
        }
    }

    /// The check found nothing newer than the running version. A download
    /// waiting to install is dropped too: its release was taken back.
    pub fn up_to_date(&mut self) {
        if self.phase == UpdatePhase::Checking {
            self.ready = None;
            self.phase = UpdatePhase::UpToDate;
        }
    }

    /// The check found `version`. True when it should be downloaded; false
    /// when it's the one already downloaded (or the check was overtaken).
    pub fn found(&mut self, version: &str) -> bool {
        if self.phase != UpdatePhase::Checking {
            return false;
        }
        if let Some(ready) = self.ready.clone().filter(|r| same_version(r, version)) {
            self.phase = UpdatePhase::Ready { version: ready };
            return false;
        }
        self.ready = None;
        self.phase = UpdatePhase::Downloading {
            version: version.to_owned(),
            percent: None,
        };
        true
    }

    /// The check failed. An update already downloaded stays ready.
    pub fn check_failed(&mut self, reason: &str) {
        if self.phase != UpdatePhase::Checking {
            return;
        }
        self.phase = match &self.ready {
            Some(version) => UpdatePhase::Ready {
                version: version.clone(),
            },
            None => UpdatePhase::Failed {
                message: format!("Couldn't check for updates: {reason}"),
            },
        };
    }

    /// `received` bytes of `total` so far. True when the percent shown
    /// changed (so not every chunk sends an event).
    pub fn progress(&mut self, received: u64, total: Option<u64>) -> bool {
        let UpdatePhase::Downloading { percent, .. } = &mut self.phase else {
            return false;
        };
        let now = percent_of(received, total);
        let changed = *percent != now;
        *percent = now;
        changed
    }

    /// The download finished and its signature checked out.
    pub fn downloaded(&mut self) {
        if let UpdatePhase::Downloading { version, .. } = &self.phase {
            let version = version.clone();
            self.ready = Some(version.clone());
            self.phase = UpdatePhase::Ready { version };
        }
    }

    pub fn download_failed(&mut self, reason: &str) {
        if let UpdatePhase::Downloading { version, .. } = &self.phase {
            self.phase = UpdatePhase::Failed {
                message: format!("Couldn't download v{version}: {reason}"),
            };
        }
    }

    /// Marks the ready update as installing and returns its version; `None`
    /// when nothing is downloaded or an install already started.
    pub fn start_install(&mut self) -> Option<String> {
        if matches!(self.phase, UpdatePhase::Installing { .. }) {
            return None;
        }
        let version = self.ready.clone()?;
        self.phase = UpdatePhase::Installing {
            version: version.clone(),
        };
        Some(version)
    }

    /// The installer didn't start; the update is still there to try again.
    pub fn install_failed(&mut self) {
        if let UpdatePhase::Installing { version } = &self.phase {
            self.phase = UpdatePhase::Ready {
                version: version.clone(),
            };
        }
    }
}

/// 0..=100, or `None` while the size is unknown.
fn percent_of(received: u64, total: Option<u64>) -> Option<u32> {
    let total = total.filter(|&t| t > 0)?;
    let percent = received.saturating_mul(100) / total;
    Some(percent.min(100) as u32)
}

/// What asking for a check led to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CheckResult {
    NotSetUp,
    /// Already checking, downloading or installing: no new check.
    Busy(UpdatePhase),
    /// The check ran; the phase it left.
    Done(UpdatePhase),
}

/// The footer notice after "Check for Updates".
pub fn check_notice(result: &CheckResult, current: &str) -> String {
    let phase = match result {
        CheckResult::NotSetUp => return "Updates aren't set up for this build.".into(),
        CheckResult::Busy(phase) => return sentence(phase.summary()),
        CheckResult::Done(phase) => phase,
    };
    match phase {
        UpdatePhase::UpToDate => format!("You're up to date (v{current})."),
        UpdatePhase::Downloading { version, .. } => {
            format!("Found v{version}. Downloading it; you'll see when it's ready.")
        }
        UpdatePhase::Ready { version } => {
            format!("v{version} is ready. Pick “Install Update and Restart” to install it.")
        }
        other => sentence(other.summary()),
    }
}

/// Ends `text` with a full stop unless it already ends a sentence ("…").
fn sentence(mut text: String) -> String {
    if !text.ends_with(['.', '…', '!', '?']) {
        text.push('.');
    }
    text
}

/// Same release, however it's written ("v1.2.0" and "1.2.0").
pub fn same_version(a: &str, b: &str) -> bool {
    match (parse(a), parse(b)) {
        (Some(a), Some(b)) => a == b,
        _ => a.trim().trim_start_matches('v') == b.trim().trim_start_matches('v'),
    }
}

/// Whether `a` is a later release than `b` (false if either isn't a version).
pub fn is_newer(a: &str, b: &str) -> bool {
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a > b)
}

fn parse(version: &str) -> Option<semver::Version> {
    semver::Version::parse(version.trim().trim_start_matches('v')).ok()
}

/// How the last install went, judged when the app starts again.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AfterInstall {
    /// Running the version that was being installed.
    Updated(String),
    /// Still on an older version: the installer didn't finish.
    NotInstalled(String),
}

/// `installing`: the version an install was started for before the app
/// closed, if any; `current`: the version running now.
pub fn after_install(installing: Option<&str>, current: &str) -> Option<AfterInstall> {
    let version = installing?;
    if same_version(version, current) {
        Some(AfterInstall::Updated(version.to_owned()))
    } else if is_newer(version, current) {
        Some(AfterInstall::NotInstalled(version.to_owned()))
    } else {
        // Something newer was installed some other way meanwhile.
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn enabled() -> Tracker {
        let mut t = Tracker::new();
        t.enable();
        t
    }

    fn ready(version: &str) -> UpdatePhase {
        UpdatePhase::Ready {
            version: version.into(),
        }
    }

    #[test]
    fn not_set_up_never_checks() {
        let mut t = Tracker::new();
        assert_eq!(t.phase(), &UpdatePhase::NotSetUp);
        assert!(!t.start_check());
        assert_eq!(t.phase().summary(), "Updates aren't set up for this build");
    }

    #[test]
    fn check_finds_nothing() {
        let mut t = enabled();
        assert_eq!(t.phase(), &UpdatePhase::Idle);
        assert!(t.start_check());
        assert!(!t.start_check(), "one check at a time");
        t.up_to_date();
        assert_eq!(t.phase(), &UpdatePhase::UpToDate);
    }

    #[test]
    fn found_downloads_then_is_ready() {
        let mut t = enabled();
        t.start_check();
        assert!(t.found("0.2.0"));
        assert_eq!(t.phase().summary(), "Downloading v0.2.0…");
        assert!(!t.start_check(), "no check while downloading");
        assert!(!t.progress(10, None), "size unknown: nothing to show");
        assert!(t.progress(40, Some(100)));
        assert!(!t.progress(41, Some(100)), "same percent: no event");
        assert_eq!(t.phase().summary(), "Downloading v0.2.0… 41%");
        assert_eq!(t.start_install(), None, "nothing downloaded yet");
        t.downloaded();
        assert_eq!(t.phase(), &ready("0.2.0"));
        assert_eq!(t.ready(), Some("0.2.0"));
        assert_eq!(t.phase().summary(), "v0.2.0 is ready: restart to install");
    }

    #[test]
    fn a_later_check_keeps_or_replaces_the_download() {
        let mut t = enabled();
        t.start_check();
        t.found("0.2.0");
        t.downloaded();
        // The same release again: nothing to download.
        assert!(t.start_check());
        assert_eq!(t.phase(), &UpdatePhase::Checking);
        assert!(!t.found("v0.2.0"));
        assert_eq!(t.phase(), &ready("0.2.0"));
        // A failed check doesn't lose it.
        t.start_check();
        t.check_failed("offline");
        assert_eq!(t.phase(), &ready("0.2.0"));
        // A newer release replaces it.
        t.start_check();
        assert!(t.found("0.3.0"));
        assert_eq!(t.ready(), None);
    }

    #[test]
    fn failures_say_what_failed() {
        let mut t = enabled();
        t.start_check();
        t.check_failed("couldn't reach GitHub");
        assert_eq!(
            t.phase().summary(),
            "Couldn't check for updates: couldn't reach GitHub"
        );
        assert!(t.start_check(), "can try again");
        t.found("0.2.0");
        t.download_failed("connection reset");
        assert_eq!(
            t.phase().summary(),
            "Couldn't download v0.2.0: connection reset"
        );
    }

    #[test]
    fn install_runs_once_and_can_be_retried() {
        let mut t = enabled();
        t.start_check();
        t.found("0.2.0");
        t.downloaded();
        assert_eq!(t.start_install().as_deref(), Some("0.2.0"));
        assert_eq!(t.start_install(), None, "already installing");
        assert!(!t.start_check());
        t.install_failed();
        assert_eq!(t.phase(), &ready("0.2.0"));
        assert_eq!(t.start_install().as_deref(), Some("0.2.0"));
    }

    #[test]
    fn late_results_are_ignored() {
        let mut t = enabled();
        t.start_check();
        t.found("0.2.0");
        t.downloaded();
        t.start_check();
        // Installing while the check runs, then the check answers.
        assert_eq!(t.start_install().as_deref(), Some("0.2.0"));
        t.up_to_date();
        t.check_failed("late");
        assert!(!t.found("0.3.0"));
        assert!(matches!(t.phase(), UpdatePhase::Installing { .. }));
        // Download events outside a download do nothing either.
        t.downloaded();
        assert!(!t.progress(1, Some(2)));
        assert!(matches!(t.phase(), UpdatePhase::Installing { .. }));
    }

    #[test]
    fn percent_is_bounded() {
        assert_eq!(percent_of(0, Some(0)), None);
        assert_eq!(percent_of(5, Some(10)), Some(50));
        assert_eq!(percent_of(20, Some(10)), Some(100));
        assert_eq!(percent_of(u64::MAX, Some(1)), Some(100));
    }

    #[test]
    fn notices_after_a_check() {
        let notice = |r: CheckResult| check_notice(&r, "0.1.0");
        assert_eq!(
            notice(CheckResult::NotSetUp),
            "Updates aren't set up for this build."
        );
        assert_eq!(
            notice(CheckResult::Done(UpdatePhase::UpToDate)),
            "You're up to date (v0.1.0)."
        );
        assert_eq!(
            notice(CheckResult::Done(UpdatePhase::Downloading {
                version: "0.2.0".into(),
                percent: None
            })),
            "Found v0.2.0. Downloading it; you'll see when it's ready."
        );
        assert_eq!(
            notice(CheckResult::Done(ready("0.2.0"))),
            "v0.2.0 is ready. Pick “Install Update and Restart” to install it."
        );
        assert_eq!(
            notice(CheckResult::Busy(UpdatePhase::Checking)),
            "Checking for updates…"
        );
        assert_eq!(
            notice(CheckResult::Done(UpdatePhase::Failed {
                message: "Couldn't check for updates: offline".into()
            })),
            "Couldn't check for updates: offline."
        );
    }

    #[test]
    fn versions_compare_like_releases() {
        assert!(same_version("v1.2.0", "1.2.0"));
        assert!(!same_version("1.2.0", "1.2.1"));
        assert!(same_version("nightly", " nightly"));
        assert!(is_newer("0.10.0", "0.9.0"), "numeric, not text");
        assert!(is_newer("1.0.0", "1.0.0-beta.2"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("garbage", "1.0.0"));
    }

    #[test]
    fn after_an_install_restart() {
        assert_eq!(after_install(None, "0.2.0"), None);
        assert_eq!(
            after_install(Some("0.2.0"), "0.2.0"),
            Some(AfterInstall::Updated("0.2.0".into()))
        );
        assert_eq!(
            after_install(Some("0.2.0"), "0.1.0"),
            Some(AfterInstall::NotInstalled("0.2.0".into()))
        );
        assert_eq!(after_install(Some("0.2.0"), "0.3.0"), None);
    }
}
