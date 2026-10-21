//! A small HTTPS GET over WinHTTP, for the exchange rates. WinHTTP comes
//! with Windows, uses the system's certificates and proxy settings, and
//! saves pulling a TLS stack into the app for one file a day.

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

/// Bigger answers aren't the rates file.
const MAX_BYTES: usize = 1 << 20;
/// Per step (resolve, connect, send, receive), in milliseconds.
const TIMEOUT_MS: i32 = 15_000;

/// Closes a WinHTTP handle when dropped.
struct Handle(*mut c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle WinHTTP gave us, closed once.
            unsafe { WinHttpCloseHandle(self.0) };
        }
    }
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error(step: &str) -> String {
    format!("{step} failed: {}", std::io::Error::last_os_error())
}

/// Downloads `https://<host><path>`. Blocks; call it off the async runtime.
pub fn https_get(host: &str, path: &str) -> Result<Vec<u8>, String> {
    let agent = wide("desktop-assistant");
    let host = wide(host);
    let path = wide(path);
    let verb = wide("GET");
    // SAFETY: every pointer passed is either null where WinHTTP allows it
    // or points at a live, NUL-terminated UTF-16 buffer or a local
    // variable of the size given; handles are checked before use and
    // closed by `Handle`.
    unsafe {
        let session = Handle(WinHttpOpen(
            agent.as_ptr(),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            null(),
            null(),
            0,
        ));
        if session.0.is_null() {
            return Err(last_error("WinHttpOpen"));
        }
        WinHttpSetTimeouts(session.0, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS, TIMEOUT_MS);
        let connection = Handle(WinHttpConnect(
            session.0,
            host.as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if connection.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let request = Handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        if WinHttpSendRequest(request.0, null(), 0, null(), 0, 0, 0) == 0 {
            return Err(last_error("sending the request"));
        }
        if WinHttpReceiveResponse(request.0, null_mut()) == 0 {
            return Err(last_error("receiving the response"));
        }
        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let queried = WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            null_mut(),
        );
        if queried == 0 {
            return Err(last_error("reading the status"));
        }
        if status != 200 {
            return Err(format!("the server answered {status}"));
        }
        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request.0, &mut available) == 0 {
                return Err(last_error("reading the response"));
            }
            if available == 0 {
                break;
            }
            let start = body.len();
            if start + available as usize > MAX_BYTES {
                return Err("the response is too big".into());
            }
            body.resize(start + available as usize, 0);
            let mut read: u32 = 0;
            if WinHttpReadData(
                request.0,
                body[start..].as_mut_ptr().cast(),
                available,
                &mut read,
            ) == 0
            {
                return Err(last_error("reading the response"));
            }
            body.truncate(start + read as usize);
        }
        Ok(body)
    }
}
