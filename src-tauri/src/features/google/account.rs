//! Your Google connection on this PC. The client secret and the refresh
//! token live in Credential Manager (`desktop-assistant:google-*`); who
//! you are and what you allowed sit in `google-account.json` in the local
//! data folder; access tokens only ever live in memory.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, RwLock};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use super::api::ApiError;
use super::oauth::{self, Granted, Tokens};
use crate::net::{self, Request};
use crate::secrets::{self, Secret};
use crate::settings::{self, SharedSettings};

/// Credential Manager names (`desktop-assistant:` goes in front).
pub const CLIENT_SECRET: &str = "google-client-secret";
pub const REFRESH_TOKEN: &str = "google-refresh-token";
const ACCOUNT_FILE: &str = "google-account.json";

/// Who's connected; written once connecting worked.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AccountInfo {
    pub email: Option<String>,
    /// The OAuth client the refresh token belongs to: if the client ID in
    /// the settings changes, the token is no use.
    pub client_id: String,
    /// What you allowed on the consent page.
    pub scope: Option<String>,
}

/// What Settings → Google shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct GoogleStatus {
    /// The client secret is saved on this PC.
    pub secret_saved: bool,
    /// Waiting for you to approve in the browser.
    pub connecting: bool,
    pub connected: bool,
    pub email: Option<String>,
    /// The last thing that went wrong (connecting, or reading calendar,
    /// mail or Drive), if it still stands.
    pub problem: Option<String>,
}

struct AccessToken {
    token: Secret,
    expires_at: i64,
}

pub struct Account {
    file: PathBuf,
    settings: SharedSettings,
    /// Set while a refresh token is saved.
    info: RwLock<Option<AccountInfo>>,
    /// Locked across a refresh, so only one runs at a time.
    access: tokio::sync::Mutex<Option<AccessToken>>,
    problem: Mutex<Option<String>>,
    connecting: AtomicBool,
}

pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs() as i64)
}

fn read_lock<T: Clone>(lock: &RwLock<T>) -> T {
    match lock.read() {
        Ok(v) => v.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

fn write_lock<T>(lock: &RwLock<T>, value: T) {
    match lock.write() {
        Ok(mut v) => *v = value,
        Err(poisoned) => *poisoned.into_inner() = value,
    }
}

impl Account {
    /// The connection saved in `data_dir`, if its refresh token is still
    /// in Credential Manager.
    pub fn load(data_dir: &Path, settings: SharedSettings) -> Self {
        let file = data_dir.join(ACCOUNT_FILE);
        let info = std::fs::read(&file)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<AccountInfo>(&bytes).ok())
            .filter(|_| secrets::read(REFRESH_TOKEN).is_some());
        Self {
            file,
            settings,
            info: RwLock::new(info),
            access: tokio::sync::Mutex::new(None),
            problem: Mutex::new(None),
            connecting: AtomicBool::new(false),
        }
    }

    /// The client ID from the settings.
    pub fn client_id(&self) -> String {
        settings::current(&self.settings)
            .google
            .client_id
            .trim()
            .to_owned()
    }

    /// The connection, if it's for the client in the settings.
    pub fn info(&self) -> Option<AccountInfo> {
        read_lock(&self.info).filter(|i| i.client_id == self.client_id())
    }

    pub fn connected(&self) -> bool {
        self.info().is_some()
    }

    pub fn email(&self) -> Option<String> {
        self.info().and_then(|i| i.email)
    }

    pub fn granted(&self) -> Granted {
        self.info()
            .map(|i| Granted::from_scope(i.scope.as_deref()))
            .unwrap_or_default()
    }

    pub fn set_connecting(&self, on: bool) {
        self.connecting.store(on, Ordering::Relaxed);
    }

    /// Remembers (or clears) what went wrong. Returns whether it changed.
    pub fn set_problem(&self, problem: Option<String>) -> bool {
        let mut current = self.problem.lock().unwrap_or_else(|p| p.into_inner());
        let changed = *current != problem;
        *current = problem;
        changed
    }

    pub fn status(&self) -> GoogleStatus {
        let info = self.info();
        let stale = info.is_none() && read_lock(&self.info).is_some();
        let problem = self
            .problem
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
            .or_else(|| stale.then(|| "The client ID changed. Connect again.".to_owned()));
        GoogleStatus {
            secret_saved: secrets::read(CLIENT_SECRET).is_some(),
            connecting: self.connecting.load(Ordering::Relaxed),
            connected: info.is_some(),
            email: info.and_then(|i| i.email),
            problem,
        }
    }

    /// The client secret, for the token endpoint.
    pub fn client_secret(&self) -> Result<Secret, ApiError> {
        secrets::get(CLIENT_SECRET)
            .map_err(|e| ApiError::SignIn(e.to_string()))?
            .ok_or_else(|| ApiError::Reconnect("the client secret isn't saved on this PC".into()))
    }

    /// An access token good for at least a minute: the one in memory, or
    /// a new one from the refresh token. `force` skips the one in memory
    /// (Google just refused it).
    pub async fn access_token(&self, force: bool) -> Result<Secret, ApiError> {
        let mut access = self.access.lock().await;
        if let Some(current) = access.as_ref() {
            if !force && !oauth::needs_refresh(current.expires_at, unix_now()) {
                return Ok(current.token.clone());
            }
        }
        let Some(info) = self.info() else {
            return Err(ApiError::NotConnected);
        };
        let refresh = secrets::get(REFRESH_TOKEN)
            .map_err(|e| ApiError::SignIn(e.to_string()))?
            .ok_or(ApiError::NotConnected)?;
        let form = oauth::refresh_form(&refresh, &info.client_id, &self.client_secret()?);
        let response = net::fetch(Request::post_form(oauth::TOKEN_HOST, oauth::TOKEN_PATH, form))
            .await
            .map_err(|e| ApiError::SignIn(e.to_string()))?;
        let tokens = match oauth::parse_tokens(response.status, &response.body, unix_now()) {
            Ok(tokens) => tokens,
            Err(e) if e.needs_reconnect() => {
                tracing::warn!("Google refused the refresh token: {e}");
                *access = None;
                self.forget();
                let message = e.to_string();
                self.set_problem(Some(message.clone()));
                return Err(ApiError::Reconnect(message));
            }
            Err(e) => return Err(ApiError::SignIn(e.to_string())),
        };
        if let Some(rotated) = &tokens.refresh {
            if let Err(e) = secrets::set(REFRESH_TOKEN, rotated) {
                tracing::warn!("could not save Google's new refresh token: {e}");
            }
        }
        *access = Some(AccessToken {
            token: tokens.access.clone(),
            expires_at: tokens.expires_at,
        });
        Ok(tokens.access)
    }

    /// Saves a new connection: the refresh token in Credential Manager,
    /// the rest in the account file.
    pub async fn save_connection(&self, tokens: &Tokens, client_id: &str) -> Result<AccountInfo, String> {
        let refresh = tokens.refresh.as_ref().ok_or(
            "Google didn't hand out a refresh token. Remove the launcher's access at myaccount.google.com/permissions and connect again",
        )?;
        secrets::set(REFRESH_TOKEN, refresh).map_err(|e| e.to_string())?;
        let info = AccountInfo {
            email: tokens.email.clone(),
            client_id: client_id.to_owned(),
            scope: tokens.scope.clone(),
        };
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let json = serde_json::to_vec_pretty(&info).map_err(|e| e.to_string())?;
        std::fs::write(&self.file, json)
            .map_err(|e| format!("could not write {}: {e}", self.file.display()))?;
        *self.access.lock().await = Some(AccessToken {
            token: tokens.access.clone(),
            expires_at: tokens.expires_at,
        });
        write_lock(&self.info, Some(info.clone()));
        self.set_problem(None);
        Ok(info)
    }

    /// Drops the connection on this PC (the refresh token and the account
    /// file); the client secret stays.
    pub fn forget(&self) {
        if let Err(e) = secrets::delete(REFRESH_TOKEN) {
            tracing::warn!("could not remove the Google refresh token: {e}");
        }
        if self.file.exists() {
            if let Err(e) = std::fs::remove_file(&self.file) {
                tracing::warn!("could not remove {}: {e}", self.file.display());
            }
        }
        write_lock(&self.info, None);
    }

    /// Disconnect: asks Google to revoke the refresh token (which also ends
    /// its access tokens), then deletes it and the client secret here.
    /// Revoking is best effort: offline, the token is still forgotten.
    pub async fn disconnect(&self) {
        if let Some(refresh) = secrets::read(REFRESH_TOKEN) {
            let form = net::form(&[("token", refresh.expose())]);
            match net::fetch(Request::post_form(oauth::TOKEN_HOST, oauth::REVOKE_PATH, form)).await {
                Ok(r) if r.ok() => tracing::info!("revoked the Google sign-in"),
                // 400 `invalid_token`: it was already dead.
                Ok(r) => tracing::info!("Google answered {} to the revoke", r.status),
                Err(e) => tracing::warn!("could not revoke the Google sign-in: {e}"),
            }
        }
        *self.access.lock().await = None;
        self.forget();
        if let Err(e) = secrets::delete(CLIENT_SECRET) {
            tracing::warn!("could not remove the Google client secret: {e}");
        }
        self.set_problem(None);
    }

    #[cfg(test)]
    pub fn set_info(&self, info: Option<AccountInfo>) {
        write_lock(&self.info, info);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::sync::Arc;

    fn settings(client_id: &str) -> SharedSettings {
        let mut s = Settings::default();
        s.google.client_id = client_id.into();
        Arc::new(RwLock::new(s))
    }

    fn tokens(refresh: Option<&str>) -> Tokens {
        Tokens {
            access: Secret::new("ya29.test"),
            expires_at: unix_now() + 3600,
            refresh: refresh.map(Secret::new),
            email: Some("me@example.com".into()),
            scope: Some(oauth::SCOPES.join(" ")),
        }
    }

    // One test, since it uses the process-wide (in-memory) secret store.
    #[tokio::test]
    async fn connect_status_and_disconnect() {
        let dir = tempfile::tempdir().unwrap();
        let shared = settings(" id-1.apps ");
        let account = Account::load(dir.path(), Arc::clone(&shared));
        assert!(!account.connected());
        assert_eq!(account.client_id(), "id-1.apps");
        assert_eq!(account.access_token(false).await, Err(ApiError::NotConnected));

        // No refresh token: refused, nothing saved.
        assert!(account.save_connection(&tokens(None), "id-1.apps").await.is_err());
        assert!(!account.connected());

        secrets::set(CLIENT_SECRET, &Secret::new("GOCSPX-test")).unwrap();
        let info = account
            .save_connection(&tokens(Some("1//refresh")), "id-1.apps")
            .await
            .unwrap();
        assert_eq!(info.email.as_deref(), Some("me@example.com"));
        let status = account.status();
        assert!(status.connected && status.secret_saved && !status.connecting);
        assert_eq!(status.email.as_deref(), Some("me@example.com"));
        assert!(account.granted().mail);
        // The fresh access token is used without a request.
        assert_eq!(account.access_token(false).await.unwrap().expose(), "ya29.test");
        // A restart finds the connection again.
        let reloaded = Account::load(dir.path(), Arc::clone(&shared));
        assert_eq!(reloaded.email().as_deref(), Some("me@example.com"));

        // Another client ID in the settings: the token is no use.
        shared.write().unwrap().google.client_id = "id-2.apps".into();
        assert!(!account.connected());
        assert_eq!(
            account.status().problem.as_deref(),
            Some("The client ID changed. Connect again.")
        );
        shared.write().unwrap().google.client_id = "id-1.apps".into();

        account.disconnect().await;
        assert!(!account.connected());
        assert!(secrets::read(REFRESH_TOKEN).is_none());
        assert!(secrets::read(CLIENT_SECRET).is_none());
        assert!(!dir.path().join(ACCOUNT_FILE).exists());
        assert_eq!(account.status().problem, None);
        assert!(Account::load(dir.path(), shared).info().is_none());
    }
}
