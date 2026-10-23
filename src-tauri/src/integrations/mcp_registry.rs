//! The MCP registry's settings: servers you add once, given to both AIs.
//! Plain text that can sync between PCs; a value marked secret is kept in
//! Credential Manager on each PC and only named here.

use std::fmt;

use serde::ser::SerializeStruct;
use serde::{Deserialize, Serialize, Serializer};

use crate::mcp::protocol::SERVER_NAME;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct McpSettings {
    /// Give Claude and ChatGPT the servers below.
    pub enabled: bool,
    pub servers: Vec<McpServer>,
}

impl Default for McpSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            servers: Vec::new(),
        }
    }
}

/// One MCP server. A stdio server is a program the AI starts (`command`,
/// `args`, `env`); an HTTP server is an address (`url`, `headers`). The
/// other transport's fields stay empty.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct McpServer {
    /// What the AIs call it (their tools are `mcp__<name>__<tool>`):
    /// lowercase letters, digits, `-` and `_`.
    pub name: String,
    pub enabled: bool,
    pub transport: McpTransport,
    /// The program that starts the server ("npx", "uvx", a full path).
    pub command: String,
    pub args: Vec<String>,
    /// Environment variables for the program.
    pub env: Vec<McpValue>,
    /// The server's address (http:// or https://).
    pub url: String,
    /// Headers sent with each request (an API key, `Authorization`).
    pub headers: Vec<McpValue>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum McpTransport {
    Stdio,
    Http,
}

/// An environment variable or a header. A secret's value lives in
/// Credential Manager: here it's always empty (it is blanked whenever
/// settings are written), except in an edit coming from the settings menu,
/// where it is the new value to save.
#[derive(Clone, PartialEq, Deserialize, specta::Type)]
pub struct McpValue {
    pub name: String,
    pub value: String,
    pub secret: bool,
}

impl Serialize for McpValue {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut out = serializer.serialize_struct("McpValue", 3)?;
        out.serialize_field("name", &self.name)?;
        out.serialize_field("value", if self.secret { "" } else { &self.value })?;
        out.serialize_field("secret", &self.secret)?;
        out.end()
    }
}

impl fmt::Debug for McpValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let value = if self.secret { "***" } else { &self.value };
        f.debug_struct("McpValue")
            .field("name", &self.name)
            .field("value", &value)
            .field("secret", &self.secret)
            .finish()
    }
}

/// Which list a value is in, for messages and secret names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValueKind {
    Env,
    Header,
}

impl ValueKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ValueKind::Env => "env",
            ValueKind::Header => "header",
        }
    }
}

impl McpServer {
    /// The values this server's transport uses (the others are ignored).
    pub fn values(&self) -> (ValueKind, &[McpValue]) {
        match self.transport {
            McpTransport::Stdio => (ValueKind::Env, &self.env),
            McpTransport::Http => (ValueKind::Header, &self.headers),
        }
    }

    /// Trimmed, with the other transport's fields and empty arguments
    /// dropped, so what's saved is only what's used.
    pub fn normalized(mut self) -> Self {
        self.name = self.name.trim().to_owned();
        self.command = self.command.trim().to_owned();
        self.url = self.url.trim().to_owned();
        self.args = self
            .args
            .iter()
            .map(|a| a.trim())
            .filter(|a| !a.is_empty())
            .map(String::from)
            .collect();
        for value in self.env.iter_mut().chain(self.headers.iter_mut()) {
            value.name = value.name.trim().to_owned();
            if value.secret {
                value.value = value.value.trim().to_owned();
            }
        }
        match self.transport {
            McpTransport::Stdio => {
                self.url.clear();
                self.headers.clear();
            }
            McpTransport::Http => {
                self.command.clear();
                self.args.clear();
                self.env.clear();
            }
        }
        self
    }
}

const MAX_NAME: usize = 64;

/// Whether `server` can be saved next to `others` (the rest of the list).
/// The message says what to fix.
pub fn check_server<'a>(
    server: &McpServer,
    others: impl IntoIterator<Item = &'a McpServer>,
) -> Result<(), String> {
    check_name(&server.name)?;
    if others.into_iter().any(|o| o.name == server.name) {
        return Err(format!(
            "There's already a server called \"{}\".",
            server.name
        ));
    }
    match server.transport {
        McpTransport::Stdio if server.command.trim().is_empty() => {
            return Err("Enter the command that starts the server.".into());
        }
        McpTransport::Http if !is_web_address(&server.url) => {
            return Err("Enter the server's address, starting with http:// or https://.".into());
        }
        _ => {}
    }
    let (kind, values) = server.values();
    check_values(kind, values)
}

/// Names become config keys and tool names in both AIs.
pub fn check_name(name: &str) -> Result<(), String> {
    if name.is_empty() {
        return Err("Give the server a name.".into());
    }
    if name == SERVER_NAME {
        return Err(format!(
            "\"{SERVER_NAME}\" is the launcher's own server. Pick another name."
        ));
    }
    let ok = name.len() <= MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(format!(
            "\"{name}\" won't work as a name: use lowercase letters, digits, - and _ (up to {MAX_NAME})."
        ))
    }
}

fn is_web_address(url: &str) -> bool {
    let rest = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"));
    rest.is_some_and(|r| !r.is_empty() && !r.chars().any(char::is_whitespace))
}

fn check_values(kind: ValueKind, values: &[McpValue]) -> Result<(), String> {
    let what = match kind {
        ValueKind::Env => "variable",
        ValueKind::Header => "header",
    };
    let mut seen: Vec<String> = Vec::new();
    for value in values {
        let name = &value.name;
        if name.is_empty() {
            return Err(format!("Give each {what} a name."));
        }
        let ok = match kind {
            // Windows' own rules are looser, but these work everywhere.
            ValueKind::Env => {
                !name.starts_with(|c: char| c.is_ascii_digit())
                    && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
            }
            ValueKind::Header => name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_'),
        };
        if !ok {
            return Err(match kind {
                ValueKind::Env => format!(
                    "\"{name}\" won't work as a variable name: use letters, digits and _, not starting with a digit."
                ),
                ValueKind::Header => format!(
                    "\"{name}\" won't work as a header name: use letters, digits, - and _."
                ),
            });
        }
        // Both are case-insensitive on Windows / in HTTP.
        let key = name.to_ascii_lowercase();
        if seen.contains(&key) {
            return Err(format!("The {what} \"{name}\" is listed twice."));
        }
        seen.push(key);
        if kind == ValueKind::Header && value.value.contains(['\r', '\n']) {
            return Err(format!("The header \"{name}\" can't contain a line break."));
        }
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub fn value(name: &str, value: &str, secret: bool) -> McpValue {
        McpValue {
            name: name.into(),
            value: value.into(),
            secret,
        }
    }

    pub fn stdio(name: &str) -> McpServer {
        McpServer {
            name: name.into(),
            enabled: true,
            transport: McpTransport::Stdio,
            command: "npx".into(),
            args: vec!["-y".into(), "@modelcontextprotocol/server-github".into()],
            env: vec![value("GITHUB_TOKEN", "", true), value("LOG", "info", false)],
            url: String::new(),
            headers: Vec::new(),
        }
    }

    pub fn http(name: &str) -> McpServer {
        McpServer {
            name: name.into(),
            enabled: true,
            transport: McpTransport::Http,
            command: String::new(),
            args: Vec::new(),
            env: Vec::new(),
            url: "https://mcp.example.com/mcp".into(),
            headers: vec![value("Authorization", "", true)],
        }
    }

    #[test]
    fn defaults_are_on_and_empty() {
        let d = McpSettings::default();
        assert!(d.enabled);
        assert!(d.servers.is_empty());
    }

    #[test]
    fn checks_names() {
        assert!(check_name("github").is_ok());
        assert!(check_name("my_server-2").is_ok());
        assert!(check_name("").unwrap_err().contains("name"));
        assert!(check_name("launcher")
            .unwrap_err()
            .contains("launcher's own"));
        for bad in ["GitHub", "has space", "dots.too", "ü", &"x".repeat(65)] {
            assert!(check_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn checks_a_server_against_the_list() {
        let others = [stdio("github"), http("linear")];
        assert!(check_server(&stdio("files"), &others).is_ok());
        let taken = check_server(&http("linear"), &others).unwrap_err();
        assert!(taken.contains("already"), "{taken}");
        let mut no_command = stdio("x");
        no_command.command = " ".into();
        assert!(check_server(&no_command, &others)
            .unwrap_err()
            .contains("command"));
        for url in ["", "mcp.example.com", "ftp://x", "https://", "https://a b"] {
            let mut bad = http("y");
            bad.url = url.into();
            assert!(check_server(&bad, &others).is_err(), "{url}");
        }
        let mut plain = http("y");
        plain.url = "http://127.0.0.1:8080/mcp".into();
        assert!(check_server(&plain, &[]).is_ok());
    }

    #[test]
    fn checks_variables_and_headers() {
        let mut s = stdio("a");
        s.env = vec![value("1ABC", "x", false)];
        assert!(check_server(&s, &[]).unwrap_err().contains("variable name"));
        s.env = vec![value("TOKEN", "x", false), value("token", "y", false)];
        assert!(check_server(&s, &[]).unwrap_err().contains("twice"));
        s.env = vec![value("", "x", false)];
        assert!(check_server(&s, &[]).is_err());

        let mut h = http("b");
        h.headers = vec![value("X-Api-Key", "a\r\nInjected: yes", false)];
        assert!(check_server(&h, &[]).unwrap_err().contains("line break"));
        h.headers = vec![value("Bad Header", "x", false)];
        assert!(check_server(&h, &[]).is_err());
        h.headers = vec![value("X-Api-Key", "x", false)];
        assert!(check_server(&h, &[]).is_ok());
    }

    #[test]
    fn normalizing_keeps_only_what_the_transport_uses() {
        let mut s = stdio(" github ");
        s.args.push("  ".into());
        s.url = "https://left.over".into();
        s.headers = vec![value("X", "y", false)];
        let n = s.normalized();
        assert_eq!(n.name, "github");
        assert_eq!(n.args, ["-y", "@modelcontextprotocol/server-github"]);
        assert!(n.url.is_empty() && n.headers.is_empty());

        let mut h = http("linear");
        h.command = "npx".into();
        h.env = vec![value("A", "b", false)];
        let n = h.normalized();
        assert!(n.command.is_empty() && n.env.is_empty() && n.args.is_empty());
        assert_eq!(n.headers.len(), 1);
    }

    #[test]
    fn secret_values_never_reach_the_settings_file_or_debug() {
        let mut s = stdio("github");
        s.env = vec![
            value("GITHUB_TOKEN", "ghp_supersecret", true),
            value("LOG", "info", false),
        ];
        let mut settings = crate::settings::Settings::default();
        settings.mcp.servers.push(s.clone());
        let toml = toml::to_string_pretty(&settings).unwrap();
        assert!(!toml.contains("ghp_supersecret"), "{toml}");
        assert!(toml.contains("GITHUB_TOKEN") && toml.contains("info"));
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("ghp_supersecret"));
        assert!(!format!("{s:?}").contains("ghp_supersecret"));
        assert!(format!("{s:?}").contains("info"));
    }

    #[test]
    fn servers_round_trip_through_the_settings_file() {
        let mut settings = crate::settings::Settings::default();
        settings.mcp.servers = vec![stdio("github"), http("linear")];
        settings.mcp.enabled = false;
        let text = toml::to_string_pretty(&settings).unwrap();
        let back: crate::settings::Settings = toml::from_str(&text).unwrap();
        assert_eq!(back.mcp, settings.mcp);
    }
}
