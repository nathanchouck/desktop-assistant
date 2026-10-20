//! Shell COM objects must be used from a single-threaded apartment (STA).
//! `StaWorker` is one long-lived STA thread that runs jobs in order;
//! `run_once` spins up a short-lived STA thread for a single job.

use std::panic::{catch_unwind, AssertUnwindSafe};
use std::sync::mpsc;

use tokio::sync::oneshot;
use windows::Win32::System::Com::{
    CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED, COINIT_DISABLE_OLE1DDE,
};

use crate::platform::PlatformError;

type Job = Box<dyn FnOnce() + Send>;

pub struct StaWorker {
    tx: mpsc::Sender<Job>,
}

impl StaWorker {
    pub fn spawn(name: &str) -> Self {
        let (tx, rx) = mpsc::channel::<Job>();
        let spawned = std::thread::Builder::new()
            .name(name.into())
            .spawn(move || {
                let _com = ComApartment::enter();
                for job in rx {
                    // Keep the worker alive if one job panics (debug builds).
                    let _ = catch_unwind(AssertUnwindSafe(job));
                }
            });
        if let Err(e) = spawned {
            tracing::error!("could not start {name}: {e}");
        }
        Self { tx }
    }

    /// Runs `f` on the worker thread and waits for its result.
    pub async fn run<T: Send + 'static>(
        &self,
        f: impl FnOnce() -> T + Send + 'static,
    ) -> Result<T, PlatformError> {
        let (done, result) = oneshot::channel();
        self.tx
            .send(Box::new(move || {
                let _ = done.send(f());
            }))
            .map_err(|_| PlatformError::WorkerGone)?;
        result.await.map_err(|_| PlatformError::WorkerGone)
    }
}

/// Runs `f` on a fresh STA thread and waits for its result.
pub async fn run_once<T: Send + 'static>(
    f: impl FnOnce() -> T + Send + 'static,
) -> Result<T, PlatformError> {
    let (done, result) = oneshot::channel();
    std::thread::Builder::new()
        .name("shell-once".into())
        .spawn(move || {
            let _com = ComApartment::enter();
            let _ = done.send(f());
        })
        .map_err(|e| PlatformError::Os(e.to_string()))?;
    result.await.map_err(|_| PlatformError::WorkerGone)
}

/// COM initialized for this thread until dropped.
struct ComApartment {
    initialized: bool,
}

impl ComApartment {
    fn enter() -> Self {
        // SAFETY: called once at the start of a thread we own.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        if hr.is_err() {
            tracing::warn!("COM init failed: {hr:?}");
        }
        Self {
            initialized: hr.is_ok(),
        }
    }
}

impl Drop for ComApartment {
    fn drop(&mut self) {
        if self.initialized {
            // SAFETY: balances the successful CoInitializeEx on this thread.
            unsafe { CoUninitialize() };
        }
    }
}
