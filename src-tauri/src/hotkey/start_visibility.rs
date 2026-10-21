//! Windows only: `IAppVisibility`, which tells us when Start opens or
//! closes (documented, Windows 8 and later; other shell tools use it for
//! "Start opened"). It covers Start only, not Windows Search. Uses the
//! `windows` crate because the event sink is a COM object we implement.

use windows::core::{implement, Result, BOOL};
use windows::Win32::Graphics::Gdi::HMONITOR;
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL, COINIT_APARTMENTTHREADED,
    COINIT_DISABLE_OLE1DDE,
};
use windows::Win32::UI::Shell::{
    AppVisibility, IAppVisibility, IAppVisibilityEvents, IAppVisibilityEvents_Impl,
    MONITOR_APP_VISIBILITY,
};
use windows_sys::Win32::UI::WindowsAndMessaging::PostThreadMessageW;

/// COM (single-threaded apartment) on this thread until dropped.
pub struct Apartment;

impl Apartment {
    pub fn enter() -> Option<Self> {
        // SAFETY: once, at the start of a thread we own.
        let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED | COINIT_DISABLE_OLE1DDE) };
        hr.is_ok().then_some(Apartment)
    }
}

impl Drop for Apartment {
    fn drop(&mut self) {
        // SAFETY: balances the successful CoInitializeEx above.
        unsafe { CoUninitialize() };
    }
}

#[implement(IAppVisibilityEvents)]
struct Sink {
    thread_id: u32,
    message: u32,
}

impl IAppVisibilityEvents_Impl for Sink_Impl {
    fn AppVisibilityOnMonitorChanged(
        &self,
        _monitor: HMONITOR,
        _previous: MONITOR_APP_VISIBILITY,
        _current: MONITOR_APP_VISIBILITY,
    ) -> Result<()> {
        Ok(())
    }

    fn LauncherVisibilityChange(&self, visible: BOOL) -> Result<()> {
        // May be called on any thread: only post to the guard thread.
        // SAFETY: posting to a thread id has no preconditions; it fails
        // harmlessly if the thread is gone.
        unsafe {
            PostThreadMessageW(
                self.thread_id,
                self.message,
                usize::from(visible.as_bool()),
                0,
            )
        };
        Ok(())
    }
}

/// While this lives, Start opening or closing posts `message` to the
/// thread (wParam 1: now visible). Create and drop it on a thread inside
/// an `Apartment`.
pub struct Watch {
    visibility: IAppVisibility,
    cookie: u32,
}

impl Watch {
    pub fn start(thread_id: u32, message: u32) -> Result<Self> {
        // SAFETY: COM is initialized on this thread (the caller's contract).
        let visibility: IAppVisibility =
            unsafe { CoCreateInstance(&AppVisibility, None, CLSCTX_ALL)? };
        let sink: IAppVisibilityEvents = Sink { thread_id, message }.into();
        // SAFETY: `sink` is a live COM object; Advise keeps a reference.
        let cookie = unsafe { visibility.Advise(&sink)? };
        Ok(Self { visibility, cookie })
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        // SAFETY: the cookie came from Advise on this object.
        let _ = unsafe { self.visibility.Unadvise(self.cookie) };
    }
}
