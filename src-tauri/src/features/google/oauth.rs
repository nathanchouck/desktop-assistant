//! OAuth 2.0 for installed apps (RFC 8252), the pure parts: PKCE, the
//! consent page's address, the redirect's check, and reading what the
//! token endpoint answers. `connect.rs` does the talking.

use std::fmt;

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine as _;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::net;
use crate::secrets::Secret;

pub const AUTH_ENDPOINT: &str = "https://accounts.google.com/o/oauth2/v2/auth";
pub const TOKEN_HOST: &str = "oauth2.googleapis.com";
pub const TOKEN_PATH: &str = "/token";
pub const REVOKE_PATH: &str = "/revoke";

pub const CALENDAR_SCOPE: &str = "https://www.googleapis.com/auth/calendar.events.readonly";
/// Subjects, senders and dates; never message bodies.
pub const GMAIL_SCOPE: &str = "https://www.googleapis.com/auth/gmail.metadata";
pub const DRIVE_SCOPE: &str = "https://www.googleapis.com/auth/drive.metadata.readonly";
/// Read-only access, plus `openid email` for "connected as …".
pub const SCOPES: [&str; 5] = ["openid", "email", CALENDAR_SCOPE, GMAIL_SCOPE, DRIVE_SCOPE];

/// An access token is refreshed this long before Google says it expires,
/// so a request never starts with one that runs out on the way.
pub const EXPIRY_MARGIN_SECS: i64 = 60;

/// Proof Key for Code Exchange: the verifier stays here, its SHA-256
/// (the challenge) goes in the consent page's address, and the token
/// request must show the verifier. A code caught on its way back to us is
/// useless without it.
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
}

impl Pkce {
    /// From 32 random bytes: a 43-character verifier (RFC 7636 allows
    /// 43 to 128) and its S256 challenge.
    pub fn from_bytes(random: &[u8; 32]) -> Self {
        let verifier = URL_SAFE_NO_PAD.encode(random);
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        Self {
            verifier,
            challenge,
        }
    }

    pub fn generate() -> Result<Self, String> {
        let mut bytes = [0u8; 32];
        getrandom::fill(&mut bytes).map_err(|e| format!("no random numbers: {e}"))?;
        Ok(Self::from_bytes(&bytes))
    }
}

/// Never shows the verifier.
impl fmt::Debug for Pkce {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Pkce(***)")
    }
}

/// A random value the consent page sends back unchanged, so a redirect
/// that some other page made up is refused.
pub fn random_state() -> Result<String, String> {
    let mut bytes = [0u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| format!("no random numbers: {e}"))?;
    Ok(URL_SAFE_NO_PAD.encode(bytes))
}

/// The consent page. `prompt=consent` makes Google hand out a refresh
/// token every time, even when the account already allowed this client.
pub fn authorization_url(client_id: &str, redirect_uri: &str, challenge: &str, state: &str) -> String {
    let scope = SCOPES.join(" ");
    let query = net::form(&[
        ("client_id", client_id),
        ("redirect_uri", redirect_uri),
        ("response_type", "code"),
        ("scope", &scope),
        ("code_challenge", challenge),
        ("code_challenge_method", "S256"),
        ("state", state),
        ("access_type", "offline"),
        ("prompt", "consent"),
    ]);
    format!("{AUTH_ENDPOINT}?{query}")
}

/// What came back to the loopback address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Callback {
    /// The authorization code, to trade for tokens.
    Code(String),
    /// You said no, or Google refused (`access_denied`, ...).
    Refused(String),
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CallbackError {
    /// Not from the consent page we opened (or an old tab).
    #[error("the answer didn't come from this sign-in (its state didn't match)")]
    WrongState,
    /// Some other request (a browser asking for an icon).
    #[error("not a sign-in answer")]
    NotACallback,
}

/// Reads the redirect's query string (`state=…&code=…`). The state must
/// match before anything else is believed.
pub fn parse_callback(query: &str, expected_state: &str) -> Result<Callback, CallbackError> {
    let mut state = None;
    let mut code = None;
    let mut error = None;
    for (name, value) in parse_query(query) {
        match name.as_str() {
            "state" => state = Some(value),
            "code" => code = Some(value),
            "error" => error = Some(value),
            _ => {}
        }
    }
    if state.is_none() && code.is_none() && error.is_none() {
        return Err(CallbackError::NotACallback);
    }
    if !state.is_some_and(|s| same(&s, expected_state)) {
        return Err(CallbackError::WrongState);
    }
    match (code, error) {
        (_, Some(error)) => Ok(Callback::Refused(error)),
        (Some(code), None) if !code.is_empty() => Ok(Callback::Code(code)),
        _ => Err(CallbackError::NotACallback),
    }
}

/// Compares without stopping at the first difference.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// `a=1&b=x%2Fy` → pairs, percent- and plus-decoded.
fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .trim_start_matches('?')
        .split('&')
        .filter(|part| !part.is_empty())
        .map(|part| {
            let (name, value) = part.split_once('=').unwrap_or((part, ""));
            (decode(name), decode(value))
        })
        .collect()
}

fn decode(text: &str) -> String {
    let hex = |b: u8| (b as char).to_digit(16);
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let escaped = (bytes[i] == b'%')
            .then(|| Some(hex(*bytes.get(i + 1)?)? * 16 + hex(*bytes.get(i + 2)?)?))
            .flatten();
        match (bytes[i], escaped) {
            (_, Some(value)) => {
                out.push(value as u8);
                i += 3;
            }
            (b'+', None) => {
                out.push(b' ');
                i += 1;
            }
            (other, None) => {
                out.push(other);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The form that trades the code for tokens.
pub fn code_exchange_form(
    code: &str,
    client_id: &str,
    client_secret: &Secret,
    redirect_uri: &str,
    verifier: &str,
) -> String {
    net::form(&[
        ("code", code),
        ("client_id", client_id),
        ("client_secret", client_secret.expose()),
        ("redirect_uri", redirect_uri),
        ("grant_type", "authorization_code"),
        ("code_verifier", verifier),
    ])
}

/// The form that gets a new access token with the refresh token.
pub fn refresh_form(refresh_token: &Secret, client_id: &str, client_secret: &Secret) -> String {
    net::form(&[
        ("refresh_token", refresh_token.expose()),
        ("client_id", client_id),
        ("client_secret", client_secret.expose()),
        ("grant_type", "refresh_token"),
    ])
}

/// What the token endpoint gave.
#[derive(Debug, Clone, PartialEq)]
pub struct Tokens {
    pub access: Secret,
    /// Unix seconds.
    pub expires_at: i64,
    /// Only on the first exchange (and not always on a refresh).
    pub refresh: Option<Secret>,
    /// From the ID token (`openid email`).
    pub email: Option<String>,
    /// What you allowed; you can untick some scopes on the consent page.
    pub scope: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TokenError {
    /// Google said no (`invalid_grant`, `invalid_client`, ...).
    #[error("Google refused: {}", describe_refusal(.error, .description.as_deref()))]
    Refused {
        error: String,
        description: Option<String>,
    },
    #[error("Google's answer couldn't be read ({0})")]
    Unreadable(String),
}

impl TokenError {
    /// The refresh token is dead (revoked, expired, or the password
    /// changed): only connecting again helps.
    pub fn needs_reconnect(&self) -> bool {
        matches!(self, Self::Refused { error, .. }
            if matches!(error.as_str(), "invalid_grant" | "invalid_client" | "unauthorized_client"))
    }
}

fn describe_refusal(error: &str, description: Option<&str>) -> String {
    match error {
        "invalid_grant" => {
            "the sign-in expired or was revoked. Connect again (in testing mode Google ends it after 7 days)".into()
        }
        "invalid_client" | "unauthorized_client" => {
            "the client ID or secret is wrong. Check them in Settings → Google".into()
        }
        _ => match description {
            Some(d) if !d.is_empty() => format!("{d} ({error})"),
            _ => error.to_owned(),
        },
    }
}

#[derive(Deserialize)]
struct TokenJson {
    access_token: String,
    expires_in: Option<i64>,
    refresh_token: Option<String>,
    id_token: Option<String>,
    scope: Option<String>,
}

#[derive(Deserialize)]
struct ErrorJson {
    error: String,
    error_description: Option<String>,
}

/// Reads the token endpoint's answer (`now` in Unix seconds).
pub fn parse_tokens(status: u16, body: &[u8], now: i64) -> Result<Tokens, TokenError> {
    if !(200..300).contains(&status) {
        return Err(match serde_json::from_slice::<ErrorJson>(body) {
            Ok(e) => TokenError::Refused {
                error: e.error,
                description: e.error_description,
            },
            Err(_) => TokenError::Unreadable(format!("HTTP {status}")),
        });
    }
    let json: TokenJson =
        serde_json::from_slice(body).map_err(|e| TokenError::Unreadable(e.to_string()))?;
    if json.access_token.is_empty() {
        return Err(TokenError::Unreadable("no access token".into()));
    }
    Ok(Tokens {
        access: Secret::new(json.access_token),
        // Google's tokens last an hour; assume so if it doesn't say.
        expires_at: now + json.expires_in.unwrap_or(3600).max(0),
        refresh: json
            .refresh_token
            .filter(|t| !t.is_empty())
            .map(Secret::new),
        email: json.id_token.as_deref().and_then(email_from_id_token),
        scope: json.scope,
    })
}

/// Whether an access token expiring at `expires_at` should be renewed now.
pub fn needs_refresh(expires_at: i64, now: i64) -> bool {
    now + EXPIRY_MARGIN_SECS >= expires_at
}

/// The `email` claim of an ID token. It came straight from Google's token
/// endpoint over TLS, so its signature needn't be checked (OpenID Connect
/// Core 3.1.3.7); it's only shown, never trusted for anything.
pub fn email_from_id_token(token: &str) -> Option<String> {
    let payload = token.split('.').nth(1)?;
    let bytes = URL_SAFE_NO_PAD.decode(payload.trim_end_matches('=')).ok()?;
    let claims: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    claims
        .get("email")?
        .as_str()
        .filter(|e| !e.is_empty())
        .map(str::to_owned)
}

/// Which parts you allowed on the consent page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Granted {
    pub calendar: bool,
    pub mail: bool,
    pub drive: bool,
}

impl Granted {
    /// From the token's `scope` (space separated). Without one, Google
    /// granted what was asked.
    pub fn from_scope(scope: Option<&str>) -> Self {
        let Some(scope) = scope else {
            return Self {
                calendar: true,
                mail: true,
                drive: true,
            };
        };
        let has = |wanted: &str| scope.split_whitespace().any(|s| s == wanted);
        Self {
            // The broader calendar scopes cover events too.
            calendar: has(CALENDAR_SCOPE)
                || has("https://www.googleapis.com/auth/calendar.readonly")
                || has("https://www.googleapis.com/auth/calendar"),
            mail: has(GMAIL_SCOPE),
            drive: has(DRIVE_SCOPE),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkce_matches_the_rfc_example() {
        // RFC 7636 appendix B.
        let bytes: [u8; 32] = [
            116, 24, 223, 180, 151, 153, 224, 37, 79, 250, 96, 125, 216, 173, 187, 186, 22, 212,
            37, 77, 105, 214, 191, 240, 91, 88, 5, 88, 83, 132, 141, 121,
        ];
        let pkce = Pkce::from_bytes(&bytes);
        assert_eq!(pkce.verifier, "dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk");
        assert_eq!(pkce.challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
        assert_eq!(format!("{pkce:?}"), "Pkce(***)");
        let fresh = Pkce::generate().unwrap();
        assert_eq!(fresh.verifier.len(), 43);
        assert_ne!(fresh.verifier, Pkce::generate().unwrap().verifier);
        assert_eq!(random_state().unwrap().len(), 22);
    }

    #[test]
    fn the_consent_page_address() {
        let url = authorization_url(
            "123-abc.apps.googleusercontent.com",
            "http://127.0.0.1:51234",
            "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "st4te",
        );
        assert!(url.starts_with("https://accounts.google.com/o/oauth2/v2/auth?client_id=123-abc.apps.googleusercontent.com&"));
        for part in [
            "redirect_uri=http%3A%2F%2F127.0.0.1%3A51234",
            "response_type=code",
            "scope=openid%20email%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcalendar.events.readonly%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fgmail.metadata%20https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fdrive.metadata.readonly",
            "code_challenge=E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM",
            "code_challenge_method=S256",
            "state=st4te",
            "access_type=offline",
            "prompt=consent",
        ] {
            assert!(url.contains(part), "{part} missing from {url}");
        }
    }

    #[test]
    fn the_redirect_must_carry_our_state() {
        assert_eq!(
            parse_callback("state=abc&code=4%2F0AQ-x_y&scope=email", "abc"),
            Ok(Callback::Code("4/0AQ-x_y".into()))
        );
        assert_eq!(
            parse_callback("?error=access_denied&state=abc", "abc"),
            Ok(Callback::Refused("access_denied".into()))
        );
        assert_eq!(
            parse_callback("state=abd&code=4%2F0", "abc"),
            Err(CallbackError::WrongState)
        );
        assert_eq!(
            parse_callback("code=4%2F0", "abc"),
            Err(CallbackError::WrongState)
        );
        assert_eq!(
            parse_callback("state=ab&code=x", "abc"),
            Err(CallbackError::WrongState)
        );
        assert_eq!(parse_callback("", "abc"), Err(CallbackError::NotACallback));
        assert_eq!(
            parse_callback("state=abc&code=", "abc"),
            Err(CallbackError::NotACallback)
        );
    }

    #[test]
    fn decodes_query_values() {
        assert_eq!(decode("a%20b+c%2F"), "a b c/");
        assert_eq!(decode("100%"), "100%");
        assert_eq!(decode("%zz"), "%zz");
        assert_eq!(decode("%C3%A9"), "é");
        assert_eq!(decode("%é%4"), "%é%4");
    }

    #[test]
    fn token_forms() {
        let form = code_exchange_form(
            "4/0AQ",
            "id.apps",
            &Secret::new("GOCSPX-s"),
            "http://127.0.0.1:5",
            "verif",
        );
        assert_eq!(
            form,
            "code=4%2F0AQ&client_id=id.apps&client_secret=GOCSPX-s&redirect_uri=http%3A%2F%2F127.0.0.1%3A5&grant_type=authorization_code&code_verifier=verif"
        );
        assert_eq!(
            refresh_form(&Secret::new("1//r"), "id", &Secret::new("s")),
            "refresh_token=1%2F%2Fr&client_id=id&client_secret=s&grant_type=refresh_token"
        );
    }

    /// An ID token with `{"email":"me@example.com"}` as its claims.
    fn id_token() -> String {
        let claims = URL_SAFE_NO_PAD.encode(br#"{"iss":"https://accounts.google.com","email":"me@example.com","email_verified":true}"#);
        format!("eyJhbGciOiJSUzI1NiJ9.{claims}.c2ln")
    }

    #[test]
    fn reads_a_first_exchange() {
        let body = format!(
            r#"{{"access_token":"ya29.a0","expires_in":3599,"refresh_token":"1//0g","scope":"openid {CALENDAR_SCOPE} email","token_type":"Bearer","id_token":"{}"}}"#,
            id_token()
        );
        let tokens = parse_tokens(200, body.as_bytes(), 1_000).unwrap();
        assert_eq!(tokens.access.expose(), "ya29.a0");
        assert_eq!(tokens.expires_at, 4_599);
        assert_eq!(tokens.refresh.as_ref().map(Secret::expose), Some("1//0g"));
        assert_eq!(tokens.email.as_deref(), Some("me@example.com"));
        let granted = Granted::from_scope(tokens.scope.as_deref());
        assert_eq!(
            granted,
            Granted {
                calendar: true,
                mail: false,
                drive: false
            }
        );
        // Tokens never show up in logs.
        assert!(!format!("{tokens:?}").contains("ya29"));
    }

    #[test]
    fn reads_a_refresh_and_errors() {
        let tokens = parse_tokens(200, br#"{"access_token":"ya29.b","expires_in":3599}"#, 0).unwrap();
        assert!(tokens.refresh.is_none() && tokens.email.is_none());
        assert_eq!(Granted::from_scope(None), Granted { calendar: true, mail: true, drive: true });

        let dead = parse_tokens(
            400,
            br#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#,
            0,
        )
        .unwrap_err();
        assert!(dead.needs_reconnect());
        assert!(dead.to_string().contains("Connect again"), "{dead}");
        let other = parse_tokens(400, br#"{"error":"invalid_request","error_description":"Missing code verifier."}"#, 0).unwrap_err();
        assert!(!other.needs_reconnect());
        assert_eq!(other.to_string(), "Google refused: Missing code verifier. (invalid_request)");
        assert!(matches!(parse_tokens(502, b"<html>", 0), Err(TokenError::Unreadable(_))));
        assert!(matches!(parse_tokens(200, b"{}", 0), Err(TokenError::Unreadable(_))));
        assert!(parse_tokens(401, br#"{"error":"invalid_client"}"#, 0).unwrap_err().needs_reconnect());
    }

    #[test]
    fn refreshes_a_minute_early() {
        assert!(!needs_refresh(1_000, 900));
        assert!(needs_refresh(1_000, 940));
        assert!(needs_refresh(1_000, 2_000));
    }

    #[test]
    fn email_from_odd_tokens() {
        assert_eq!(email_from_id_token(&id_token()).as_deref(), Some("me@example.com"));
        assert_eq!(email_from_id_token("not-a-jwt"), None);
        assert_eq!(email_from_id_token("a.!!!.c"), None);
        let no_email = URL_SAFE_NO_PAD.encode(br#"{"sub":"1"}"#);
        assert_eq!(email_from_id_token(&format!("a.{no_email}.c")), None);
    }
}
