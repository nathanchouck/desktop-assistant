//! The one-shot web server on 127.0.0.1 (a random free port) that the
//! consent page sends you back to (RFC 8252 §7.3). It answers the browser
//! with a short page and hands the code over; requests without our state
//! are refused, and it gives up after `timeout`.

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use http_body_util::Full;
use hyper::body::Bytes;
use hyper::header::CONTENT_TYPE;
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpListener;
use tokio::sync::mpsc;

use super::oauth::{parse_callback, Callback, CallbackError};

/// After a failed accept, before the next one.
const ACCEPT_RETRY: Duration = Duration::from_millis(200);

pub struct Loopback {
    listener: TcpListener,
    port: u16,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum WaitError {
    #[error("nobody approved in the browser within {} minutes", .0.as_secs() / 60)]
    TimedOut(Duration),
}

impl Loopback {
    pub async fn bind() -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
        let port = listener.local_addr()?.port();
        Ok(Self { listener, port })
    }

    /// Where the consent page sends you back. An IP literal, not
    /// `localhost`, so no other program can answer for it (RFC 8252 §8.3).
    pub fn redirect_uri(&self) -> String {
        format!("http://127.0.0.1:{}", self.port)
    }

    /// Serves until a redirect carrying `state` arrives (or `timeout`).
    pub async fn wait(self, state: &str, timeout: Duration) -> Result<Callback, WaitError> {
        let (sender, mut received) = mpsc::channel::<Callback>(1);
        let state: Arc<str> = Arc::from(state);
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);
        loop {
            tokio::select! {
                _ = &mut deadline => return Err(WaitError::TimedOut(timeout)),
                Some(callback) = received.recv() => return Ok(callback),
                accepted = self.listener.accept() => {
                    let stream = match accepted {
                        Ok((stream, _)) => stream,
                        Err(e) => {
                            tracing::debug!("sign-in listener: {e}");
                            tokio::time::sleep(ACCEPT_RETRY).await;
                            continue;
                        }
                    };
                    let sender = sender.clone();
                    let state = Arc::clone(&state);
                    tokio::spawn(async move {
                        let service = hyper::service::service_fn(move |req: Request<hyper::body::Incoming>| {
                            let (status, page, callback) = answer(req.method(), req.uri().path(), req.uri().query(), &state);
                            if let Some(callback) = callback {
                                let _ = sender.try_send(callback);
                            }
                            async move { Ok::<_, Infallible>(html(status, page)) }
                        });
                        if let Err(e) = hyper::server::conn::http1::Builder::new()
                            .serve_connection(TokioIo::new(stream), service)
                            .await
                        {
                            tracing::debug!("sign-in connection ended: {e}");
                        }
                    });
                }
            }
        }
    }
}

const SIGNED_IN: &str = "Signed in to Google. You can close this tab and go back to the launcher.";
const REFUSED: &str = "Google sign-in was cancelled. You can close this tab.";
const OUT_OF_DATE: &str =
    "This sign-in page is out of date. Start again from the launcher's settings.";
const NOT_FOUND: &str = "Not found.";

/// What to tell the browser for a request, and the callback it carried.
pub fn answer(
    method: &Method,
    path: &str,
    query: Option<&str>,
    state: &str,
) -> (StatusCode, &'static str, Option<Callback>) {
    if method != Method::GET || path != "/" {
        return (StatusCode::NOT_FOUND, NOT_FOUND, None);
    }
    match parse_callback(query.unwrap_or(""), state) {
        Ok(callback @ Callback::Code(_)) => (StatusCode::OK, SIGNED_IN, Some(callback)),
        Ok(callback @ Callback::Refused(_)) => (StatusCode::OK, REFUSED, Some(callback)),
        Err(CallbackError::WrongState) => (StatusCode::BAD_REQUEST, OUT_OF_DATE, None),
        Err(CallbackError::NotACallback) => (StatusCode::NOT_FOUND, NOT_FOUND, None),
    }
}

/// A plain page in the browser's own style.
fn html(status: StatusCode, text: &str) -> Response<Full<Bytes>> {
    let body = format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Desktop Assistant</title>\
         <p style=\"font: 16px system-ui, sans-serif; margin: 3em auto; max-width: 32em\">{text}</p>"
    );
    let mut response = Response::new(Full::new(Bytes::from(body)));
    *response.status_mut() = status;
    if let Ok(value) = "text/html; charset=utf-8".parse() {
        response.headers_mut().insert(CONTENT_TYPE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    #[test]
    fn answers_by_request() {
        let get = Method::GET;
        let (status, page, callback) = answer(&get, "/", Some("state=s1&code=4%2Fx"), "s1");
        assert_eq!((status, page), (StatusCode::OK, SIGNED_IN));
        assert_eq!(callback, Some(Callback::Code("4/x".into())));
        let (_, page, callback) = answer(&get, "/", Some("state=s1&error=access_denied"), "s1");
        assert_eq!(page, REFUSED);
        assert_eq!(callback, Some(Callback::Refused("access_denied".into())));
        let (status, _, callback) = answer(&get, "/", Some("state=old&code=x"), "s1");
        assert_eq!((status, callback), (StatusCode::BAD_REQUEST, None));
        assert_eq!(
            answer(&get, "/favicon.ico", None, "s1").0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(answer(&get, "/", None, "s1").0, StatusCode::NOT_FOUND);
        assert_eq!(
            answer(&Method::POST, "/", Some("state=s1&code=x"), "s1").2,
            None
        );
    }

    async fn request(port: u16, target: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let text = format!("GET {target} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n");
        stream.write_all(text.as_bytes()).await.unwrap();
        let mut reply = String::new();
        stream.read_to_string(&mut reply).await.unwrap();
        reply
    }

    #[tokio::test]
    async fn catches_the_redirect() {
        let loopback = Loopback::bind().await.unwrap();
        let port = loopback.port;
        assert_eq!(loopback.redirect_uri(), format!("http://127.0.0.1:{port}"));
        let waiting =
            tokio::spawn(async move { loopback.wait("st", Duration::from_secs(10)).await });
        // A stray request and a wrong state don't end the wait.
        assert!(request(port, "/favicon.ico")
            .await
            .starts_with("HTTP/1.1 404"));
        assert!(request(port, "/?state=nope&code=x")
            .await
            .starts_with("HTTP/1.1 400"));
        let reply = request(port, "/?state=st&code=4%2F0AQ&scope=email").await;
        assert!(reply.starts_with("HTTP/1.1 200"), "{reply}");
        assert!(reply.contains("close this tab"));
        assert_eq!(waiting.await.unwrap(), Ok(Callback::Code("4/0AQ".into())));
    }

    #[tokio::test]
    async fn gives_up_in_time() {
        let loopback = Loopback::bind().await.unwrap();
        let result = loopback.wait("st", Duration::from_millis(50)).await;
        assert_eq!(result, Err(WaitError::TimedOut(Duration::from_millis(50))));
    }
}
