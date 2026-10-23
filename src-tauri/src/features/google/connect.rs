//! "Connect Google account": the consent page in your browser, the
//! redirect back to a one-shot listener, and the code traded for tokens
//! (Authorization Code + PKCE, RFC 8252).

use std::sync::Arc;
use std::time::Duration;

use tauri::AppHandle;
use tauri_plugin_opener::OpenerExt;

use super::account::{unix_now, AccountInfo, CLIENT_SECRET};
use super::hub::Google;
use super::loopback::Loopback;
use super::oauth::{self, Callback, Pkce};
use crate::net::{self, Request};
use crate::secrets::{self, Secret};
use crate::toast;

/// How long the consent page may stay open.
pub const TIMEOUT: Duration = Duration::from_secs(5 * 60);

/// Opens the consent page and finishes in the background; the result
/// arrives as a status event and a toast. A sign-in already waiting is
/// replaced.
pub async fn start(google: Arc<Google>, app: AppHandle) -> Result<(), String> {
    let client_id = google.account.client_id();
    if client_id.is_empty() {
        return Err("Enter the client ID first.".into());
    }
    let secret = secrets::read(CLIENT_SECRET).ok_or("Save the client secret first.")?;
    let pkce = Pkce::generate()?;
    let state = oauth::random_state()?;
    let loopback = Loopback::bind()
        .await
        .map_err(|e| format!("couldn't listen for Google's answer: {e}"))?;
    let redirect = loopback.redirect_uri();
    let url = oauth::authorization_url(&client_id, &redirect, &pkce.challenge, &state);
    app.opener()
        .open_url(&url, None::<&str>)
        .map_err(|e| format!("couldn't open the browser: {e}"))?;
    tracing::info!("opened Google's consent page; waiting on {redirect}");
    google.account.set_connecting(true);
    google.account.set_problem(None);
    google.announce();

    let task_google = Arc::clone(&google);
    let task = tauri::async_runtime::spawn(async move {
        let google = task_google;
        let result = finish(
            &google, loopback, &state, &pkce, &client_id, &secret, &redirect,
        )
        .await;
        google.account.set_connecting(false);
        match result {
            Ok(info) => {
                let who = info.email.as_deref().unwrap_or("your account");
                tracing::info!("connected to Google");
                toast::info(&app, format!("Connected to Google as {who}."));
                google.clear();
                google.cache.refresh_now();
            }
            Err(e) => {
                tracing::warn!("connecting to Google failed: {e}");
                google.account.set_problem(Some(sentence(&e)));
                toast::error(&app, format!("Couldn't connect to Google: {e}"));
            }
        }
        google.announce();
    });
    google.replace_connecting(Some(task));
    Ok(())
}

/// Waits for the redirect, then trades its code for tokens and saves them.
async fn finish(
    google: &Google,
    loopback: Loopback,
    state: &str,
    pkce: &Pkce,
    client_id: &str,
    secret: &Secret,
    redirect: &str,
) -> Result<AccountInfo, String> {
    let code = match loopback
        .wait(state, TIMEOUT)
        .await
        .map_err(|e| e.to_string())?
    {
        Callback::Code(code) => code,
        Callback::Refused(error) if error == "access_denied" => {
            return Err("access wasn't allowed in the browser".into())
        }
        Callback::Refused(error) => return Err(format!("Google refused ({error})")),
    };
    let form = oauth::code_exchange_form(&code, client_id, secret, redirect, &pkce.verifier);
    let response = net::fetch(Request::post_form(
        oauth::TOKEN_HOST,
        oauth::TOKEN_PATH,
        form,
    ))
    .await
    .map_err(|e| e.to_string())?;
    let tokens = oauth::parse_tokens(response.status, &response.body, unix_now())
        .map_err(|e| e.to_string())?;
    google.account.save_connection(&tokens, client_id).await
}

/// Ends a sign-in still waiting in the browser.
pub fn cancel(google: &Google) {
    google.replace_connecting(None);
    google.account.set_connecting(false);
}

fn sentence(text: &str) -> String {
    let mut chars = text.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}
