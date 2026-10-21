//! When to look for updates without being asked: a minute after the app
//! starts (not at once, so starting stays quick), then every 6 hours, and
//! 30 minutes after a check that failed (offline, GitHub down).
//!
//! Times are wall-clock seconds rather than a sleeping timer, so a laptop
//! that slept through a check catches up soon after it wakes.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

pub const FIRST_CHECK: Duration = Duration::from_secs(60);
pub const EVERY: Duration = Duration::from_secs(6 * 60 * 60);
pub const RETRY: Duration = Duration::from_secs(30 * 60);
/// The background task wakes at least this often to look at the clock and
/// the settings again.
pub const TICK: Duration = Duration::from_secs(5 * 60);

/// The last check this run: when it ended and whether it failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Attempt {
    /// Seconds since 1970.
    pub at: u64,
    pub failed: bool,
}

/// How long until the next automatic check is due (zero: now). `started`
/// is when the app started; all times are seconds since 1970.
pub fn wait(now: u64, started: u64, last: Option<Attempt>) -> Duration {
    let (from, gap) = match last {
        None => (started, FIRST_CHECK),
        Some(a) if a.failed => (a.at, RETRY),
        Some(a) => (a.at, EVERY),
    };
    // A clock set back makes `now` earlier than `from`: then wait one gap.
    let elapsed = now.saturating_sub(from);
    Duration::from_secs(gap.as_secs().saturating_sub(elapsed))
}

/// Seconds since 1970 (0 if the clock is before that).
pub fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

#[cfg(test)]
mod tests {
    use super::*;

    const T0: u64 = 1_800_000_000;

    #[test]
    fn first_check_a_minute_after_start() {
        assert_eq!(wait(T0, T0, None), Duration::from_secs(60));
        assert_eq!(wait(T0 + 45, T0, None), Duration::from_secs(15));
        assert_eq!(wait(T0 + 600, T0, None), Duration::ZERO);
    }

    #[test]
    fn then_every_six_hours() {
        let ok = Some(Attempt { at: T0, failed: false });
        assert_eq!(wait(T0, T0 - 100, ok), EVERY);
        assert_eq!(wait(T0 + 3600, T0, ok), Duration::from_secs(5 * 3600));
        assert_eq!(wait(T0 + 6 * 3600, T0, ok), Duration::ZERO);
    }

    #[test]
    fn sooner_after_a_failure() {
        let failed = Some(Attempt { at: T0, failed: true });
        assert_eq!(wait(T0 + 60, T0, failed), Duration::from_secs(29 * 60));
        assert_eq!(wait(T0 + 31 * 60, T0, failed), Duration::ZERO);
    }

    #[test]
    fn a_long_sleep_makes_it_due_and_a_clock_set_back_waits_one_gap() {
        let ok = Some(Attempt { at: T0, failed: false });
        assert_eq!(wait(T0 + 3 * 24 * 3600, T0, ok), Duration::ZERO);
        assert_eq!(wait(T0 - 3600, T0, ok), EVERY);
    }
}
