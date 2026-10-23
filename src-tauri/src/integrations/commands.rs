//! What the settings menu calls for API keys and MCP servers (registered
//! in `ipc/mod.rs` under `// [integrations]`). Secret values only go in:
//! replies say whether one is saved and, for a long one, its last four
//! characters.

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, State};

use super::mcp_edit::{self, SECRET_PREFIX};
use super::mcp_registry::{check_server, McpServer};
use crate::error::IpcError;
use crate::secrets::{self, Secret, ANTHROPIC_API_KEY, OPENAI_API_KEY};
use crate::settings::{self, Settings};
use crate::state::AppState;

/// Whether a secret is saved on this PC.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
pub struct SecretStatus {
    pub name: String,
    pub saved: bool,
    /// The last four characters of a long value, to tell keys apart.
    pub last4: Option<String>,
}

/// The API keys the menu sets and removes directly. MCP servers' secrets
/// change through `mcp_server_save` and `mcp_server_remove`.
const API_KEYS: [&str; 2] = [ANTHROPIC_API_KEY, OPENAI_API_KEY];

fn err(message: impl ToString) -> IpcError {
    IpcError {
        message: message.to_string(),
    }
}

fn api_key_name(name: &str) -> Result<&str, IpcError> {
    API_KEYS
        .into_iter()
        .find(|k| *k == name)
        .ok_or_else(|| err(format!("\"{name}\" isn't an API key the launcher keeps")))
}

/// Names the menu may ask about: the API keys and MCP servers' secrets.
fn may_ask_about(name: &str) -> bool {
    API_KEYS.contains(&name) || name.starts_with(SECRET_PREFIX)
}

fn status(name: &str) -> Result<SecretStatus, IpcError> {
    let saved = secrets::get(name).map_err(err)?;
    Ok(SecretStatus {
        name: name.to_owned(),
        saved: saved.is_some(),
        last4: saved.and_then(|s| s.last4()),
    })
}

/// Which of `names` are saved on this PC. One that can't be read (a name
/// from a hand-edited settings file that isn't valid) counts as not saved,
/// so the others still show.
#[tauri::command]
#[specta::specta]
pub async fn secret_status(names: Vec<String>) -> Result<Vec<SecretStatus>, IpcError> {
    names
        .iter()
        .map(|name| {
            if !may_ask_about(name) {
                return Err(err(format!("\"{name}\" isn't a secret the launcher keeps")));
            }
            Ok(status(name).unwrap_or_else(|e| {
                tracing::warn!("could not check the secret {name}: {}", e.message);
                SecretStatus {
                    name: name.clone(),
                    saved: false,
                    last4: None,
                }
            }))
        })
        .collect()
}

/// Saves an API key in Credential Manager (replacing any saved one).
#[tauri::command]
#[specta::specta]
pub async fn secret_set(name: String, value: String) -> Result<SecretStatus, IpcError> {
    let name = api_key_name(&name)?;
    let value = value.trim();
    if value.is_empty() {
        return Err(err("Paste the key first."));
    }
    secrets::set(name, &Secret::new(value)).map_err(err)?;
    tracing::info!("saved the secret {name}");
    status(name)
}

/// Forgets an API key.
#[tauri::command]
#[specta::specta]
pub async fn secret_delete(name: String) -> Result<SecretStatus, IpcError> {
    let name = api_key_name(&name)?;
    secrets::delete(name).map_err(err)?;
    tracing::info!("removed the secret {name}");
    status(name)
}

/// A server added or changed in the menu.
#[derive(Debug, Clone, Deserialize, specta::Type)]
pub struct McpServerEdit {
    /// Its name before this edit; `None` for a new server.
    pub previous: Option<String>,
    /// A secret entry's `value` is the new value to save, or empty to keep
    /// the saved one.
    pub server: McpServer,
}

/// Adds or changes a server: checks it, saves its secrets in Credential
/// Manager and the rest in the settings. Returns the settings now in use.
#[tauri::command]
#[specta::specta]
pub async fn mcp_server_save(
    app: AppHandle,
    state: State<'_, AppState>,
    edit: McpServerEdit,
) -> Result<Settings, IpcError> {
    let old = state.settings();
    let servers = &old.mcp.servers;
    let index = match &edit.previous {
        Some(previous) => Some(
            servers
                .iter()
                .position(|s| &s.name == previous)
                .ok_or_else(|| err(format!("\"{previous}\" isn't in the list any more.")))?,
        ),
        None => None,
    };
    let next = edit.server.normalized();
    let others = servers
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i) != index)
        .map(|(_, s)| s);
    check_server(&next, others).map_err(err)?;

    let plan = mcp_edit::plan_save(index.map(|i| &servers[i]), &next);
    mcp_edit::apply_writes(&plan).map_err(err)?;
    let mut new = old.clone();
    let saved = mcp_edit::without_secret_values(next);
    match index {
        Some(i) => new.mcp.servers[i] = saved,
        None => new.mcp.servers.push(saved),
    }
    settings::live::save(&app, &old, new.clone()).map_err(err)?;
    mcp_edit::apply_deletes(&plan);
    // A new secret value alone doesn't change the settings.
    super::refresh_claude_config(&state);
    Ok(new)
}

/// Removes a server and forgets its secrets. Returns the settings now in
/// use.
#[tauri::command]
#[specta::specta]
pub async fn mcp_server_remove(
    app: AppHandle,
    state: State<'_, AppState>,
    name: String,
) -> Result<Settings, IpcError> {
    let old = state.settings();
    let Some(index) = old.mcp.servers.iter().position(|s| s.name == name) else {
        return Ok(old);
    };
    let plan = mcp_edit::plan_remove(&old.mcp.servers[index]);
    let mut new = old.clone();
    new.mcp.servers.remove(index);
    settings::live::save(&app, &old, new.clone()).map_err(err)?;
    mcp_edit::apply_deletes(&plan);
    super::refresh_claude_config(&state);
    Ok(new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn api_keys_go_in_but_never_come_out() {
        let set = secret_set(OPENAI_API_KEY.into(), "  sk-proj-test-abcd1234  ".into())
            .await
            .unwrap();
        assert_eq!(
            set,
            SecretStatus {
                name: OPENAI_API_KEY.into(),
                saved: true,
                last4: Some("1234".into()),
            }
        );
        assert_eq!(
            secrets::get(OPENAI_API_KEY).unwrap().unwrap().expose(),
            "sk-proj-test-abcd1234",
            "trimmed"
        );
        let shown = serde_json::to_string(&set).unwrap();
        assert!(!shown.contains("sk-proj"), "{shown}");

        let listed = secret_status(vec![OPENAI_API_KEY.into(), "mcp:x:env:Y".into()])
            .await
            .unwrap();
        assert!(listed[0].saved && !listed[1].saved);
        let odd = secret_status(vec!["mcp:a b:env:X".into(), OPENAI_API_KEY.into()])
            .await
            .unwrap();
        assert!(
            !odd[0].saved && odd[1].saved,
            "a bad name doesn't hide the rest"
        );

        let gone = secret_delete(OPENAI_API_KEY.into()).await.unwrap();
        assert!(!gone.saved && gone.last4.is_none());
    }

    #[tokio::test]
    async fn only_known_names_are_accepted() {
        assert!(secret_set("mcp:x:env:Y".into(), "v".into()).await.is_err());
        assert!(secret_set("something-else".into(), "v".into())
            .await
            .is_err());
        assert!(secret_delete("other".into()).await.is_err());
        assert!(secret_status(vec!["other".into()]).await.is_err());
        assert!(secret_set(ANTHROPIC_API_KEY.into(), "   ".into())
            .await
            .is_err());
    }
}
