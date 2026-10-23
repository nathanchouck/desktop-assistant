//! The request itself, over WinHTTP (synchronous mode).

use std::ffi::c_void;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts, INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
    WINHTTP_FLAG_SECURE, WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};

use super::{NetError, Request, Response, MAX_BYTES};

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

fn last_error(step: &str) -> NetError {
    NetError::Failed(format!(
        "{step} failed: {}",
        std::io::Error::last_os_error()
    ))
}

/// Sends `request` to `https://<host><path>` and reads the answer.
pub fn send(request: &Request) -> Result<Response, NetError> {
    let agent = wide("desktop-assistant");
    let host = wide(&request.host);
    let path = wide(&request.path);
    let verb = wide(request.method.as_str());
    let headers = request.header_block();
    let headers = (!headers.is_empty()).then(|| wide(&headers));
    let body_len = u32::try_from(request.body.len()).map_err(|_| NetError::TooBig)?;
    let timeout = i32::try_from(request.timeout.as_millis()).unwrap_or(i32::MAX);
    // SAFETY: every pointer passed is either null where WinHTTP allows it
    // or points at a live, NUL-terminated UTF-16 buffer, the request body
    // (of the length given) or a local variable of the size given; handles
    // are checked before use and closed by `Handle`.
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
        WinHttpSetTimeouts(session.0, timeout, timeout, timeout, timeout);
        let connection = Handle(WinHttpConnect(
            session.0,
            host.as_ptr(),
            INTERNET_DEFAULT_HTTPS_PORT,
            0,
        ));
        if connection.0.is_null() {
            return Err(last_error("WinHttpConnect"));
        }
        let request_handle = Handle(WinHttpOpenRequest(
            connection.0,
            verb.as_ptr(),
            path.as_ptr(),
            null(),
            null(),
            null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request_handle.0.is_null() {
            return Err(last_error("WinHttpOpenRequest"));
        }
        let (headers_ptr, headers_len) = match &headers {
            // NUL-terminated, so WinHTTP measures it itself ((DWORD)-1).
            Some(h) => (h.as_ptr(), u32::MAX),
            None => (null(), 0),
        };
        let body_ptr: *const c_void = if request.body.is_empty() {
            null()
        } else {
            request.body.as_ptr().cast()
        };
        if WinHttpSendRequest(
            request_handle.0,
            headers_ptr,
            headers_len,
            body_ptr,
            body_len,
            body_len,
            0,
        ) == 0
        {
            return Err(last_error("sending the request"));
        }
        if WinHttpReceiveResponse(request_handle.0, null_mut()) == 0 {
            return Err(last_error("receiving the response"));
        }
        let mut status: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let queried = WinHttpQueryHeaders(
            request_handle.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            null(),
            (&mut status as *mut u32).cast(),
            &mut size,
            null_mut(),
        );
        if queried == 0 {
            return Err(last_error("reading the status"));
        }
        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            if WinHttpQueryDataAvailable(request_handle.0, &mut available) == 0 {
                return Err(last_error("reading the response"));
            }
            if available == 0 {
                break;
            }
            let start = body.len();
            if start + available as usize > MAX_BYTES {
                return Err(NetError::TooBig);
            }
            body.resize(start + available as usize, 0);
            let mut read: u32 = 0;
            if WinHttpReadData(
                request_handle.0,
                body[start..].as_mut_ptr().cast(),
                available,
                &mut read,
            ) == 0
            {
                return Err(last_error("reading the response"));
            }
            body.truncate(start + read as usize);
        }
        Ok(Response {
            status: u16::try_from(status).unwrap_or(u16::MAX),
            body,
        })
    }
}
