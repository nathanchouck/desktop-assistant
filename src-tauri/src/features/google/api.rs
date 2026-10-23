//! Calling Google's APIs with the account's access token, and making
//! sense of their errors (an API left off in the Cloud project, a scope
//! you didn't allow).

use std::sync::Arc;
use std::time::Duration;

use serde::Deserialize;

use super::account::Account;
use crate::net::{self, NetError, Request};

/// The APIs this feature reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Service {
    Calendar,
    Gmail,
    Drive,
}

impl Service {
    /// As the Cloud Console's API Library lists it.
    pub fn api_name(self) -> &'static str {
        match self {
            Self::Calendar => "Google Calendar API",
            Self::Gmail => "Gmail API",
            Self::Drive => "Google Drive API",
        }
    }

    /// As the consent page and the rows say it.
    pub fn short_name(self) -> &'static str {
        match self {
            Self::Calendar => "Calendar",
            Self::Gmail => "Gmail",
            Self::Drive => "Drive",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ApiError {
    #[error("Google isn't connected")]
    NotConnected,
    /// The sign-in is gone (revoked, expired, client changed).
    #[error("{0}")]
    Reconnect(String),
    /// Getting an access token failed some other way (offline...).
    #[error("couldn't sign in to Google: {0}")]
    SignIn(String),
    /// The API is off in the Cloud project; `link` turns it on.
    #[error("the {} is turned off in your Google Cloud project", .service.api_name())]
    Disabled {
        service: Service,
        link: Option<String>,
    },
    /// You unticked this part on the consent page.
    #[error("{} access wasn't allowed. Connect again and tick it", .0.short_name())]
    NotAllowed(Service),
    #[error("Google answered {status}: {message}")]
    Http { status: u16, message: String },
    #[error(transparent)]
    Net(#[from] NetError),
    #[error("Google's answer couldn't be read ({0})")]
    Unreadable(String),
}

/// A request that's interactive (Drive while you type) gives up sooner.
pub const QUICK_TIMEOUT: Duration = Duration::from_secs(8);

pub struct Api {
    account: Arc<Account>,
}

impl Api {
    pub fn new(account: Arc<Account>) -> Self {
        Self { account }
    }

    /// GETs `https://<host><path>` with the access token. An expired
    /// token (401) is refreshed and the request tried once more.
    pub async fn get(
        &self,
        service: Service,
        host: &str,
        path: &str,
        timeout: Duration,
    ) -> Result<Vec<u8>, ApiError> {
        let mut forced = false;
        loop {
            let token = self.account.access_token(forced).await?;
            let request = Request::get(host, path)
                .header("Authorization", &format!("Bearer {}", token.expose()))
                .timeout(timeout);
            let response = net::fetch(request).await?;
            if response.status == 401 && !forced {
                forced = true;
                continue;
            }
            if !response.ok() {
                return Err(error_from_response(
                    service,
                    response.status,
                    &response.body,
                ));
            }
            return Ok(response.body);
        }
    }
}

#[derive(Deserialize)]
struct ErrorBody {
    error: ErrorJson,
}

#[derive(Deserialize)]
struct ErrorJson {
    #[serde(default)]
    message: String,
    #[serde(default)]
    status: String,
    #[serde(default)]
    errors: Vec<ReasonJson>,
    #[serde(default)]
    details: Vec<DetailJson>,
}

#[derive(Deserialize)]
struct ReasonJson {
    #[serde(default)]
    reason: String,
}

#[derive(Deserialize)]
struct DetailJson {
    #[serde(default)]
    reason: String,
    #[serde(default)]
    metadata: Option<serde_json::Map<String, serde_json::Value>>,
}

/// Longest error text shown on a row or in Settings.
const MAX_MESSAGE: usize = 160;

/// What a failed API call means.
pub fn error_from_response(service: Service, status: u16, body: &[u8]) -> ApiError {
    let Ok(ErrorBody { error }) = serde_json::from_slice::<ErrorBody>(body) else {
        return ApiError::Http {
            status,
            message: "no details".into(),
        };
    };
    let reasons: Vec<&str> = error
        .errors
        .iter()
        .map(|e| e.reason.as_str())
        .chain(error.details.iter().map(|d| d.reason.as_str()))
        .collect();
    let disabled = reasons
        .iter()
        .any(|r| matches!(*r, "accessNotConfigured" | "SERVICE_DISABLED"))
        || error.message.contains("has not been used in project")
        || error.message.contains("it is disabled");
    if disabled {
        let link = error
            .details
            .iter()
            .filter_map(|d| d.metadata.as_ref()?.get("activationUrl")?.as_str())
            .map(str::to_owned)
            .chain(console_link(&error.message))
            .find(|l| l.starts_with("https://"));
        return ApiError::Disabled { service, link };
    }
    let scope = reasons.iter().any(|r| {
        matches!(
            *r,
            "insufficientPermissions" | "ACCESS_TOKEN_SCOPE_INSUFFICIENT"
        )
    }) || (error.status == "PERMISSION_DENIED" && error.message.contains("scopes"));
    if scope {
        return ApiError::NotAllowed(service);
    }
    let mut message = error.message;
    if message.chars().count() > MAX_MESSAGE {
        message = message.chars().take(MAX_MESSAGE).collect::<String>() + "…";
    }
    ApiError::Http { status, message }
}

/// The Cloud Console address in an error message, if any.
fn console_link(message: &str) -> Option<String> {
    let start = message.find("https://console.")?;
    let rest = &message[start..];
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].trim_end_matches(['.', ',']).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spots_an_api_thats_turned_off() {
        let body = br#"{"error":{"code":403,"message":"Gmail API has not been used in project 123 before or it is disabled. Enable it by visiting https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=123 then retry.","errors":[{"message":"...","domain":"usageLimits","reason":"accessNotConfigured"}],"status":"PERMISSION_DENIED","details":[{"@type":"type.googleapis.com/google.rpc.ErrorInfo","reason":"SERVICE_DISABLED","metadata":{"activationUrl":"https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=123","service":"gmail.googleapis.com"}}]}}"#;
        let error = error_from_response(Service::Gmail, 403, body);
        assert_eq!(
            error,
            ApiError::Disabled {
                service: Service::Gmail,
                link: Some("https://console.developers.google.com/apis/api/gmail.googleapis.com/overview?project=123".into())
            }
        );
        assert_eq!(
            error.to_string(),
            "the Gmail API is turned off in your Google Cloud project"
        );
        // Older answers only say so in the message.
        let old = br#"{"error":{"code":403,"message":"Google Drive API has not been used in project 9 before or it is disabled. Enable it by visiting https://console.developers.google.com/apis/api/drive.googleapis.com/overview?project=9 then retry."}}"#;
        assert_eq!(
            error_from_response(Service::Drive, 403, old),
            ApiError::Disabled {
                service: Service::Drive,
                link: Some("https://console.developers.google.com/apis/api/drive.googleapis.com/overview?project=9".into())
            }
        );
    }

    #[test]
    fn spots_a_missing_scope_and_other_errors() {
        let body = br#"{"error":{"code":403,"message":"Request had insufficient authentication scopes.","errors":[{"reason":"insufficientPermissions"}],"status":"PERMISSION_DENIED","details":[{"reason":"ACCESS_TOKEN_SCOPE_INSUFFICIENT"}]}}"#;
        let error = error_from_response(Service::Calendar, 403, body);
        assert_eq!(error, ApiError::NotAllowed(Service::Calendar));
        assert_eq!(
            error.to_string(),
            "Calendar access wasn't allowed. Connect again and tick it"
        );
        let busy =
            br#"{"error":{"code":429,"message":"Quota exceeded","status":"RESOURCE_EXHAUSTED"}}"#;
        assert_eq!(
            error_from_response(Service::Drive, 429, busy).to_string(),
            "Google answered 429: Quota exceeded"
        );
        let long = format!(
            r#"{{"error":{{"code":500,"message":"{}"}}}}"#,
            "x".repeat(400)
        );
        let ApiError::Http { message, .. } =
            error_from_response(Service::Drive, 500, long.as_bytes())
        else {
            panic!("expected Http");
        };
        assert_eq!(message.chars().count(), MAX_MESSAGE + 1);
        assert_eq!(
            error_from_response(Service::Drive, 502, b"<html>"),
            ApiError::Http {
                status: 502,
                message: "no details".into()
            }
        );
    }
}
