//! Saving a server from the settings menu: what changes in Credential
//! Manager. Secret values are named after their server and entry
//! (`mcp:<server>:env:<NAME>`, `mcp:<server>:header:<Name>`), so a rename
//! moves them and a removed entry or server forgets them. Planning is
//! pure; `apply` does it.

use std::collections::HashSet;

use super::mcp_registry::{McpServer, ValueKind};
use crate::secrets::{self, Secret, SecretError};

/// What goes in front of every registry secret's name.
pub const SECRET_PREFIX: &str = "mcp:";

/// The Credential Manager name of a server's secret value.
pub fn secret_name(server: &str, kind: ValueKind, entry: &str) -> String {
    format!("{SECRET_PREFIX}{server}:{}:{entry}", kind.as_str())
}

/// The names of `server`'s secret values.
fn secret_names(server: &McpServer) -> Vec<String> {
    let (kind, values) = server.values();
    values
        .iter()
        .filter(|v| v.secret)
        .map(|v| secret_name(&server.name, kind, &v.name))
        .collect()
}

/// What a save does to secrets, in order: write new values, move kept
/// ones to their new names, then (once the settings are saved) delete
/// names nothing uses any more.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct SecretPlan {
    pub write: Vec<(String, Secret)>,
    /// (from, to)
    pub moves: Vec<(String, String)>,
    pub delete: Vec<String>,
}

/// Saving `next` (normalized), which was `previous` (`None` for a new
/// server). A secret entry with a value gets that value; one left empty
/// keeps the saved value, which follows a renamed server.
pub fn plan_save(previous: Option<&McpServer>, next: &McpServer) -> SecretPlan {
    let mut plan = SecretPlan::default();
    let (kind, values) = next.values();
    let kept: HashSet<String> = secret_names(next).into_iter().collect();
    for value in values.iter().filter(|v| v.secret) {
        let name = secret_name(&next.name, kind, &value.name);
        if !value.value.is_empty() {
            plan.write.push((name, Secret::new(value.value.clone())));
            continue;
        }
        let Some(previous) = previous.filter(|p| p.name != next.name) else {
            continue;
        };
        let (old_kind, old_values) = previous.values();
        let had_it = old_kind == kind
            && old_values
                .iter()
                .any(|old| old.secret && old.name == value.name);
        if had_it {
            plan.moves
                .push((secret_name(&previous.name, kind, &value.name), name));
        }
    }
    if let Some(previous) = previous {
        plan.delete = secret_names(previous)
            .into_iter()
            .filter(|old| !kept.contains(old))
            .collect();
    }
    plan
}

/// Removing `server` forgets its secrets.
pub fn plan_remove(server: &McpServer) -> SecretPlan {
    SecretPlan {
        delete: secret_names(server),
        ..SecretPlan::default()
    }
}

/// Writes and moves the plan's secrets (the settings are saved after
/// this, so a failure leaves them as they were).
pub fn apply_writes(plan: &SecretPlan) -> Result<(), SecretError> {
    for (name, value) in &plan.write {
        secrets::set(name, value)?;
    }
    for (from, to) in &plan.moves {
        if let Some(value) = secrets::get(from)? {
            secrets::set(to, &value)?;
        }
    }
    Ok(())
}

/// Forgets what the saved settings no longer use. A failure leaves an
/// unused credential behind, which is logged rather than undoing the save.
pub fn apply_deletes(plan: &SecretPlan) {
    for name in &plan.delete {
        if let Err(e) = secrets::delete(name) {
            tracing::warn!("could not remove the secret {name}: {e}");
        }
    }
}

/// `server` as saved in the settings: secret values blanked (they're in
/// Credential Manager).
pub fn without_secret_values(mut server: McpServer) -> McpServer {
    for value in server.env.iter_mut().chain(server.headers.iter_mut()) {
        if value.secret {
            value.value.clear();
        }
    }
    server
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp_registry::tests::{http, stdio, value};
    use crate::integrations::mcp_registry::McpTransport;

    #[test]
    fn names_secrets_after_server_and_entry() {
        assert_eq!(
            secret_name("github", ValueKind::Env, "GITHUB_TOKEN"),
            "mcp:github:env:GITHUB_TOKEN"
        );
        assert_eq!(
            secret_name("linear", ValueKind::Header, "Authorization"),
            "mcp:linear:header:Authorization"
        );
        assert!(crate::secrets::check_name("mcp:my_server-2:env:A_B").is_ok());
    }

    #[test]
    fn a_new_server_writes_its_values() {
        let mut s = stdio("github");
        s.env[0].value = "ghp_new".into();
        let plan = plan_save(None, &s);
        assert_eq!(plan.write.len(), 1);
        assert_eq!(plan.write[0].0, "mcp:github:env:GITHUB_TOKEN");
        assert_eq!(plan.write[0].1.expose(), "ghp_new");
        assert!(plan.moves.is_empty() && plan.delete.is_empty());
        assert!(!format!("{plan:?}").contains("ghp_new"));
    }

    #[test]
    fn an_empty_secret_keeps_the_saved_one() {
        let s = stdio("github");
        assert_eq!(plan_save(Some(&s), &s), SecretPlan::default());
    }

    #[test]
    fn a_rename_moves_kept_secrets_and_forgets_the_old_names() {
        let old = stdio("github");
        let mut new = stdio("gh");
        new.env.push(value("EXTRA", "", true));
        let plan = plan_save(Some(&old), &new);
        assert!(plan.write.is_empty());
        assert_eq!(
            plan.moves,
            [(
                "mcp:github:env:GITHUB_TOKEN".to_owned(),
                "mcp:gh:env:GITHUB_TOKEN".to_owned()
            )]
        );
        assert_eq!(plan.delete, ["mcp:github:env:GITHUB_TOKEN"]);
    }

    #[test]
    fn dropped_or_unmarked_entries_are_forgotten() {
        let mut old = stdio("github");
        old.env.push(value("OTHER", "", true));
        let mut new = stdio("github");
        new.env[0].secret = false;
        new.env[0].value = "now plain".into();
        let plan = plan_save(Some(&old), &new);
        assert!(plan.write.is_empty() && plan.moves.is_empty());
        let mut deleted = plan.delete.clone();
        deleted.sort();
        assert_eq!(
            deleted,
            ["mcp:github:env:GITHUB_TOKEN", "mcp:github:env:OTHER"]
        );
    }

    #[test]
    fn switching_transport_forgets_the_other_side() {
        let old = stdio("svc");
        let mut new = http("svc");
        new.headers[0].value = "Bearer x".into();
        let plan = plan_save(Some(&old), &new);
        assert_eq!(plan.write[0].0, "mcp:svc:header:Authorization");
        assert!(plan.moves.is_empty());
        assert_eq!(plan.delete, ["mcp:svc:env:GITHUB_TOKEN"]);
        assert_eq!(new.transport, McpTransport::Http);
    }

    #[test]
    fn removing_forgets_every_secret() {
        let plan = plan_remove(&http("linear"));
        assert_eq!(plan.delete, ["mcp:linear:header:Authorization"]);
    }

    #[test]
    fn applies_a_rename_in_the_store() {
        let old = stdio("apply-old");
        secrets::set(
            &secret_name("apply-old", ValueKind::Env, "GITHUB_TOKEN"),
            &Secret::new("ghp_kept"),
        )
        .unwrap();
        let new = stdio("apply-new");
        let plan = plan_save(Some(&old), &new);
        apply_writes(&plan).unwrap();
        apply_deletes(&plan);
        let moved = secrets::get("mcp:apply-new:env:GITHUB_TOKEN").unwrap();
        assert_eq!(moved.unwrap().expose(), "ghp_kept");
        assert!(secrets::get("mcp:apply-old:env:GITHUB_TOKEN")
            .unwrap()
            .is_none());
    }

    #[test]
    fn saved_settings_hold_no_secret_values() {
        let mut s = stdio("github");
        s.env[0].value = "ghp_new".into();
        let saved = without_secret_values(s);
        assert_eq!(saved.env[0].value, "");
        assert_eq!(saved.env[1].value, "info");
    }
}
