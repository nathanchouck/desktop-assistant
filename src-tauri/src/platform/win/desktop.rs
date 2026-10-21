//! What was on screen when the launcher opened: the window you were in and,
//! for File Explorer or the desktop, the folder shown and what's selected.

use std::path::Path;

use windows::core::{Interface, Result, PWSTR};
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IDispatch, IServiceProvider, CLSCTX_ALL,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
};
use windows::Win32::System::Variant::{VARIANT, VT_I4};
use windows::Win32::UI::Shell::{
    IFolderView, IPersistFolder2, IShellBrowser, IShellItem, IShellItemArray, IShellWindows,
    SHCreateItemFromIDList, SID_STopLevelBrowser, ShellWindows, SIGDN_FILESYSPATH, SVGIO_SELECTION,
    SWC_DESKTOP, SWFO_NEEDDISPATCH,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowTextW, GetWindowThreadProcessId,
    IsWindowVisible, GA_ROOT,
};

use crate::platform::{is_shell_view, ExplorerView, ForegroundWindow};

/// More selected items than this are cut off (the count is kept).
const MAX_SELECTION: usize = 50;

/// The window in front, unless it's ours. Cheap: no COM, safe to call while
/// the launcher is about to show.
pub fn foreground_window() -> Option<ForegroundWindow> {
    // SAFETY: read-only queries on a window handle; a window that has just
    // closed makes them fail or return empty text, nothing worse.
    unsafe {
        let hwnd = GetForegroundWindow();
        if hwnd.is_invalid() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        if pid == std::process::id() {
            return None;
        }
        Some(ForegroundWindow {
            handle: hwnd.0 as isize,
            title: window_text(hwnd),
            class: class_name(hwnd),
            app: process_name(pid).unwrap_or_default(),
        })
    }
}

/// The folder and selection of `window` if it's File Explorer or the
/// desktop. Must run on an STA thread.
pub fn explorer_view(window: &ForegroundWindow) -> Result<Option<ExplorerView>> {
    if !is_shell_view(&window.class) {
        return Ok(None);
    }
    // SAFETY: COM calls on an STA thread (the caller's contract).
    let shell_windows: IShellWindows =
        unsafe { CoCreateInstance(&ShellWindows, None, CLSCTX_ALL)? };
    let browser = if matches!(window.class.as_str(), "Progman" | "WorkerW") {
        desktop_browser(&shell_windows)?
    } else {
        explorer_browser(&shell_windows, HWND(window.handle as *mut _))?
    };
    browser.map(|b| view_of(&b)).transpose()
}

/// The browser behind the Explorer window `target`. With Windows 11 tabs
/// each tab is its own shell window; only the active tab is visible.
fn explorer_browser(shell_windows: &IShellWindows, target: HWND) -> Result<Option<IShellBrowser>> {
    // SAFETY: COM calls on an STA thread; handles are only compared.
    unsafe {
        let mut hidden_tab = None;
        for i in 0..shell_windows.Count()? {
            let Ok(dispatch) = shell_windows.Item(&variant_i32(i)) else {
                continue;
            };
            let Ok(browser) = top_level_browser(&dispatch) else {
                continue;
            };
            let Ok(hwnd) = browser.GetWindow() else {
                continue;
            };
            if GetAncestor(hwnd, GA_ROOT) != target {
                continue;
            }
            if IsWindowVisible(hwnd).as_bool() {
                return Ok(Some(browser));
            }
            hidden_tab.get_or_insert(browser);
        }
        Ok(hidden_tab)
    }
}

fn desktop_browser(shell_windows: &IShellWindows) -> Result<Option<IShellBrowser>> {
    const CSIDL_DESKTOP: i32 = 0;
    let location = variant_i32(CSIDL_DESKTOP);
    let root = VARIANT::default();
    let mut hwnd = 0i32;
    // SAFETY: COM call on an STA thread with valid VARIANTs.
    let dispatch = unsafe {
        shell_windows.FindWindowSW(&location, &root, SWC_DESKTOP, &mut hwnd, SWFO_NEEDDISPATCH)?
    };
    top_level_browser(&dispatch).map(Some)
}

fn top_level_browser(dispatch: &IDispatch) -> Result<IShellBrowser> {
    let provider: IServiceProvider = dispatch.cast()?;
    // SAFETY: COM call on an STA thread.
    unsafe { provider.QueryService::<IShellBrowser>(&SID_STopLevelBrowser) }
}

fn view_of(browser: &IShellBrowser) -> Result<ExplorerView> {
    // SAFETY: COM calls on an STA thread; the folder's ID list is freed here.
    unsafe {
        let view: IFolderView = browser.QueryActiveShellView()?.cast()?;
        let folder = view
            .GetFolder::<IPersistFolder2>()
            .and_then(|f| f.GetCurFolder())
            .ok()
            .and_then(|pidl| {
                let path = SHCreateItemFromIDList::<IShellItem>(pidl)
                    .ok()
                    .and_then(|item| item_path(&item));
                CoTaskMemFree(Some(pidl as *const _));
                path
            });

        let mut selection = Vec::new();
        let mut selected = 0;
        // Fails when nothing is selected.
        if let Ok(items) = view.Items::<IShellItemArray>(SVGIO_SELECTION) {
            let count = items.GetCount().unwrap_or(0);
            selected = count as usize;
            for i in 0..count.min(MAX_SELECTION as u32) {
                if let Some(path) = items.GetItemAt(i).ok().and_then(|item| item_path(&item)) {
                    selection.push(path);
                }
            }
        }
        Ok(ExplorerView {
            folder,
            selection,
            selected,
        })
    }
}

/// A file system path; `None` for virtual items like This PC.
fn item_path(item: &IShellItem) -> Option<String> {
    // SAFETY: GetDisplayName returns a CoTaskMem string we own and free here.
    unsafe {
        let raw = item.GetDisplayName(SIGDN_FILESYSPATH).ok()?;
        let path = String::from_utf16_lossy(raw.as_wide());
        CoTaskMemFree(Some(raw.0 as *const _));
        Some(path)
    }
}

fn variant_i32(value: i32) -> VARIANT {
    let mut variant = VARIANT::default();
    // SAFETY: writing the tag and the matching union field of a zeroed VARIANT.
    unsafe {
        let inner = &mut *variant.Anonymous.Anonymous;
        inner.vt = VT_I4;
        inner.Anonymous.lVal = value;
    }
    variant
}

/// SAFETY: `hwnd` is any window handle; a stale one yields empty text.
unsafe fn window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    // SAFETY: the buffer outlives the call.
    let len = unsafe { GetWindowTextW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
}

/// SAFETY: as for `window_text`.
unsafe fn class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    // SAFETY: the buffer outlives the call.
    let len = unsafe { GetClassNameW(hwnd, &mut buf) };
    String::from_utf16_lossy(&buf[..usize::try_from(len).unwrap_or(0)])
}

/// "explorer.exe" for the process `pid`.
fn process_name(pid: u32) -> Option<String> {
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
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        Path::new(&path)
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::platform::Shell;

    #[test]
    fn recognizes_explorer_and_the_desktop() {
        assert!(is_shell_view("CabinetWClass"));
        assert!(is_shell_view("WorkerW"));
        assert!(!is_shell_view("Chrome_WidgetWin_1"));
    }

    /// Real Windows: the shell's window list opens, and a window that isn't
    /// Explorer's gives no view. Exercises the COM path without needing a
    /// visible Explorer window on the CI runner.
    #[tokio::test]
    async fn reads_the_shell_window_list() {
        let shell = Shell::new();
        let foreground = shell.foreground_window();
        eprintln!("foreground: {foreground:?}");
        let not_explorer = ForegroundWindow {
            handle: 0,
            title: String::new(),
            class: "CabinetWClass".into(),
            app: String::new(),
        };
        let view = shell.explorer_view(not_explorer).await;
        eprintln!("view: {view:?}");
        // A CI runner may have no shell running; it must fail cleanly.
        assert!(!matches!(view, Ok(Some(_))), "{view:?}");
    }
}
