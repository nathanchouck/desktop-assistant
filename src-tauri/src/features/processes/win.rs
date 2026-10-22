//! Windows: listing running processes (with their memory and window
//! titles) and ending them.

use std::collections::HashMap;
use std::ffi::c_void;
use std::time::{Duration, Instant};

use windows::core::PWSTR;
use windows::Win32::Foundation::{
    CloseHandle, ERROR_ACCESS_DENIED, ERROR_INVALID_PARAMETER, HANDLE, HWND, LPARAM,
};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
};
use windows::Win32::System::ProcessStatus::{
    GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
};
use windows::Win32::System::Threading::{
    OpenProcess, QueryFullProcessImageNameW, TerminateProcess, WaitForSingleObject,
    PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_SYNCHRONIZE, PROCESS_TERMINATE,
};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindow, GetWindowLongW, GetWindowTextW,
    GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
};

use super::list::{Outcome, Proc};

pub const SUPPORTED: bool = true;

/// How long ending a program may wait for its processes to exit, so the
/// refreshed list doesn't still show them.
const EXIT_WAIT: Duration = Duration::from_secs(1);
/// Exit code of an ended process (Task Manager uses 1 too).
const ENDED_CODE: u32 = 1;

/// Closes a handle when dropped.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: a handle we opened and close once.
        let _ = unsafe { CloseHandle(self.0) };
    }
}

/// Every running process, with its path, memory and windows where
/// Windows lets us read them.
pub fn snapshot() -> Result<Vec<Proc>, String> {
    let mut windows = window_titles();
    // SAFETY: a snapshot handle we own; `entry` is sized as the API asks.
    unsafe {
        let snap = Owned(CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0).map_err(|e| e.message())?);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        Process32FirstW(snap.0, &mut entry).map_err(|e| e.message())?;
        let mut procs = Vec::new();
        loop {
            let pid = entry.th32ProcessID;
            let (path, memory) = details(pid);
            procs.push(Proc {
                pid,
                parent: entry.th32ParentProcessID,
                exe: wide(&entry.szExeFile),
                path,
                memory,
                windows: windows.remove(&pid).unwrap_or_default(),
            });
            if Process32NextW(snap.0, &mut entry).is_err() {
                break;
            }
        }
        Ok(procs)
    }
}

/// Text up to the first NUL.
fn wide(chars: &[u16]) -> String {
    let len = chars.iter().position(|&c| c == 0).unwrap_or(chars.len());
    String::from_utf16_lossy(&chars[..len])
}

/// The program's path and its private memory (`None`/0 for processes
/// Windows doesn't let us look at, e.g. other users' or protected ones).
fn details(pid: u32) -> (Option<String>, u64) {
    if pid == 0 || pid == 4 {
        return (None, 0);
    }
    // SAFETY: querying a process we opened for that; the handle is closed
    // on drop. Buffers outlive the calls.
    unsafe {
        let Ok(handle) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else {
            return (None, 0);
        };
        let process = Owned(handle);
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let path = QueryFullProcessImageNameW(
            process.0,
            PROCESS_NAME_WIN32,
            PWSTR(buf.as_mut_ptr()),
            &mut len,
        )
        .ok()
        .map(|()| String::from_utf16_lossy(&buf[..len as usize]));
        let mut counters = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        let memory = GetProcessMemoryInfo(
            process.0,
            std::ptr::from_mut(&mut counters).cast::<PROCESS_MEMORY_COUNTERS>(),
            counters.cb,
        )
        .map_or(0, |()| counters.PrivateUsage as u64);
        (path, memory)
    }
}

/// Titles of visible app windows, by process. Skips tool windows, hidden
/// (cloaked) ones such as suspended Store apps, and the frames that host
/// Store apps (they belong to ApplicationFrameHost, not the app).
fn window_titles() -> HashMap<u32, Vec<String>> {
    unsafe extern "system" fn each(hwnd: HWND, data: LPARAM) -> windows::core::BOOL {
        // SAFETY: `data` is the map below, alive for the whole enumeration;
        // the rest are read-only queries on a window handle.
        unsafe {
            let titles = &mut *(data.0 as *mut HashMap<u32, Vec<String>>);
            if let Some((pid, title)) = app_window(hwnd) {
                titles.entry(pid).or_default().push(title);
            }
        }
        true.into()
    }
    let mut titles: HashMap<u32, Vec<String>> = HashMap::new();
    // SAFETY: the callback only runs during this call, while `titles` lives.
    let _ = unsafe { EnumWindows(Some(each), LPARAM(std::ptr::from_mut(&mut titles) as isize)) };
    titles
}

/// The process and title of a window you'd see on the taskbar.
unsafe fn app_window(hwnd: HWND) -> Option<(u32, String)> {
    // SAFETY: read-only queries on a window handle from EnumWindows.
    unsafe {
        if !IsWindowVisible(hwnd).as_bool() {
            return None;
        }
        if GetWindow(hwnd, GW_OWNER).is_ok_and(|owner| !owner.is_invalid()) {
            return None;
        }
        if GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW.0 != 0 {
            return None;
        }
        let mut cloaked = 0u32;
        let read = DwmGetWindowAttribute(
            hwnd,
            DWMWA_CLOAKED,
            std::ptr::from_mut(&mut cloaked).cast::<c_void>(),
            4,
        );
        if read.is_ok() && cloaked != 0 {
            return None;
        }
        let mut class = [0u16; 64];
        let n = GetClassNameW(hwnd, &mut class);
        if wide(&class[..n.max(0) as usize]) == "ApplicationFrameWindow" {
            return None;
        }
        let mut text = [0u16; 256];
        let n = GetWindowTextW(hwnd, &mut text);
        let title = wide(&text[..n.max(0) as usize]).trim().to_owned();
        if title.is_empty() {
            return None;
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, Some(&mut pid));
        (pid != 0).then_some((pid, title))
    }
}

/// Ends each process (in this order), then waits briefly for them to exit.
pub fn end(pids: &[u32]) -> Vec<Outcome> {
    let mut outcomes = Vec::with_capacity(pids.len());
    let mut exiting = Vec::new();
    for &pid in pids {
        // SAFETY: a process handle we open for ending and waiting, closed
        // on drop.
        let opened = unsafe { OpenProcess(PROCESS_TERMINATE | PROCESS_SYNCHRONIZE, false, pid) };
        let process = match opened {
            Ok(handle) => Owned(handle),
            Err(e) => {
                outcomes.push(why(&e));
                continue;
            }
        };
        // SAFETY: as above.
        match unsafe { TerminateProcess(process.0, ENDED_CODE) } {
            Ok(()) => {
                outcomes.push(Outcome::Ended);
                exiting.push(process);
            }
            Err(e) => outcomes.push(why(&e)),
        }
    }
    let deadline = Instant::now() + EXIT_WAIT;
    for process in &exiting {
        let left = deadline.saturating_duration_since(Instant::now());
        // SAFETY: waiting on a handle we hold.
        let _ = unsafe { WaitForSingleObject(process.0, left.as_millis() as u32) };
    }
    outcomes
}

fn why(e: &windows::core::Error) -> Outcome {
    if e.code() == ERROR_ACCESS_DENIED.to_hresult() {
        Outcome::Denied
    } else if e.code() == ERROR_INVALID_PARAMETER.to_hresult() {
        // No process with that ID: it has exited.
        Outcome::Gone
    } else {
        Outcome::Failed(e.message())
    }
}
