//! How late the keyboard hook answers, and how long Windows waits for it.
//!
//! Windows gives a low-level hook `LowLevelHooksTimeout` to answer each key
//! (`HKCU\Control Panel\Desktop`; about 300 ms when unset, at most 1000 ms
//! since Windows 10 1709). Past that it uses the key anyway and ignores our
//! "hide it", which is how a tap opens both the launcher and Start. Each
//! Windows key event's age when we answer is logged, so that can be proven.
//!
//! The value is read by Windows when you sign in, so a change made here
//! applies after you sign out and back in. It's per user and needs no admin
//! rights. The cost of a longer timeout: some *other* app's stuck hook could
//! then freeze typing for up to a second per key instead of ~0.3 s.

use std::sync::OnceLock;

use super::HotkeyError;

/// Answers slower than this are logged as warnings and counted. Below
/// Windows' default timeout, so near misses show up too.
pub const LATE_MS: u32 = 200;
/// What "give the hook more time" sets: the most Windows allows.
pub const LONGER_MS: u32 = 1_000;

/// The value when the launcher started (`None`: not set), to tell whether a
/// change is still waiting for a sign-in.
static AT_STARTUP: OnceLock<Option<u32>> = OnceLock::new();

/// Milliseconds from a key event (`KBDLLHOOKSTRUCT.time`) to `now`, both
/// `GetTickCount` values (which wrap every 49.7 days). A time "from the
/// future" can't be a late answer, so it counts as 0.
pub fn answer_delay_ms(now: u32, event: u32) -> u32 {
    let delay = now.wrapping_sub(event);
    if delay > i32::MAX as u32 {
        0
    } else {
        delay
    }
}

/// A `LowLevelHooksTimeout` value as stored: people create it as a DWORD or
/// as a string, so both are read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RawTimeout<'a> {
    Dword(u32),
    Text(&'a str),
}

/// The timeout in ms Windows would use for `raw` (capped like Windows does).
pub fn parse_timeout(raw: RawTimeout<'_>) -> Option<u32> {
    let ms = match raw {
        RawTimeout::Dword(ms) => ms,
        RawTimeout::Text(text) => text.trim().trim_end_matches('\0').parse().ok()?,
    };
    Some(ms.min(LONGER_MS))
}

/// Remembers the value the session started with. Call once at startup.
pub fn remember_startup_value() {
    AT_STARTUP.get_or_init(configured_ms);
}

/// The configured timeout in ms, `None` when it isn't set.
pub fn configured_ms() -> Option<u32> {
    imp::read()
}

/// Whether the value changed since the launcher started, so it only
/// applies after you sign out and back in.
pub fn change_pending() -> bool {
    AT_STARTUP
        .get()
        .is_some_and(|start| *start != configured_ms())
}

/// `longer`: give the hook a full second. Otherwise remove the value, back
/// to Windows' default.
pub fn set_longer(longer: bool) -> Result<(), HotkeyError> {
    imp::write(longer.then_some(LONGER_MS))?;
    tracing::info!(
        "LowLevelHooksTimeout {}; applies after signing out",
        if longer {
            format!("set to {LONGER_MS} ms")
        } else {
            "removed (Windows' default)".to_owned()
        }
    );
    Ok(())
}

#[cfg(windows)]
mod imp {
    use std::ffi::c_void;

    use windows::core::{w, PCWSTR};
    use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_DWORD,
        REG_VALUE_TYPE, RRF_RT_REG_DWORD, RRF_RT_REG_SZ,
    };

    use super::{parse_timeout, RawTimeout};
    use crate::hotkey::HotkeyError;

    const KEY: PCWSTR = w!("Control Panel\\Desktop");
    const VALUE: PCWSTR = w!("LowLevelHooksTimeout");

    pub fn read() -> Option<u32> {
        let mut buf = [0u16; 32];
        let mut size = std::mem::size_of_val(&buf) as u32;
        let mut kind = REG_VALUE_TYPE::default();
        // SAFETY: constant NUL-terminated names; `buf`/`size` describe a
        // valid writable buffer, and RegGetValueW NUL-terminates strings.
        let rc = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                KEY,
                VALUE,
                RRF_RT_REG_DWORD | RRF_RT_REG_SZ,
                Some(&mut kind),
                Some(buf.as_mut_ptr().cast::<c_void>()),
                Some(&mut size),
            )
        };
        if rc != ERROR_SUCCESS {
            return None;
        }
        if kind == REG_DWORD {
            return parse_timeout(RawTimeout::Dword(
                u32::from(buf[0]) | (u32::from(buf[1]) << 16),
            ));
        }
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        parse_timeout(RawTimeout::Text(&String::from_utf16_lossy(&buf[..len])))
    }

    pub fn write(ms: Option<u32>) -> Result<(), HotkeyError> {
        // SAFETY: constant NUL-terminated names; the data pointer and size
        // describe a live u32.
        let rc = unsafe {
            match ms {
                Some(ms) => RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    KEY,
                    VALUE,
                    REG_DWORD.0,
                    Some((&ms as *const u32).cast::<c_void>()),
                    std::mem::size_of::<u32>() as u32,
                ),
                None => RegDeleteKeyValueW(HKEY_CURRENT_USER, KEY, VALUE),
            }
        };
        if rc == ERROR_SUCCESS || (ms.is_none() && rc == ERROR_FILE_NOT_FOUND) {
            Ok(())
        } else {
            Err(HotkeyError::Hook(format!(
                "could not change LowLevelHooksTimeout: {}",
                std::io::Error::from_raw_os_error(rc.0 as i32)
            )))
        }
    }
}

#[cfg(not(windows))]
mod imp {
    use crate::hotkey::HotkeyError;

    pub fn read() -> Option<u32> {
        None
    }

    pub fn write(_ms: Option<u32>) -> Result<(), HotkeyError> {
        Err(HotkeyError::Hook(
            "the hook timeout only exists on Windows".into(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delay_is_how_long_ago_the_key_happened() {
        assert_eq!(answer_delay_ms(1_350, 1_000), 350);
        assert_eq!(answer_delay_ms(1_000, 1_000), 0);
    }

    #[test]
    fn delay_survives_the_tick_count_wrapping() {
        assert_eq!(answer_delay_ms(40, u32::MAX - 59), 100);
    }

    #[test]
    fn a_key_from_the_future_is_on_time() {
        assert_eq!(answer_delay_ms(1_000, 1_016), 0);
    }

    #[test]
    fn reads_both_ways_people_store_the_timeout() {
        assert_eq!(parse_timeout(RawTimeout::Dword(500)), Some(500));
        assert_eq!(parse_timeout(RawTimeout::Text(" 750 ")), Some(750));
        assert_eq!(parse_timeout(RawTimeout::Text("abc")), None);
    }

    #[test]
    fn caps_the_timeout_like_windows() {
        assert_eq!(parse_timeout(RawTimeout::Dword(5_000)), Some(LONGER_MS));
        assert_eq!(parse_timeout(RawTimeout::Text("2000")), Some(LONGER_MS));
    }
}
