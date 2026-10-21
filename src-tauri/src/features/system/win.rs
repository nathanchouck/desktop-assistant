//! Windows: locking, sleep, signing out and restarting; the Recycle Bin;
//! the light/dark setting.

use std::ffi::c_void;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE, LPARAM, LUID, WPARAM};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, SE_PRIVILEGE_ENABLED,
    SE_SHUTDOWN_NAME, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Power::{
    GetPwrCapabilities, IsPwrHibernateAllowed, SetSuspendState, SYSTEM_POWER_CAPABILITIES,
};
use windows::Win32::System::Registry::{
    RegGetValueW, RegSetKeyValueW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, REG_DWORD,
    RRF_RT_REG_DWORD,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    SHEmptyRecycleBinW, SHQueryRecycleBinW, SHERB_NOCONFIRMATION, SHERB_NOPROGRESSUI,
    SHQUERYRBINFO,
};
use windows::Win32::UI::WindowsAndMessaging::{
    PostMessageW, SendMessageTimeoutW, HWND_BROADCAST, SC_MONITORPOWER, SMTO_ABORTIFHUNG,
    WM_SETTINGCHANGE, WM_SYSCOMMAND,
};
use windows_sys::Win32::System::Shutdown::{
    ExitWindowsEx, LockWorkStation, EWX_HYBRID_SHUTDOWN, EWX_LOGOFF, EWX_POWEROFF, EWX_REBOOT,
    EWX_SHUTDOWN, SHTDN_REASON_FLAG_PLANNED, SHTDN_REASON_MAJOR_OTHER, SHTDN_REASON_MINOR_OTHER,
};

use super::commands::{BinContents, Exit};

pub const SUPPORTED: bool = true;

const PERSONALIZE: PCWSTR = w!(r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize");
/// Fast startup ("hybrid" shut down), as the Start menu's Shut Down uses it.
const POWER: PCWSTR = w!(r"SYSTEM\CurrentControlSet\Control\Session Manager\Power");
/// How long each window gets to hear about the theme change.
const BROADCAST_TIMEOUT_MS: u32 = 200;
/// `SC_MONITORPOWER` argument: turn the display off.
const MONITOR_OFF: isize = 2;

fn last_error() -> String {
    // SAFETY: no preconditions.
    unsafe { GetLastError() }.to_hresult().message()
}

pub fn lock() -> Result<(), String> {
    // SAFETY: no preconditions; fails if the session can't be locked.
    if unsafe { LockWorkStation() } == 0 {
        return Err(last_error());
    }
    Ok(())
}

pub fn hibernate_available() -> bool {
    // SAFETY: no preconditions.
    unsafe { IsPwrHibernateAllowed() }
}

/// Sleep (or hibernate). On PCs with Modern Standby there's no classic
/// sleep to ask for: turning the display off is what sends them to
/// standby, as the power button does.
pub fn suspend(hibernate: bool) -> Result<(), String> {
    if !hibernate && modern_standby() {
        // SAFETY: posting a system command; every top-level window's
        // default handler turns the display off.
        return unsafe {
            PostMessageW(
                Some(HWND_BROADCAST),
                WM_SYSCOMMAND,
                WPARAM(SC_MONITORPOWER as usize),
                LPARAM(MONITOR_OFF),
            )
        }
        .map_err(|e| e.message());
    }
    // SAFETY: no preconditions. Returns after the PC wakes up again.
    if unsafe { SetSuspendState(hibernate, false, false) } {
        Ok(())
    } else {
        Err(last_error())
    }
}

fn modern_standby() -> bool {
    let mut caps = SYSTEM_POWER_CAPABILITIES::default();
    // SAFETY: `caps` is a valid out parameter.
    let read = unsafe { GetPwrCapabilities(&mut caps) };
    read && caps.AoAc
}

/// Restart, shut down or sign out, the way the Start menu does: apps can
/// still ask to save their work.
pub fn exit_windows(exit: Exit) -> Result<(), String> {
    let flags = match exit {
        Exit::SignOut => EWX_LOGOFF,
        Exit::Restart => EWX_REBOOT,
        Exit::ShutDown if fast_startup() => EWX_SHUTDOWN | EWX_POWEROFF | EWX_HYBRID_SHUTDOWN,
        Exit::ShutDown => EWX_SHUTDOWN | EWX_POWEROFF,
    };
    if exit != Exit::SignOut {
        enable_shutdown_privilege().map_err(|e| e.message())?;
    }
    let reason = SHTDN_REASON_MAJOR_OTHER | SHTDN_REASON_MINOR_OTHER | SHTDN_REASON_FLAG_PLANNED;
    // SAFETY: plain flags; Windows starts the shutdown and returns.
    if unsafe { ExitWindowsEx(flags, reason) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

fn fast_startup() -> bool {
    read_dword(HKEY_LOCAL_MACHINE, POWER, w!("HiberbootEnabled")) == Some(1)
}

/// Restarting and shutting down need this privilege switched on (every
/// user has it, off by default).
fn enable_shutdown_privilege() -> windows::core::Result<()> {
    // SAFETY: the token handle is ours and closed below; the structures
    // outlive the calls.
    unsafe {
        let mut token = HANDLE::default();
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )?;
        let mut luid = LUID::default();
        let result = LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid).and_then(
            |()| {
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None)
            },
        );
        let _ = CloseHandle(token);
        result
    }
}

/// Items and bytes in the Recycle Bin, across all drives.
pub fn recycle_bin() -> Result<BinContents, String> {
    let mut info = SHQUERYRBINFO {
        cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32,
        ..Default::default()
    };
    // SAFETY: `info` is sized and valid for the call; null = all drives.
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info) }.map_err(|e| e.message())?;
    Ok(BinContents {
        items: u64::try_from(info.i64NumItems).unwrap_or(0),
        bytes: u64::try_from(info.i64Size).unwrap_or(0),
    })
}

/// Empties it on every drive, without Windows' own question (the launcher
/// asked) or progress window.
pub fn empty_recycle_bin() -> Result<(), String> {
    // SAFETY: no window, null root = all drives.
    unsafe {
        SHEmptyRecycleBinW(
            None,
            PCWSTR::null(),
            SHERB_NOCONFIRMATION | SHERB_NOPROGRESSUI,
        )
    }
    .map_err(|e| e.message())
}

/// Whether apps use the light theme now (`None` if Windows doesn't say).
pub fn apps_use_light_theme() -> Option<bool> {
    read_dword(HKEY_CURRENT_USER, PERSONALIZE, w!("AppsUseLightTheme")).map(|v| v != 0)
}

/// Switches apps and Windows to the other theme; returns whether it's
/// light now. Open windows are told in the background.
pub fn toggle_theme() -> Result<bool, String> {
    let light = !apps_use_light_theme().unwrap_or(true);
    let value = u32::from(light);
    for name in [w!("AppsUseLightTheme"), w!("SystemUsesLightTheme")] {
        // SAFETY: `value` outlives the call and is 4 bytes, as REG_DWORD says.
        unsafe {
            RegSetKeyValueW(
                HKEY_CURRENT_USER,
                PERSONALIZE,
                name,
                REG_DWORD.0,
                Some(std::ptr::from_ref(&value).cast::<c_void>()),
                4,
            )
        }
        .ok()
        .map_err(|e| e.message())?;
    }
    // Hung windows can make this take a while; nobody waits for it.
    let told = std::thread::Builder::new()
        .name("theme-broadcast".into())
        .spawn(|| {
            // SAFETY: a static string; the call returns within the timeout
            // per window and skips hung ones.
            unsafe {
                SendMessageTimeoutW(
                    HWND_BROADCAST,
                    WM_SETTINGCHANGE,
                    WPARAM(0),
                    LPARAM(w!("ImmersiveColorSet").as_ptr() as isize),
                    SMTO_ABORTIFHUNG,
                    BROADCAST_TIMEOUT_MS,
                    None,
                );
            }
        });
    if let Err(e) = told {
        tracing::debug!("could not announce the theme change: {e}");
    }
    Ok(light)
}

fn read_dword(root: HKEY, key: PCWSTR, name: PCWSTR) -> Option<u32> {
    let mut value = 0u32;
    let mut size = 4u32;
    // SAFETY: `value` and `size` are valid out parameters for a DWORD.
    let status = unsafe {
        RegGetValueW(
            root,
            key,
            name,
            RRF_RT_REG_DWORD,
            None,
            Some(std::ptr::from_mut(&mut value).cast::<c_void>()),
            Some(&mut size),
        )
    };
    status.is_ok().then_some(value)
}
