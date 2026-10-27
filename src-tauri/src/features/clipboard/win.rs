//! Windows: hearing copies and reading the clipboard.
//!
//! A message-only window on its own thread registers with
//! `AddClipboardFormatListener`, so Windows posts it `WM_CLIPBOARDUPDATE`
//! after every change. The listener checks the copy may be kept, copies
//! the raw bytes out and closes the clipboard right away (apps that copy
//! twice quickly fail if it's held open), then hands them to a recorder
//! thread that decodes and stores them.
//!
//! A copy is skipped when its app asks (the formats password managers set:
//! `ExcludeClipboardContentFromMonitorProcessing`, `Clipboard Viewer
//! Ignore`, `CanIncludeInClipboardHistory` = 0), when it was made in an app
//! on the ignore list, and when it's our own write.

use std::cell::RefCell;
use std::sync::{mpsc, Arc, OnceLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use windows::core::{w, Error, Result, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, GlobalFree, HANDLE, HGLOBAL, HINSTANCE, HWND, LPARAM, LRESULT, WPARAM,
};
use windows::Win32::System::DataExchange::{
    AddClipboardFormatListener, CloseClipboard, EmptyClipboard, EnumClipboardFormats,
    GetClipboardData, GetClipboardOwner, GetClipboardSequenceNumber, IsClipboardFormatAvailable,
    OpenClipboard, RegisterClipboardFormatW, SetClipboardData,
};
use windows::Win32::System::Memory::{
    GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE,
};
use windows::Win32::System::Ole::{CF_DIB, CF_DIBV5, CF_HDROP, CF_UNICODETEXT};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetForegroundWindow, GetMessageW,
    GetWindowThreadProcessId, RegisterClassW, HWND_MESSAGE, MSG, WINDOW_EX_STYLE, WINDOW_STYLE,
    WM_CLIPBOARDUPDATE, WNDCLASSW,
};

use super::content::{self, Captured, MAX_TEXT_UNITS};
use super::history::History;
use super::recorder::{self, Capture};
use super::system;
use crate::settings::{self, SharedSettings};

/// Formats bigger than this aren't read (a huge bitmap, say).
const MAX_BYTES: usize = 256 * 1024 * 1024;
/// Another app may have the clipboard open for a moment.
const OPEN_ATTEMPTS: u32 = 5;
const OPEN_RETRY: Duration = Duration::from_millis(20);

/// Clipboard formats that have no fixed number; Windows assigns one per name.
struct Formats {
    /// Set by password managers: don't look at this at all.
    exclude: u32,
    /// The older way to say the same.
    viewer_ignore: u32,
    /// A DWORD; 0 means "don't keep this in a clipboard history".
    history: u32,
    /// PNG file bytes (browsers, Snipping Tool).
    png: u32,
}

fn formats() -> &'static Formats {
    static FORMATS: OnceLock<Formats> = OnceLock::new();
    FORMATS.get_or_init(|| {
        // SAFETY: registering a name has no preconditions; 0 means it failed
        // and those checks are then skipped.
        unsafe {
            Formats {
                exclude: RegisterClipboardFormatW(w!(
                    "ExcludeClipboardContentFromMonitorProcessing"
                )),
                viewer_ignore: RegisterClipboardFormatW(w!("Clipboard Viewer Ignore")),
                history: RegisterClipboardFormatW(w!("CanIncludeInClipboardHistory")),
                png: RegisterClipboardFormatW(w!("PNG")),
            }
        }
    })
}

pub fn sequence_number() -> u32 {
    // SAFETY: no preconditions.
    unsafe { GetClipboardSequenceNumber() }
}

/// Starts the listener and recorder threads.
pub fn start(history: Arc<History>, settings: SharedSettings) {
    let (captures, inbox) = mpsc::channel::<Capture>();
    let recorder_settings = Arc::clone(&settings);
    let recorder = std::thread::Builder::new()
        .name("clipboard-recorder".into())
        .spawn(move || {
            for capture in inbox {
                let options = settings::current(&recorder_settings).clipboard;
                // Errors never carry the copied content, so they're safe to log.
                if let Err(e) = recorder::record(&history, capture, &options, now_ms()) {
                    crate::toast::report(format!("Clipboard history couldn't keep a copy: {e}"));
                }
            }
        });
    if let Err(e) = recorder {
        crate::toast::report(format!("Clipboard history is off: {e}"));
        return;
    }
    let listener = Listener { captures, settings };
    if let Err(e) = std::thread::Builder::new()
        .name("clipboard-listener".into())
        .spawn(move || run(listener))
    {
        crate::toast::report(format!("Clipboard history is off: {e}"));
    }
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as i64)
}

struct Listener {
    captures: mpsc::Sender<Capture>,
    settings: SharedSettings,
}

thread_local! {
    static LISTENER: RefCell<Option<Listener>> = const { RefCell::new(None) };
}

fn run(listener: Listener) {
    LISTENER.with(|l| *l.borrow_mut() = Some(listener));
    match create_window() {
        Ok(_) => tracing::info!("clipboard history is listening"),
        Err(e) => {
            crate::toast::report(format!("Clipboard history is off: {e}"));
            return;
        }
    }
    let mut msg = MSG::default();
    // SAFETY: a standard message loop on the thread that owns the window.
    unsafe {
        while GetMessageW(&mut msg, None, 0, 0).0 > 0 {
            DispatchMessageW(&msg);
        }
    }
}

fn create_window() -> Result<HWND> {
    // SAFETY: plain Win32 window setup on this thread; the class name and
    // window procedure are 'static.
    unsafe {
        let module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null());
        let instance = HINSTANCE(module);
        let class = WNDCLASSW {
            lpfnWndProc: Some(window_proc),
            hInstance: instance,
            lpszClassName: w!("DesktopAssistantClipboard"),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(Error::from_thread());
        }
        // A message-only window: never shown, only receives messages.
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("DesktopAssistantClipboard"),
            w!("Desktop Assistant clipboard"),
            WINDOW_STYLE(0),
            0,
            0,
            0,
            0,
            Some(HWND_MESSAGE),
            None,
            Some(instance),
            None,
        )?;
        AddClipboardFormatListener(hwnd)?;
        Ok(hwnd)
    }
}

unsafe extern "system" fn window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    if msg == WM_CLIPBOARDUPDATE {
        LISTENER.with(|l| {
            if let Ok(listener) = l.try_borrow() {
                if let Some(listener) = listener.as_ref() {
                    listener.on_change(hwnd);
                }
            }
        });
        return LRESULT(0);
    }
    // SAFETY: everything else gets the default handling.
    unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
}

impl Listener {
    fn on_change(&self, hwnd: HWND) {
        if system::is_own_change(sequence_number) {
            return;
        }
        let options = settings::current(&self.settings).clipboard;
        if !options.enabled {
            return;
        }
        if marked_private() {
            tracing::debug!("skipped a copy marked private");
            return;
        }
        let source = source_program();
        if content::is_ignored(&source, &options.ignore_apps) {
            tracing::debug!("skipped a copy from {source} (ignore list)");
            return;
        }
        match read(Some(hwnd), options.keep_images, true) {
            Ok(Some(content)) => {
                let _ = self.captures.send(Capture { content, source });
            }
            Ok(None) => {}
            Err(e) => tracing::debug!("could not read the clipboard: {e}"),
        }
    }
}

/// Whether the app that copied asked clipboard managers to look away.
/// Checking doesn't need the clipboard open.
fn marked_private() -> bool {
    let f = formats();
    [f.exclude, f.viewer_ignore]
        .into_iter()
        // SAFETY: no preconditions.
        .any(|format| format != 0 && unsafe { IsClipboardFormatAvailable(format) }.is_ok())
}

/// The program that made the copy: the clipboard's owner window, else the
/// window in front (many apps copy without an owner).
fn source_program() -> String {
    // SAFETY: read-only queries; a window that just closed only makes them fail.
    unsafe {
        let window = GetClipboardOwner().ok().or_else(|| {
            let front = GetForegroundWindow();
            (!front.is_invalid()).then_some(front)
        });
        let Some(window) = window else {
            return String::new();
        };
        let mut pid = 0u32;
        GetWindowThreadProcessId(window, Some(&mut pid));
        if pid == std::process::id() {
            return "Desktop Assistant".into();
        }
        program_path(pid)
            .map(|path| content::program_name(&path))
            .unwrap_or_default()
    }
}

fn program_path(pid: u32) -> Option<String> {
    // SAFETY: the handle is closed before returning; the buffer outlives the call.
    unsafe {
        let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid).ok()?;
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let named = QueryFullProcessImageNameW(
            process,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        );
        let _ = CloseHandle(process);
        named.ok()?;
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

/// The clipboard, open until dropped.
struct OpenClipboardGuard;

impl OpenClipboardGuard {
    fn open(owner: Option<HWND>) -> Result<Self> {
        let mut attempt = 1;
        loop {
            // SAFETY: closed again when the guard drops.
            match unsafe { OpenClipboard(owner) } {
                Ok(()) => return Ok(Self),
                Err(e) if attempt >= OPEN_ATTEMPTS => return Err(e),
                Err(_) => {
                    attempt += 1;
                    std::thread::sleep(OPEN_RETRY);
                }
            }
        }
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        // SAFETY: we opened it.
        let _ = unsafe { CloseClipboard() };
    }
}

/// What's on the clipboard now, private or not: snippets borrow the
/// clipboard to paste and put this back right after.
pub fn read_now() -> Option<Captured> {
    read(None, true, false).ok().flatten()
}

/// Reads the copy: files, else text, else an image (if `images`). Only
/// raw bytes are copied while the clipboard is open. With `for_history`,
/// a copy its app keeps out of clipboard histories is skipped.
fn read(owner: Option<HWND>, images: bool, for_history: bool) -> Result<Option<Captured>> {
    let f = formats();
    let _open = OpenClipboardGuard::open(owner)?;
    if for_history && f.history != 0 {
        // SAFETY: the clipboard is open.
        if let Some(flag) = unsafe { format_bytes(f.history) } {
            if flag.get(..4) == Some(&[0, 0, 0, 0]) {
                return Ok(None);
            }
        }
    }
    // SAFETY (each `format_bytes`): the clipboard is open.
    if let Some(bytes) = unsafe { format_bytes(u32::from(CF_HDROP.0)) } {
        let files = content::files_from_dropfiles(&bytes);
        if !files.is_empty() {
            return Ok(Some(Captured::Files(files)));
        }
    }
    if let Some(bytes) = unsafe { format_bytes(u32::from(CF_UNICODETEXT.0)) } {
        if bytes.len() / 2 > MAX_TEXT_UNITS {
            return Ok(None);
        }
        let text = content::text_from_utf16(&bytes);
        if !text.trim().is_empty() {
            return Ok(Some(Captured::Text(text)));
        }
    }
    if !images {
        return Ok(None);
    }
    if f.png != 0 {
        if let Some(bytes) = unsafe { format_bytes(f.png) } {
            return Ok(Some(Captured::Png(bytes)));
        }
    }
    // The V5 bitmap first: it keeps transparency. Windows makes either
    // from the other.
    for format in [CF_DIBV5, CF_DIB] {
        if let Some(bytes) = unsafe { format_bytes(u32::from(format.0)) } {
            return Ok(Some(Captured::Dib(bytes)));
        }
    }
    Ok(None)
}

/// Everything on the clipboard, format by format, so it can be put back
/// exactly (an Excel range keeps its cells, Word its formatting). Formats
/// that are GDI handles rather than memory blocks are skipped; Windows
/// makes them again from the bitmap and text formats.
pub struct Snapshot {
    formats: Vec<(u32, Vec<u8>)>,
}

impl Snapshot {
    /// The text that was on the clipboard, if any.
    pub fn text(&self) -> Option<String> {
        let unicode = u32::from(CF_UNICODETEXT.0);
        self.formats
            .iter()
            .find(|(format, _)| *format == unicode)
            .map(|(_, bytes)| content::text_from_utf16(bytes))
    }
}

/// All of a snapshot together may not be bigger than this.
const MAX_SNAPSHOT_BYTES: usize = 64 * 1024 * 1024;

/// Takes a snapshot of the clipboard; `None` if it couldn't be opened.
/// May wait for the app that copied to render its formats.
pub fn snapshot() -> Option<Snapshot> {
    let _open = OpenClipboardGuard::open(None).ok()?;
    let mut formats = Vec::new();
    let mut total = 0usize;
    let mut format = 0;
    loop {
        // SAFETY: the clipboard is open.
        format = unsafe { EnumClipboardFormats(format) };
        if format == 0 {
            break;
        }
        if !content::is_memory_format(format) {
            continue;
        }
        // SAFETY: the clipboard is open and `format` holds a memory block.
        if let Some(bytes) = unsafe { format_bytes(format) } {
            total += bytes.len();
            if total > MAX_SNAPSHOT_BYTES {
                break;
            }
            formats.push((format, bytes));
        }
    }
    Some(Snapshot { formats })
}

/// Puts a snapshot back (an empty one empties the clipboard), kept out of
/// clipboard histories: it was there already. Returns the change's number.
pub fn restore(snapshot: &Snapshot) -> Result<u32> {
    let f = formats();
    let _open = OpenClipboardGuard::open(None)?;
    // SAFETY: the clipboard is open; each block is handed to the clipboard
    // on success and freed on failure.
    unsafe {
        EmptyClipboard()?;
        for (format, bytes) in &snapshot.formats {
            if *format == f.exclude || *format == f.history {
                continue;
            }
            set_bytes(*format, bytes);
        }
        if f.exclude != 0 {
            set_bytes(f.exclude, &[0, 0, 0, 0]);
        }
        if f.history != 0 {
            set_bytes(f.history, &[0, 0, 0, 0]);
        }
    }
    Ok(sequence_number())
}

/// Copies `bytes` into a new global block and hands it to the clipboard.
///
/// # Safety
/// The clipboard must be open and emptied by us.
unsafe fn set_bytes(format: u32, bytes: &[u8]) {
    // SAFETY: per the contract above.
    unsafe {
        let Ok(global) = GlobalAlloc(GMEM_MOVEABLE, bytes.len().max(1)) else {
            return;
        };
        let data = GlobalLock(global) as *mut u8;
        if data.is_null() {
            let _ = GlobalFree(Some(global));
            return;
        }
        std::ptr::copy_nonoverlapping(bytes.as_ptr(), data, bytes.len());
        let _ = GlobalUnlock(global);
        if SetClipboardData(format, Some(HANDLE(global.0))).is_err() {
            let _ = GlobalFree(Some(global));
        }
    }
}

/// A copy of one format's bytes, if the clipboard has it.
///
/// # Safety
/// The clipboard must be open, and `format` must be one whose data is a
/// global memory block (text, file lists, bitmaps, registered formats).
unsafe fn format_bytes(format: u32) -> Option<Vec<u8>> {
    // SAFETY: per the contract above; the lock is released before returning.
    unsafe {
        let handle = GetClipboardData(format).ok()?;
        let global = HGLOBAL(handle.0);
        let size = GlobalSize(global);
        if size == 0 || size > MAX_BYTES {
            return None;
        }
        let data = GlobalLock(global) as *const u8;
        if data.is_null() {
            return None;
        }
        let bytes = std::slice::from_raw_parts(data, size).to_vec();
        let _ = GlobalUnlock(global);
        Some(bytes)
    }
}
