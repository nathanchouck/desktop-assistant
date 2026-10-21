//! The last install / sign-in check of an AI CLI, kept so searching never
//! waits for one. Re-checked often while something needs fixing, rarely
//! while all is well.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use super::claude::ClaudeStatus;

/// A provider's status type: which value means "can answer", and which
/// one stands for "not checked yet".
pub trait Readiness: Copy {
    const UNKNOWN: Self;
    fn is_ready(self) -> bool;
}

/// Re-check this often when everything was fine...
const RECHECK_READY: Duration = Duration::from_secs(30 * 60);
/// ...and this often while something needs fixing, so a fix shows up soon.
const RECHECK_BROKEN: Duration = Duration::from_secs(15);

pub struct StatusCache<S: Readiness = ClaudeStatus> {
    last: Mutex<Option<(S, Instant)>>,
    checking: AtomicBool,
}

impl<S: Readiness> Default for StatusCache<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: Readiness> StatusCache<S> {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(None),
            checking: AtomicBool::new(false),
        }
    }

    /// Claims the right to run a check; `false` if one is already running.
    pub fn begin_check(&self) -> bool {
        !self.checking.swap(true, Ordering::AcqRel)
    }

    /// Stores a check's result and releases the claim.
    pub fn finish_check(&self, status: S) {
        self.set(status);
        self.checking.store(false, Ordering::Release);
    }

    /// The last result; unknown before the first check finishes.
    pub fn get(&self) -> S {
        self.last
            .lock()
            .ok()
            .and_then(|last| last.map(|(status, _)| status))
            .unwrap_or(S::UNKNOWN)
    }

    pub fn set(&self, status: S) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some((status, Instant::now()));
        }
    }

    /// Whether it's time to check again.
    pub fn is_stale(&self) -> bool {
        let Ok(last) = self.last.lock() else {
            return true;
        };
        match *last {
            None => true,
            Some((status, at)) if status.is_ready() => at.elapsed() > RECHECK_READY,
            Some((_, at)) => at.elapsed() > RECHECK_BROKEN,
        }
    }
}
