//! ChatGPT through OpenAI's Codex CLI: how a conversation's
//! `codex app-server` starts (billing, sandbox, instructions). `locate.rs`
//! finds Codex, `status.rs` checks its sign-in, `protocol.rs` and
//! `parser.rs` speak its JSON-RPC, `session.rs` drives one process.
//!
//! `codex app-server` is the interface with streamed text, approvals we can
//! answer, interrupts and resuming a thread (`codex exec --json` has none
//! of those).

mod describe;
mod locate;
pub mod parser;
pub mod protocol;
pub mod session;
mod status;

use std::path::{Path, PathBuf};
use std::process::Stdio;

pub use super::launch::Launch;
use super::{AiError, Approvals, BillingMode};
use crate::settings::ChatGptSettings;
pub use locate::locate;
pub use status::{check_status, parse_login_status, sign_in, CodexStatus};

/// How messages name it.
pub const NAME: &str = "Codex";

/// Variables that would sign Codex in with an API key (or token) instead of
/// ChatGPT, or that commands Codex runs could read. Always removed from its
/// environment; in API key mode the key goes over stdin instead.
pub const SECRET_ENV: [&str; 3] = ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];

/// Where API key mode reads the key from, in order.
const KEY_ENV: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];

/// An OpenAI API key. Its `Debug` never shows it, so it can't end up in a log.
#[derive(Clone, PartialEq, Eq)]
pub struct ApiKey(String);

impl ApiKey {
    pub fn new(key: impl Into<String>) -> Self {
        Self(key.into())
    }

    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("ApiKey(hidden)")
    }
}

/// The MCP server the launcher runs, for Codex's per-thread config.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct McpServer {
    pub url: String,
    pub token: String,
}

/// Everything that decides how a conversation's process and thread start.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    /// Empty: Codex's default (the user's `config.toml`, else its own).
    pub model: String,
    pub approvals: Approvals,
    pub billing: BillingMode,
    /// API key mode: the key, sent over stdin once Codex starts.
    pub api_key: Option<ApiKey>,
    pub cwd: PathBuf,
    /// The user's home folder: auto mode never makes it writable as a whole.
    pub home: PathBuf,
    /// Continue an earlier thread (after the idle process was shut down).
    pub resume: Option<String>,
    pub instructions: String,
    /// The launcher's own tools, when its MCP server is running.
    pub mcp: Option<McpServer>,
    /// Desktop context is on: the screen tools are offered.
    pub screen_tools: bool,
    /// Ask for Codex's Windows sandbox (no admin needed) because the user's
    /// Codex config doesn't choose one. Without it every command asks.
    pub windows_sandbox: bool,
}

/// Which sandbox a thread runs in (see docs/ARCHITECTURE.md, "Approvals").
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Sandbox {
    /// Reads anything; writing or the network needs your Approve.
    ReadOnly,
    /// Also writes inside the working folder without asking.
    WorkspaceWrite,
}

impl Sandbox {
    pub fn as_str(self) -> &'static str {
        match self {
            Sandbox::ReadOnly => "read-only",
            Sandbox::WorkspaceWrite => "workspace-write",
        }
    }
}

/// "Ask": read-only, so every change comes to the Approve card. "Auto":
/// Codex's own auto mode, writing freely inside the folder you came from.
/// With no folder (the chat starts in your home folder), auto stays
/// read-only: the whole home folder is too much to hand over.
pub fn sandbox(approvals: Approvals, cwd: &Path, home: &Path) -> Sandbox {
    match approvals {
        Approvals::Auto if cwd != home => Sandbox::WorkspaceWrite,
        _ => Sandbox::ReadOnly,
    }
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// `codex` without any key in its environment (see `SECRET_ENV`), and
/// without our log filter, which would make it log requests.
fn base_command(launch: &Launch, window_flags: u32) -> tokio::process::Command {
    let mut cmd = launch.command();
    for var in SECRET_ENV {
        cmd.env_remove(var);
    }
    cmd.env_remove("RUST_LOG");
    #[cfg(windows)]
    cmd.creation_flags(window_flags);
    #[cfg(not(windows))]
    let _ = window_flags;
    cmd
}

/// Command-line arguments for a conversation (after any `node codex.js`).
pub fn args(options: &Options) -> Vec<String> {
    let mut args = vec!["app-server".to_owned()];
    if options.billing == BillingMode::ApiKey {
        // The key is given over stdin and kept in memory only, so the
        // ChatGPT sign-in saved on disk stays as it is.
        args.extend([
            "-c".into(),
            "cli_auth_credentials_store=\"ephemeral\"".into(),
        ]);
    }
    if options.windows_sandbox {
        args.extend(["-c".into(), "windows.sandbox=\"unelevated\"".into()]);
    }
    args
}

/// The ready-to-spawn conversation process: pipes, working folder, no
/// console window.
pub fn command(launch: &Launch, options: &Options) -> tokio::process::Command {
    let mut cmd = base_command(launch, CREATE_NO_WINDOW);
    cmd.args(args(options))
        .current_dir(&options.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    cmd
}

/// The API key for API key mode, from the environment.
pub fn api_key() -> Option<ApiKey> {
    KEY_ENV
        .iter()
        .filter_map(std::env::var_os)
        .map(|v| v.to_string_lossy().trim().to_owned())
        .find(|v| !v.is_empty())
        .map(ApiKey)
}

/// Codex's own folder: `CODEX_HOME`, else `~/.codex`.
fn codex_home() -> Option<PathBuf> {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(|| dirs::home_dir().map(|h| h.join(".codex")))
}

/// Whether the user's Codex config already picks a Windows sandbox
/// (`[windows] sandbox = …`), which we then leave alone.
pub fn config_picks_windows_sandbox(config_toml: &str) -> bool {
    toml::from_str::<toml::Table>(config_toml)
        .ok()
        .and_then(|t| t.get("windows")?.get("sandbox").cloned())
        .is_some()
}

/// Only on Windows, and only when the user's config doesn't choose one.
fn wants_windows_sandbox() -> bool {
    if !cfg!(windows) {
        return false;
    }
    let config = codex_home()
        .and_then(|home| std::fs::read_to_string(home.join("config.toml")).ok())
        .unwrap_or_default();
    !config_picks_windows_sandbox(&config)
}

/// Where Codex is and how to start it, from the settings.
pub fn prepare(settings: &ChatGptSettings) -> Result<(Launch, Options), AiError> {
    let launch = locate(&settings.path).ok_or(AiError::CodexNotInstalled)?;
    let api_key = match settings.billing {
        BillingMode::ApiKey => Some(api_key().ok_or(AiError::MissingOpenAiKey)?),
        BillingMode::Subscription => None,
    };
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let options = Options {
        model: settings.model.trim().to_owned(),
        approvals: settings.approvals,
        billing: settings.billing,
        api_key,
        instructions: instructions(&home),
        cwd: home.clone(),
        home,
        resume: None,
        mcp: None,
        screen_tools: settings.desktop_context,
        windows_sandbox: wants_windows_sandbox(),
    };
    Ok((launch, options))
}

/// Tells ChatGPT where it's running and how to answer (added to Codex's own
/// instructions, not replacing them).
pub fn instructions(home: &Path) -> String {
    format!(
        "You are answering inside a small desktop launcher popup on the user's Windows PC, \
         not a terminal or an editor. Keep answers short and easy to skim; use Markdown \
         sparingly. The user's home folder is {}. When the user asks you to do something \
         with their files or apps (move, rename, sort, clean up), do it with your tools \
         rather than explaining how; the launcher asks the user to approve changes when \
         needed. A message may start with a <desktop_context> block, added automatically: \
         the window the user was in when they opened the launcher, and for File Explorer \
         the folder shown and the selected files. Words like \"this\", \"these files\" and \
         \"here\" refer to it. Use it when it's relevant; don't mention it otherwise. \
         The launcher also offers tools on its MCP server \"launcher\" (named \
         mcp__launcher__<tool>; in code mode tools.mcp__launcher__<tool>): \
         get_desktop_context (a fresh look at the screen), search (the user's apps and \
         files by name, instantly), open (a file, folder or app), reveal (show a file in \
         File Explorer) and screenshot (a picture of the user's window; only when you need \
         to see it, and the user is asked first).",
        home.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn options() -> Options {
        Options {
            model: String::new(),
            approvals: Approvals::Ask,
            billing: BillingMode::Subscription,
            api_key: None,
            cwd: PathBuf::from("/home/me/Downloads"),
            home: PathBuf::from("/home/me"),
            resume: None,
            instructions: String::new(),
            mcp: None,
            screen_tools: true,
            windows_sandbox: false,
        }
    }

    #[test]
    fn starts_the_app_server() {
        assert_eq!(args(&options()), ["app-server"]);
        let a = args(&Options {
            billing: BillingMode::ApiKey,
            windows_sandbox: true,
            ..options()
        });
        assert!(a
            .windows(2)
            .any(|w| w == ["-c", "cli_auth_credentials_store=\"ephemeral\""]));
        assert!(a
            .windows(2)
            .any(|w| w == ["-c", "windows.sandbox=\"unelevated\""]));
        assert!(
            !a.iter().any(|x| x.contains("forced_login_method")),
            "a login mismatch would sign the user out"
        );
    }

    #[test]
    fn never_shows_the_key() {
        let options = Options {
            api_key: Some(ApiKey::new("sk-secret")),
            ..options()
        };
        assert!(!format!("{options:?}").contains("sk-secret"));
        assert_eq!(options.api_key.unwrap().expose(), "sk-secret");
    }

    #[test]
    fn auto_writes_only_inside_the_folder_you_came_from() {
        let home = Path::new("/home/me");
        let downloads = Path::new("/home/me/Downloads");
        assert_eq!(sandbox(Approvals::Ask, downloads, home), Sandbox::ReadOnly);
        assert_eq!(
            sandbox(Approvals::Auto, downloads, home),
            Sandbox::WorkspaceWrite
        );
        assert_eq!(sandbox(Approvals::Auto, home, home), Sandbox::ReadOnly);
        assert_eq!(Sandbox::WorkspaceWrite.as_str(), "workspace-write");
    }

    #[test]
    fn respects_a_windows_sandbox_the_user_chose() {
        assert!(config_picks_windows_sandbox(
            "[windows]\nsandbox = \"elevated\"\n"
        ));
        assert!(!config_picks_windows_sandbox("model = \"gpt-6-luna\"\n"));
        assert!(!config_picks_windows_sandbox(
            "[windows]\nallow_mxc = false\n"
        ));
        assert!(!config_picks_windows_sandbox("not toml = ="));
        assert!(!config_picks_windows_sandbox(""));
    }
}
