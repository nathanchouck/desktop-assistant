//! The registry as each AI takes it: Claude Code reads a `--mcp-config`
//! JSON file (with the launcher's own server), Codex gets each server in a
//! thread's config over stdin. Building both is pure; secrets come in
//! through a lookup, so tests never need a real store.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Map, Value};

use super::mcp_edit::secret_name;
use super::mcp_registry::{check_server, McpServer, McpSettings, McpTransport, McpValue};
use crate::mcp::Endpoint;
use crate::secrets::Secret;

/// Claude Code's `--mcp-config` file, in the data folder (this PC only,
/// never the synced settings folder).
pub const CLAUDE_CONFIG_FILE: &str = "mcp-config.json";

/// A server ready for an AI: enabled, valid, with its secrets filled in.
/// Every value is a `Secret`, so `Debug` never shows one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedServer {
    pub name: String,
    pub launch: ResolvedLaunch,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedLaunch {
    Stdio {
        command: String,
        args: Vec<String>,
        env: Vec<(String, Secret)>,
    },
    Http {
        url: String,
        headers: Vec<(String, Secret)>,
    },
}

/// The servers to give the AIs now. Off, invalid (a hand edit, a synced
/// file from a newer version) and disabled servers are left out; a secret
/// that isn't saved on this PC is left out of its server.
pub fn resolve(
    settings: &McpSettings,
    lookup: impl Fn(&str) -> Option<Secret>,
) -> Vec<ResolvedServer> {
    if !settings.enabled {
        return Vec::new();
    }
    settings
        .servers
        .iter()
        .enumerate()
        .filter(|(_, s)| s.enabled)
        .filter_map(|(i, server)| {
            let others = settings.servers.iter().take(i);
            if let Err(problem) = check_server(server, others) {
                tracing::warn!("MCP server {:?} left out: {problem}", server.name);
                return None;
            }
            Some(resolve_one(server, &lookup))
        })
        .collect()
}

fn resolve_one(server: &McpServer, lookup: &impl Fn(&str) -> Option<Secret>) -> ResolvedServer {
    let (kind, values) = server.values();
    let values: Vec<(String, Secret)> = values
        .iter()
        .filter_map(|v: &McpValue| {
            if !v.secret {
                return Some((v.name.clone(), Secret::new(v.value.clone())));
            }
            let found = lookup(&secret_name(&server.name, kind, &v.name));
            if found.is_none() {
                tracing::info!(
                    "MCP server {}: {} isn't saved on this PC; left out",
                    server.name,
                    v.name
                );
            }
            found.map(|secret| (v.name.clone(), secret))
        })
        .collect();
    let launch = match server.transport {
        McpTransport::Stdio => ResolvedLaunch::Stdio {
            command: server.command.clone(),
            args: server.args.clone(),
            env: values,
        },
        McpTransport::Http => ResolvedLaunch::Http {
            url: server.url.clone(),
            headers: values,
        },
    };
    ResolvedServer {
        name: server.name.clone(),
        launch,
    }
}

fn object(values: &[(String, Secret)]) -> Value {
    Value::Object(
        values
            .iter()
            .map(|(name, value)| (name.clone(), json!(value.expose())))
            .collect(),
    )
}

/// One server in Claude Code's `mcpServers` shape.
pub fn claude_entry(server: &ResolvedServer) -> Value {
    match &server.launch {
        ResolvedLaunch::Stdio { command, args, env } => json!({
            "type": "stdio",
            "command": command,
            "args": args,
            "env": object(env),
        }),
        ResolvedLaunch::Http { url, headers } => json!({
            "type": "http",
            "url": url,
            "headers": object(headers),
        }),
    }
}

/// Claude Code's whole `--mcp-config`: the launcher's server (when it's
/// running) and the registry's. `None` when there's nothing to add.
pub fn claude_config(launcher: Option<&Endpoint>, servers: &[ResolvedServer]) -> Option<Value> {
    if launcher.is_none() && servers.is_empty() {
        return None;
    }
    let mut config = launcher
        .map(Endpoint::config)
        .unwrap_or_else(|| json!({ "mcpServers": {} }));
    if let Some(list) = config["mcpServers"].as_object_mut() {
        for server in servers {
            list.insert(server.name.clone(), claude_entry(server));
        }
    }
    Some(config)
}

/// One server in Codex's `mcp_servers.<name>` shape.
pub fn codex_entry(server: &ResolvedServer) -> Value {
    match &server.launch {
        ResolvedLaunch::Stdio { command, args, env } => json!({
            "command": command,
            "args": args,
            "env": object(env),
        }),
        ResolvedLaunch::Http { url, headers } => json!({
            "url": url,
            "http_headers": object(headers),
        }),
    }
}

/// The registry's servers for a Codex thread's `config.mcp_servers`.
pub fn codex_servers(servers: &[ResolvedServer]) -> Map<String, Value> {
    servers
        .iter()
        .map(|s| (s.name.clone(), codex_entry(s)))
        .collect()
}

/// Writes Claude Code's `--mcp-config` file into `dir` when its content
/// changed, and returns its path. `None` when Claude needs no file; an old
/// one is removed then, since it may hold secrets.
pub fn write_claude_config(
    dir: &Path,
    launcher: Option<&Endpoint>,
    servers: &[ResolvedServer],
) -> io::Result<Option<PathBuf>> {
    let path = dir.join(CLAUDE_CONFIG_FILE);
    let Some(config) = claude_config(launcher, servers) else {
        match std::fs::remove_file(&path) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => return Ok(None),
        }
    };
    std::fs::create_dir_all(dir)?;
    write_if_changed(&path, &config.to_string())?;
    Ok(Some(path))
}

/// Replaces `path` with `text` unless it already holds it. Returns whether
/// it wrote.
fn write_if_changed(path: &Path, text: &str) -> io::Result<bool> {
    if std::fs::read_to_string(path).is_ok_and(|old| old == text) {
        return Ok(false);
    }
    // Whole or not at all: a Claude Code starting right now reads it.
    let temp = path.with_extension("json.tmp");
    std::fs::write(&temp, text)?;
    if let Err(e) = std::fs::rename(&temp, path) {
        // Windows refuses to replace a file another program holds open.
        tracing::debug!(
            "could not replace {} ({e}); writing it in place",
            path.display()
        );
        let _ = std::fs::remove_file(&temp);
        std::fs::write(path, text)?;
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::integrations::mcp_registry::tests::{http, stdio, value};

    fn lookup(name: &str) -> Option<Secret> {
        match name {
            "mcp:github:env:GITHUB_TOKEN" => Some(Secret::new("ghp_secret")),
            "mcp:linear:header:Authorization" => Some(Secret::new("Bearer lin_secret")),
            _ => None,
        }
    }

    fn settings() -> McpSettings {
        McpSettings {
            enabled: true,
            servers: vec![stdio("github"), http("linear")],
        }
    }

    fn launcher() -> Endpoint {
        Endpoint {
            port: 4321,
            token: "launcher-token".into(),
        }
    }

    #[test]
    fn fills_in_secrets_and_skips_what_is_off() {
        let mut s = settings();
        let mut off = stdio("off");
        off.enabled = false;
        let mut bad = stdio("launcher");
        bad.command = "x".into();
        let mut twice = http("linear");
        twice.url = "https://other.example".into();
        s.servers.extend([off, bad, twice]);
        let resolved = resolve(&s, lookup);
        let names: Vec<_> = resolved.iter().map(|r| r.name.as_str()).collect();
        assert_eq!(names, ["github", "linear"], "the first of a name wins");
        let ResolvedLaunch::Stdio { env, .. } = &resolved[0].launch else {
            panic!("stdio")
        };
        assert_eq!(env[0].0, "GITHUB_TOKEN");
        assert_eq!(env[0].1.expose(), "ghp_secret");
        assert_eq!(env[1].1.expose(), "info");

        s.enabled = false;
        assert!(resolve(&s, lookup).is_empty());
    }

    #[test]
    fn a_secret_missing_on_this_pc_is_left_out() {
        let resolved = resolve(&settings(), |_| None);
        let ResolvedLaunch::Stdio { env, .. } = &resolved[0].launch else {
            panic!("stdio")
        };
        assert_eq!(env.len(), 1);
        assert_eq!(env[0].0, "LOG");
        let ResolvedLaunch::Http { headers, .. } = &resolved[1].launch else {
            panic!("http")
        };
        assert!(headers.is_empty());
    }

    #[test]
    fn debug_output_hides_every_value() {
        let resolved = resolve(&settings(), lookup);
        let shown = format!("{resolved:?}");
        assert!(!shown.contains("ghp_secret"), "{shown}");
        assert!(!shown.contains("lin_secret"), "{shown}");
        assert!(shown.contains("GITHUB_TOKEN") && shown.contains("linear"));
    }

    #[test]
    fn claude_gets_the_launcher_and_the_registry() {
        let servers = resolve(&settings(), lookup);
        let config = claude_config(Some(&launcher()), &servers).unwrap();
        let list = &config["mcpServers"];
        assert_eq!(list["launcher"]["type"], "http");
        assert_eq!(
            list["launcher"]["headers"]["Authorization"],
            "Bearer launcher-token"
        );
        assert_eq!(
            list["github"],
            json!({
                "type": "stdio",
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": { "GITHUB_TOKEN": "ghp_secret", "LOG": "info" },
            })
        );
        assert_eq!(
            list["linear"],
            json!({
                "type": "http",
                "url": "https://mcp.example.com/mcp",
                "headers": { "Authorization": "Bearer lin_secret" },
            })
        );
        // Without the launcher's server, the registry still goes to Claude.
        let only = claude_config(None, &servers).unwrap();
        assert!(only["mcpServers"].get("launcher").is_none());
        assert_eq!(only["mcpServers"].as_object().unwrap().len(), 2);
        assert!(claude_config(None, &[]).is_none());
    }

    #[test]
    fn codex_gets_its_own_shape() {
        let servers = codex_servers(&resolve(&settings(), lookup));
        assert_eq!(
            servers["github"],
            json!({
                "command": "npx",
                "args": ["-y", "@modelcontextprotocol/server-github"],
                "env": { "GITHUB_TOKEN": "ghp_secret", "LOG": "info" },
            })
        );
        assert_eq!(
            servers["linear"],
            json!({
                "url": "https://mcp.example.com/mcp",
                "http_headers": { "Authorization": "Bearer lin_secret" },
            })
        );
    }

    #[test]
    fn writes_the_file_only_when_it_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut servers = resolve(&settings(), lookup);
        let path = write_claude_config(dir.path(), Some(&launcher()), &servers)
            .unwrap()
            .unwrap();
        assert_eq!(path, dir.path().join(CLAUDE_CONFIG_FILE));
        let read =
            || -> Value { serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap() };
        assert_eq!(read()["mcpServers"]["github"]["command"], "npx");

        let same = claude_config(Some(&launcher()), &servers)
            .unwrap()
            .to_string();
        assert!(!write_if_changed(&path, &same).unwrap(), "left alone");

        // A server removed: rewritten, the launcher's server kept.
        servers.retain(|s| s.name != "github");
        write_claude_config(dir.path(), Some(&launcher()), &servers).unwrap();
        assert!(read()["mcpServers"].get("github").is_none());
        assert_eq!(
            read()["mcpServers"]["launcher"]["url"],
            "http://127.0.0.1:4321/mcp"
        );
        assert!(!path.with_extension("json.tmp").exists());

        // Nothing left to give Claude: the old file goes.
        assert_eq!(write_claude_config(dir.path(), None, &[]).unwrap(), None);
        assert!(!path.exists());
        assert_eq!(write_claude_config(dir.path(), None, &[]).unwrap(), None);
    }

    #[test]
    fn plain_values_keep_their_text() {
        let mut s = stdio("tools");
        s.env = vec![value("EMPTY", "", false)];
        let resolved = resolve(
            &McpSettings {
                enabled: true,
                servers: vec![s],
            },
            lookup,
        );
        assert_eq!(codex_entry(&resolved[0])["env"]["EMPTY"], "");
    }
}
