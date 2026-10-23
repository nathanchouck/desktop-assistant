//! Keeps today's events and the inbox in memory while connected, so `cal`
//! and `mail` never wait for Google: every 5 minutes, at once when the
//! settings change, and on demand (the launcher opening, a search) when
//! what's kept is over a minute old.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use futures::{StreamExt, TryStreamExt};
use tokio::sync::{watch, Notify};
use tokio::time::Instant;

use super::api::{ApiError, Service};
use super::calendar::{self, Event};
use super::hub::Google;
use super::mail::{self, Inbox, Thread};
use super::GoogleSettings;
use crate::settings;

pub const REFRESH_EVERY: Duration = Duration::from_secs(5 * 60);
/// A search or the launcher opening asks for fresh data past this age.
pub const STALE_AFTER: Duration = Duration::from_secs(60);
/// Threads fetched at once (one request each).
const PARALLEL_THREADS: usize = 4;

/// The last answer for one kind of data, and how the last try went.
#[derive(Debug, Clone)]
pub struct Fetched<T> {
    /// The newest data that came in (kept when a later refresh fails).
    pub data: Option<T>,
    /// Why the last refresh failed, if it did.
    pub error: Option<ApiError>,
    pub tried: Option<Instant>,
}

impl<T> Default for Fetched<T> {
    fn default() -> Self {
        Self {
            data: None,
            error: None,
            tried: None,
        }
    }
}

impl<T> Fetched<T> {
    fn record(&mut self, result: Result<T, ApiError>) {
        match result {
            Ok(data) => {
                self.data = Some(data);
                self.error = None;
            }
            Err(e) => self.error = Some(e),
        }
        self.tried = Some(Instant::now());
    }
}

pub struct Cache {
    calendar: RwLock<Fetched<Vec<Event>>>,
    mail: RwLock<Fetched<Inbox>>,
    refreshing: AtomicBool,
    wake: Notify,
    /// Counts finished refreshes, so `refresh_and_wait` knows when one is done.
    done: watch::Sender<u64>,
}

impl Default for Cache {
    fn default() -> Self {
        Self {
            calendar: RwLock::default(),
            mail: RwLock::default(),
            refreshing: AtomicBool::new(false),
            wake: Notify::new(),
            done: watch::channel(0).0,
        }
    }
}

fn read<T: Clone>(lock: &RwLock<T>) -> T {
    match lock.read() {
        Ok(v) => v.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn update<T>(lock: &RwLock<T>, change: impl FnOnce(&mut T)) {
    match lock.write() {
        Ok(mut v) => change(&mut v),
        Err(poisoned) => change(&mut poisoned.into_inner()),
    }
}

impl Cache {
    pub fn calendar(&self) -> Fetched<Vec<Event>> {
        read(&self.calendar)
    }

    pub fn mail(&self) -> Fetched<Inbox> {
        read(&self.mail)
    }

    /// Asks for a refresh if the data is over `STALE_AFTER` old (or was
    /// never fetched) and none is running.
    pub fn refresh_if_stale(&self) {
        if self.refreshing.load(Ordering::Relaxed) {
            return;
        }
        let oldest = [self.calendar().tried, self.mail().tried]
            .into_iter()
            .min()
            .flatten();
        if oldest.is_none_or(|t| t.elapsed() >= STALE_AFTER) {
            self.wake.notify_one();
        }
    }

    /// Asks for a refresh now, whatever the age.
    pub fn refresh_now(&self) {
        self.wake.notify_one();
    }

    /// Refreshes now and waits for it (at most `limit`).
    pub async fn refresh_and_wait(&self, limit: Duration) {
        let mut done = self.done.subscribe();
        let before = *done.borrow_and_update();
        self.wake.notify_one();
        let _ = tokio::time::timeout(limit, done.wait_for(|n| *n != before)).await;
    }

    pub fn clear(&self) {
        update(&self.calendar, |c| *c = Fetched::default());
        update(&self.mail, |m| *m = Fetched::default());
    }

    #[cfg(test)]
    pub fn put_calendar(&self, events: Vec<Event>) {
        update(&self.calendar, |c| c.record(Ok(events)));
    }
}

/// Runs for the life of the app.
pub async fn run(google: Arc<Google>) {
    let mut changes = settings::changes();
    let mut seen: Option<GoogleSettings> = None;
    loop {
        if google.active() {
            seen = Some(google.settings());
            refresh(&google).await;
        }
        // Wait for the next round, a request, or a change to this
        // feature's settings (other sections don't need a refresh).
        let next = tokio::time::sleep(REFRESH_EVERY);
        tokio::pin!(next);
        loop {
            tokio::select! {
                _ = &mut next => break,
                _ = google.cache.wake.notified() => break,
                changed = changes.changed() => {
                    if changed.is_err() {
                        // The sender lives for the whole run; just in case.
                        tokio::time::sleep(REFRESH_EVERY).await;
                        break;
                    }
                    if seen.as_ref() != Some(&google.settings()) {
                        break;
                    }
                }
            }
        }
    }
}

/// Reads the calendar and the inbox (whichever you allowed), then
/// updates the status line.
async fn refresh(google: &Google) {
    let cache = &google.cache;
    cache.refreshing.store(true, Ordering::Relaxed);
    let granted = google.account.granted();
    let days = google.settings().calendar_days;
    let previous = cache.mail().data;
    let (events, inbox) = tokio::join!(
        async {
            if granted.calendar {
                fetch_calendar(google, days).await
            } else {
                Err(ApiError::NotAllowed(Service::Calendar))
            }
        },
        async {
            if granted.mail {
                fetch_inbox(google, previous.as_ref()).await
            } else {
                Err(ApiError::NotAllowed(Service::Gmail))
            }
        },
    );
    if let Err(e) = &events {
        tracing::warn!("could not read the calendar: {e}");
    }
    if let Err(e) = &inbox {
        tracing::warn!("could not read the inbox: {e}");
    }
    let problem = describe_problems(&[
        (Service::Calendar, events.as_ref().err()),
        (Service::Gmail, inbox.as_ref().err()),
    ]);
    update(&cache.calendar, |c| c.record(events));
    update(&cache.mail, |m| m.record(inbox));
    cache.refreshing.store(false, Ordering::Relaxed);
    cache.done.send_modify(|n| *n = n.wrapping_add(1));
    if google.account.set_problem(problem) {
        google.announce();
    }
}

/// One line for Settings: what failed, by service.
pub fn describe_problems(errors: &[(Service, Option<&ApiError>)]) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();
    for (service, error) in errors {
        let line = match error {
            None | Some(ApiError::NotConnected) => continue,
            // Already says what to do, and is the same for every service.
            Some(e @ ApiError::Reconnect(_)) => e.to_string(),
            Some(e @ ApiError::NotAllowed(_)) => e.to_string(),
            Some(e) => format!("{}: {e}", service.short_name()),
        };
        if !parts.contains(&line) {
            parts.push(line);
        }
    }
    (!parts.is_empty()).then(|| {
        let mut text = parts.join(". ");
        if let Some(first) = text.get(..1) {
            text = first.to_uppercase() + &text[1..];
        }
        text
    })
}

async fn fetch_calendar(google: &Google, days: u32) -> Result<Vec<Event>, ApiError> {
    let path = calendar::events_path(&chrono::Local::now(), days);
    let body = google
        .api
        .get(
            Service::Calendar,
            calendar::HOST,
            &path,
            crate::net::DEFAULT_TIMEOUT,
        )
        .await?;
    calendar::parse_events(&body).map_err(ApiError::Unreadable)
}

/// The unread count, the newest unread threads and the newest inbox
/// threads. Threads whose history id hasn't moved are reused, so a quiet
/// inbox costs three small requests.
async fn fetch_inbox(google: &Google, previous: Option<&Inbox>) -> Result<Inbox, ApiError> {
    let get = |path: String| async move {
        google
            .api
            .get(
                Service::Gmail,
                mail::HOST,
                &path,
                crate::net::DEFAULT_TIMEOUT,
            )
            .await
    };
    let unread_count = mail::parse_unread_count(&get(mail::inbox_label_path()).await?)
        .map_err(ApiError::Unreadable)?;
    let unread_ids =
        mail::parse_thread_list(&get(mail::threads_path(true, mail::UNREAD_SHOWN)).await?)
            .map_err(ApiError::Unreadable)?;
    let recent_ids =
        mail::parse_thread_list(&get(mail::threads_path(false, mail::RECENT_KEPT)).await?)
            .map_err(ApiError::Unreadable)?;

    let known: HashMap<&str, &Thread> = previous
        .map(|p| {
            p.unread
                .iter()
                .chain(&p.recent)
                .map(|t| (t.id.as_str(), t))
                .collect()
        })
        .unwrap_or_default();
    let mut wanted: Vec<(String, String)> = Vec::new();
    for (id, history) in unread_ids.iter().chain(&recent_ids) {
        if !wanted.iter().any(|(seen, _)| seen == id) {
            wanted.push((id.clone(), history.clone()));
        }
    }
    let fetched: Vec<Option<Thread>> = futures::stream::iter(wanted)
        .map(|(id, history)| {
            let reused = known
                .get(id.as_str())
                .filter(|t| !history.is_empty() && t.history_id == history)
                .map(|t| (*t).clone());
            async move {
                if let Some(thread) = reused {
                    return Ok(Some(thread));
                }
                match get(mail::thread_path(&id)).await {
                    // Deleted since it was listed.
                    Err(ApiError::Http { status: 404, .. }) => Ok(None),
                    Err(e) => Err(e),
                    Ok(body) => mail::parse_thread(&body)
                        .map(Some)
                        .map_err(ApiError::Unreadable),
                }
            }
        })
        .buffered(PARALLEL_THREADS)
        .try_collect()
        .await?;
    let threads: Vec<Thread> = fetched.into_iter().flatten().collect();
    let by_id = |ids: &[(String, String)]| -> Vec<Thread> {
        ids.iter()
            .filter_map(|(id, _)| threads.iter().find(|t| &t.id == id).cloned())
            .collect()
    };
    Ok(Inbox {
        unread_count,
        unread: by_id(&unread_ids),
        recent: by_id(&recent_ids),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn problems_read_as_one_line() {
        assert_eq!(
            describe_problems(&[(Service::Calendar, None), (Service::Gmail, None)]),
            None
        );
        assert_eq!(
            describe_problems(&[(Service::Calendar, Some(&ApiError::NotConnected))]),
            None
        );
        let off = ApiError::Disabled {
            service: Service::Gmail,
            link: None,
        };
        let offline = ApiError::Net(crate::net::NetError::Failed("no internet".into()));
        assert_eq!(
            describe_problems(&[(Service::Calendar, Some(&offline)), (Service::Gmail, Some(&off))])
                .as_deref(),
            Some("Calendar: no internet. Gmail: the Gmail API is turned off in your Google Cloud project")
        );
        let dead = ApiError::Reconnect("the sign-in expired".into());
        assert_eq!(
            describe_problems(&[
                (Service::Calendar, Some(&dead)),
                (Service::Gmail, Some(&dead))
            ])
            .as_deref(),
            Some("The sign-in expired")
        );
    }

    #[tokio::test]
    async fn staleness_and_waiting() {
        let cache = Cache::default();
        // Nothing fetched yet: a refresh is asked for.
        cache.refresh_if_stale();
        tokio::time::timeout(Duration::from_millis(100), cache.wake.notified())
            .await
            .expect("woken");
        update(&cache.calendar, |c| c.record(Ok(Vec::new())));
        update(&cache.mail, |m| m.record(Ok(Inbox::default())));
        cache.refresh_if_stale();
        assert!(
            tokio::time::timeout(Duration::from_millis(50), cache.wake.notified())
                .await
                .is_err(),
            "fresh data isn't fetched again"
        );
        assert!(cache.calendar().data.is_some());
        // A failed refresh keeps the data and notes the error.
        update(&cache.mail, |m| m.record(Err(ApiError::NotConnected)));
        let mail = cache.mail();
        assert!(mail.data.is_some() && mail.error.is_some());
        cache.clear();
        assert!(cache.mail().data.is_none());
        // Waiting ends when a refresh finishes, or at the limit.
        let started = Instant::now();
        cache.refresh_and_wait(Duration::from_millis(30)).await;
        assert!(started.elapsed() >= Duration::from_millis(30));
    }
}
