//! Small HTTPS requests over WinHTTP: exchange rates (calculator) and the
//! Google APIs. WinHTTP comes with Windows and uses the system's
//! certificates and proxy settings, which saves pulling a TLS stack into
//! the app for a handful of requests (D-028).
//!
//! Only the Windows build sends anything; elsewhere every request fails
//! with `NetError::Unsupported`, and tests use recorded responses.
//!
//! Requests are never logged with their headers or bodies: they carry
//! access tokens and client secrets.

#[cfg(windows)]
mod win;

use std::fmt;
use std::time::Duration;

/// Responses bigger than this are refused.
pub const MAX_BYTES: usize = 4 << 20;
/// Per step (resolve, connect, send, receive) unless a request says less.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(15);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Get,
    Post,
}

impl Method {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Get => "GET",
            Self::Post => "POST",
        }
    }
}

/// One HTTPS request to `host` (port 443).
#[derive(Clone)]
pub struct Request {
    pub method: Method,
    pub host: String,
    /// The path and query string, already percent-encoded.
    pub path: String,
    /// Extra headers (`Authorization`, `Content-Type`).
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub timeout: Duration,
}

/// Shows where a request goes, never its headers or body (tokens).
impl fmt::Debug for Request {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Request")
            .field("method", &self.method)
            .field("host", &self.host)
            .field("headers", &self.headers.len())
            .field("body", &self.body.len())
            .finish_non_exhaustive()
    }
}

impl Request {
    pub fn get(host: &str, path: &str) -> Self {
        Self {
            method: Method::Get,
            host: host.to_owned(),
            path: path.to_owned(),
            headers: Vec::new(),
            body: Vec::new(),
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// A form post (`application/x-www-form-urlencoded`), as OAuth uses.
    pub fn post_form(host: &str, path: &str, form: String) -> Self {
        Self {
            method: Method::Post,
            body: form.into_bytes(),
            ..Self::get(host, path)
        }
        .header("Content-Type", "application/x-www-form-urlencoded")
    }

    pub fn header(mut self, name: &str, value: &str) -> Self {
        self.headers.push((name.to_owned(), value.to_owned()));
        self
    }

    pub fn timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The headers as WinHTTP takes them: `Name: value` lines.
    pub fn header_block(&self) -> String {
        self.headers
            .iter()
            .map(|(name, value)| format!("{name}: {value}\r\n"))
            .collect()
    }
}

/// What the server answered, whatever the status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
}

impl Response {
    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum NetError {
    /// Offline, DNS, TLS, a time-out: what WinHTTP said.
    #[error("{0}")]
    Failed(String),
    #[error("the response is too big")]
    TooBig,
    #[error("going online is only built for Windows")]
    Unsupported,
}

/// Sends `request` and reads the whole answer. Blocks; call it off the
/// async runtime (or use `fetch`).
pub fn send(request: &Request) -> Result<Response, NetError> {
    #[cfg(windows)]
    {
        win::send(request)
    }
    #[cfg(not(windows))]
    {
        let _ = request;
        Err(NetError::Unsupported)
    }
}

/// `send` on a blocking thread.
pub async fn fetch(request: Request) -> Result<Response, NetError> {
    tokio::task::spawn_blocking(move || send(&request))
        .await
        .unwrap_or_else(|e| Err(NetError::Failed(format!("the request crashed: {e}"))))
}

/// Downloads `https://<host><path>`, which must answer 200. Blocks.
pub fn https_get(host: &str, path: &str) -> Result<Vec<u8>, String> {
    let response = send(&Request::get(host, path)).map_err(|e| e.to_string())?;
    if response.status != 200 {
        return Err(format!("the server answered {}", response.status));
    }
    Ok(response.body)
}

/// Percent-encodes everything but unreserved characters, so `text` can go
/// in a query string, a path or a form body.
pub fn encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len() * 3);
    for byte in text.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// `name=value&…`, each part encoded: a query string or a form body.
pub fn form(pairs: &[(&str, &str)]) -> String {
    pairs
        .iter()
        .map(|(name, value)| format!("{}={}", encode(name), encode(value)))
        .collect::<Vec<_>>()
        .join("&")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_everything_but_unreserved() {
        assert_eq!(encode("a b/c?d=e&f+g~h"), "a%20b%2Fc%3Fd%3De%26f%2Bg~h");
        assert_eq!(encode("é"), "%C3%A9");
        assert_eq!(
            form(&[("q", "name contains 'x'"), ("pageSize", "8")]),
            "q=name%20contains%20%27x%27&pageSize=8"
        );
    }

    #[test]
    fn debug_never_shows_headers_or_body() {
        let request = Request::post_form("oauth2.googleapis.com", "/token", "secret=1".into())
            .header("Authorization", "Bearer ya29.secret");
        let shown = format!("{request:?}");
        assert!(
            !shown.contains("ya29") && !shown.contains("secret=1"),
            "{shown}"
        );
        assert!(shown.contains("oauth2.googleapis.com"));
        assert_eq!(
            request.header_block(),
            "Content-Type: application/x-www-form-urlencoded\r\nAuthorization: Bearer ya29.secret\r\n"
        );
    }

    #[test]
    fn fails_clearly_off_windows() {
        if cfg!(not(windows)) {
            assert_eq!(
                send(&Request::get("example.com", "/")),
                Err(NetError::Unsupported)
            );
            assert!(https_get("example.com", "/").is_err());
        }
    }
}
