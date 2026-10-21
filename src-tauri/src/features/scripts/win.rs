//! Windows: reading output in the console's code page, and stopping a
//! script together with everything it started.

use std::time::Duration;

use windows_sys::Win32::Globalization::{MultiByteToWideChar, CP_OEMCP};

/// `bytes` read in the OEM code page, which cmd's `echo` and Windows
/// PowerShell use when printing into a pipe.
pub fn oem_to_string(bytes: &[u8]) -> Option<String> {
    if bytes.is_empty() {
        return Some(String::new());
    }
    let len = i32::try_from(bytes.len()).ok()?;
    // SAFETY: the pointer and length describe `bytes`; a null output
    // buffer asks for the size needed.
    let needed = unsafe {
        MultiByteToWideChar(
            CP_OEMCP,
            0,
            bytes.as_ptr(),
            len,
            std::ptr::null_mut(),
            0,
        )
    };
    let size = usize::try_from(needed).ok().filter(|n| *n > 0)?;
    let mut wide = vec![0u16; size];
    // SAFETY: `wide` has room for `needed` UTF-16 units.
    let written = unsafe {
        MultiByteToWideChar(CP_OEMCP, 0, bytes.as_ptr(), len, wide.as_mut_ptr(), needed)
    };
    wide.truncate(usize::try_from(written).ok().filter(|n| *n > 0)?);
    Some(String::from_utf16_lossy(&wide))
}

/// Ends process `pid` and the processes it started (a script's own
/// programs would otherwise live on).
pub async fn kill_tree(pid: u32) {
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut taskkill = tokio::process::Command::new("taskkill.exe");
    taskkill
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(5), taskkill.status()).await {
        Ok(Ok(_)) => {}
        Ok(Err(e)) => tracing::debug!("taskkill failed: {e}"),
        Err(_) => tracing::debug!("taskkill took too long"),
    }
}
