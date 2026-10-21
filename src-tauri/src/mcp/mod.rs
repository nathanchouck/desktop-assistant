//! A small MCP server inside the launcher, so Claude Code can use the
//! desktop: what's on screen, the launcher's search, opening files, and a
//! screenshot of the window you were in (`tools.rs`).
//!
//! Streamable HTTP on 127.0.0.1 with a random port and a secret token per
//! run. Each request gets a plain JSON reply (no event stream), which the
//! spec allows. Requests from browsers (they send `Origin`) and requests
//! with any other host name (DNS rebinding) are refused.

pub mod protocol;
pub mod tools;

use std::convert::Infallible;
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use http_body_util::{BodyExt, Full, Limited};
use hyper::body::{Bytes, Incoming};
use hyper::header::{AUTHORIZATION, CONTENT_TYPE, HOST, ORIGIN};
use hyper::{Method, Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use serde_json::{json, Value};
use tokio::net::TcpListener;

use protocol::{ToolHost, SERVER_NAME};

/// Request bodies over this size are refused.
const MAX_BODY: usize = 1024 * 1024;
const PATH: &str = "/mcp";

/// Where the running server is, for Claude Code's `--mcp-config`.
#[derive(Debug, Clone)]
pub struct Endpoint {
    pub port: u16,
    pub token: String,
}

impl Endpoint {
    /// The `--mcp-config` JSON naming this server.
    pub fn config(&self) -> Value {
        json!({
            "mcpServers": {
                SERVER_NAME: {
                    "type": "http",
                    "url": format!("http://127.0.0.1:{}{PATH}", self.port),
                    "headers": { "Authorization": format!("Bearer {}", self.token) },
                }
            }
        })
    }

    /// Writes the config next to the app's data, readable only by this
    /// user's processes like the rest of it. Returns its path.
    pub fn write_config(&self, dir: &Path) -> std::io::Result<PathBuf> {
        std::fs::create_dir_all(dir)?;
        let path = dir.join("mcp-config.json");
        std::fs::write(&path, self.config().to_string())?;
        Ok(path)
    }
}

/// Starts serving `host` on a free local port.
pub async fn start(host: Arc<dyn ToolHost>) -> std::io::Result<Endpoint> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0))).await?;
    let endpoint = Endpoint {
        port: listener.local_addr()?.port(),
        token: random_token()?,
    };
    let shared = Arc::new((host, endpoint.clone()));
    tokio::spawn(async move {
        loop {
            let stream = match listener.accept().await {
                Ok((stream, _)) => stream,
                Err(e) => {
                    tracing::warn!("MCP server stopped accepting: {e}");
                    continue;
                }
            };
            let shared = Arc::clone(&shared);
            tokio::spawn(async move {
                let service = hyper::service::service_fn(move |req| {
                    let shared = Arc::clone(&shared);
                    async move { Ok::<_, Infallible>(respond(&shared.0, &shared.1, req).await) }
                });
                if let Err(e) = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await
                {
                    tracing::debug!("MCP connection ended: {e}");
                }
            });
        }
    });
    tracing::info!("MCP server on 127.0.0.1:{}", endpoint.port);
    Ok(endpoint)
}

async fn respond(
    host: &Arc<dyn ToolHost>,
    endpoint: &Endpoint,
    req: Request<Incoming>,
) -> Response<Full<Bytes>> {
    if let Err(status) = check_request(&req, endpoint) {
        return plain(status);
    }
    if req.method() != Method::POST {
        // No server-to-client event stream; the spec allows saying so.
        return plain(StatusCode::METHOD_NOT_ALLOWED);
    }
    let body = match Limited::new(req.into_body(), MAX_BODY).collect().await {
        Ok(body) => body.to_bytes(),
        Err(_) => return plain(StatusCode::PAYLOAD_TOO_LARGE),
    };
    let Ok(message) = serde_json::from_slice::<Value>(&body) else {
        return json_reply(&json!({
            "jsonrpc": "2.0",
            "id": null,
            "error": { "code": -32700, "message": "not JSON" },
        }));
    };
    let reply = match &message {
        // Older clients may batch several messages.
        Value::Array(batch) => {
            let mut replies = Vec::new();
            for item in batch {
                if let Some(reply) = protocol::handle(host.as_ref(), item).await {
                    replies.push(reply);
                }
            }
            (!replies.is_empty()).then_some(Value::Array(replies))
        }
        single => protocol::handle(host.as_ref(), single).await,
    };
    match reply {
        Some(reply) => json_reply(&reply),
        None => plain(StatusCode::ACCEPTED),
    }
}

/// Only Claude Code on this machine, with the token, at our address.
fn check_request<B>(req: &Request<B>, endpoint: &Endpoint) -> Result<(), StatusCode> {
    if req.uri().path() != PATH {
        return Err(StatusCode::NOT_FOUND);
    }
    if req.headers().contains_key(ORIGIN) {
        return Err(StatusCode::FORBIDDEN);
    }
    let host = req
        .headers()
        .get(HOST)
        .and_then(|h| h.to_str().ok())
        .unwrap_or_default();
    let port = endpoint.port;
    if host != format!("127.0.0.1:{port}") && host != format!("localhost:{port}") {
        return Err(StatusCode::FORBIDDEN);
    }
    let expected = format!("Bearer {}", endpoint.token);
    let given = req
        .headers()
        .get(AUTHORIZATION)
        .map(|v| v.as_bytes())
        .unwrap_or_default();
    if !same_bytes(given, expected.as_bytes()) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(())
}

/// Compares without stopping at the first difference.
fn same_bytes(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

fn random_token() -> std::io::Result<String> {
    let mut bytes = [0u8; 32];
    getrandom::fill(&mut bytes).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

fn plain(status: StatusCode) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::new()));
    *response.status_mut() = status;
    response
}

fn json_reply(value: &Value) -> Response<Full<Bytes>> {
    let mut response = Response::new(Full::new(Bytes::from(value.to_string())));
    response.headers_mut().insert(
        CONTENT_TYPE,
        "application/json".parse().expect("valid header"),
    );
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// One raw HTTP/1.1 exchange; returns (status, body).
    async fn post(port: u16, headers: &[(&str, String)], body: &str) -> (u16, String) {
        let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", port))
            .await
            .unwrap();
        let mut request = format!(
            "POST /mcp HTTP/1.1\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (name, value) in headers {
            request.push_str(&format!("{name}: {value}\r\n"));
        }
        request.push_str("\r\n");
        request.push_str(body);
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut raw = String::new();
        stream.read_to_string(&mut raw).await.unwrap();
        let status = raw[9..12].parse().unwrap();
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or_default().to_owned();
        (status, body)
    }

    fn good_headers(endpoint: &Endpoint) -> Vec<(&'static str, String)> {
        vec![
            ("Host", format!("127.0.0.1:{}", endpoint.port)),
            ("Authorization", format!("Bearer {}", endpoint.token)),
        ]
    }

    #[tokio::test]
    async fn serves_tools_over_http() {
        let endpoint = start(Arc::new(protocol::tests::Echo)).await.unwrap();
        let headers = good_headers(&endpoint);

        let (status, body) = post(
            endpoint.port,
            &headers,
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"echo","arguments":{"text":"hello"}}}"#,
        )
        .await;
        assert_eq!(status, 200);
        let reply: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(reply["result"]["content"][0]["text"], "hello");

        let (status, _) = post(
            endpoint.port,
            &headers,
            r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        )
        .await;
        assert_eq!(status, 202);
    }

    #[tokio::test]
    async fn refuses_strangers() {
        let endpoint = start(Arc::new(protocol::tests::Echo)).await.unwrap();
        let ping = r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#;
        let host = ("Host", format!("127.0.0.1:{}", endpoint.port));

        let (status, _) = post(endpoint.port, std::slice::from_ref(&host), ping).await;
        assert_eq!(status, 401, "no token");

        let wrong = ("Authorization", "Bearer nope".to_owned());
        let (status, _) = post(endpoint.port, &[host.clone(), wrong], ping).await;
        assert_eq!(status, 401, "wrong token");

        let mut from_browser = good_headers(&endpoint);
        from_browser.push(("Origin", "https://evil.example".into()));
        let (status, _) = post(endpoint.port, &from_browser, ping).await;
        assert_eq!(status, 403, "browser origin");

        let mut rebound = good_headers(&endpoint);
        rebound[0].1 = format!("evil.example:{}", endpoint.port);
        let (status, _) = post(endpoint.port, &rebound, ping).await;
        assert_eq!(status, 403, "other host name");
    }

    #[test]
    fn config_names_the_server_and_token() {
        let endpoint = Endpoint {
            port: 4321,
            token: "abc".into(),
        };
        let config = endpoint.config();
        let server = &config["mcpServers"]["launcher"];
        assert_eq!(server["type"], "http");
        assert_eq!(server["url"], "http://127.0.0.1:4321/mcp");
        assert_eq!(server["headers"]["Authorization"], "Bearer abc");
    }
}
