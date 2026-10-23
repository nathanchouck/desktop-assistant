//! Processes started ahead of a question (`warm`), so the first answer
//! comes about a second sooner. There's one per setup asked for: one
//! normally, two while a comparison is being typed. The next conversation
//! with the same setup claims its spare; unclaimed, it's closed after a
//! while.

use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use super::session::{Session, Setup};
use super::Chat;
use crate::ai::AiError;

pub(super) struct Spare {
    pub(super) session: Session,
    pub(super) resume: Arc<StdMutex<Option<String>>>,
    /// A question with another setup (provider, folder, model...) can't
    /// use it.
    setup: Setup,
    born: Instant,
}

impl Spare {
    fn usable_for(&self, setup: &Setup) -> bool {
        self.setup == *setup && self.session.process().is_alive()
    }
}

impl Chat {
    /// Starts a spare process for the next question, unless a matching one
    /// is already waiting.
    pub async fn warm(&self, setup: Setup) -> Result<(), AiError> {
        self.warm_all(vec![setup]).await
    }

    /// Makes sure a spare is waiting for each of `setups` (one per AI when
    /// comparing). Spares for any other setup are closed: the user's setup
    /// has moved on. Returns the first error, after trying them all.
    pub async fn warm_all(&self, setups: Vec<Setup>) -> Result<(), AiError> {
        let mut spares = self.spare.lock().await;
        let (keep, old): (Vec<Spare>, Vec<Spare>) = std::mem::take(&mut *spares)
            .into_iter()
            .partition(|spare| setups.iter().any(|setup| spare.usable_for(setup)));
        for spare in old {
            spare.session.end().await;
        }
        *spares = keep;
        let mut failed = None;
        for setup in setups {
            if spares.iter().any(|spare| spare.setup == setup) {
                continue;
            }
            let resume = Arc::new(StdMutex::new(None));
            match Session::spawn(&setup, 0, self.sink.clone(), Arc::clone(&resume)) {
                Ok(session) => spares.push(Spare {
                    session,
                    resume,
                    setup,
                    born: Instant::now(),
                }),
                Err(e) => failed = failed.or(Some(e)),
            }
        }
        failed.map_or(Ok(()), Err)
    }

    /// The spare for each of `setups` (alive and started with it), in
    /// order. Spares nobody takes are closed: the user's setup has moved on.
    pub(super) async fn take_spares(&self, setups: &[&Setup]) -> Vec<Option<Spare>> {
        let mut left = std::mem::take(&mut *self.spare.lock().await);
        let taken = setups
            .iter()
            .map(|setup| {
                let at = left.iter().position(|spare| spare.usable_for(setup))?;
                Some(left.swap_remove(at))
            })
            .collect();
        for unused in left {
            unused.session.end_soon();
        }
        taken
    }

    /// The spare, if it's alive and was started with `setup`.
    pub(super) async fn take_spare(&self, setup: &Setup) -> Option<Spare> {
        self.take_spares(&[setup]).await.pop().flatten()
    }

    /// Closes spares nobody claimed within `max_age`.
    pub async fn drop_stale_spare(&self, max_age: Duration) {
        let mut spares = self.spare.lock().await;
        let (old, young): (Vec<Spare>, Vec<Spare>) = std::mem::take(&mut *spares)
            .into_iter()
            .partition(|spare| spare.born.elapsed() > max_age);
        *spares = young;
        for spare in old {
            tracing::info!(
                "closing the unused warm {} process",
                spare.session.process().name
            );
            spare.session.end().await;
        }
    }

    /// Ends every spare (app exit).
    pub(super) async fn end_spares(&self) {
        let spares = std::mem::take(&mut *self.spare.lock().await);
        for spare in spares {
            spare.session.end().await;
        }
    }
}
