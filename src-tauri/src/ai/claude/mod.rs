//! Claude Code as the chat engine: where it's installed, and the exact
//! command line for a conversation. `protocol.rs` speaks its stream-json.

pub mod protocol;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde::Serialize;

use super::{AiError, Approvals, BillingMode};
use crate::settings::ClaudeSettings;

/// How to start Claude Code on this machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Launch {
    /// A native `claude` executable (the official installer).
    Exe(PathBuf),
    /// An npm install, started as `node cli.js` rather than through its
    /// `claude.cmd` shim (which would go through cmd.exe and its quoting).
    Node { node: PathBuf, cli: PathBuf },
}

impl Launch {
    fn command(&self) -> tokio::process::Command {
        match self {
            Launch::Exe(exe) => tokio::process::Command::new(exe),
            Launch::Node { node, cli } => {
                let mut cmd = tokio::process::Command::new(node);
                cmd.arg(cli);
                cmd
            }
        }
    }
}

/// Finds Claude Code: `override_path` if set, else PATH, else the usual
/// install locations.
pub fn locate(override_path: &str) -> Option<Launch> {
    let path_dirs: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    locate_in(
        override_path,
        &path_dirs,
        dirs::home_dir().as_deref(),
        dirs::data_dir().as_deref(),
    )
}

#[cfg(windows)]
const NAMES: &[&str] = &["claude.exe", "claude.cmd"];
#[cfg(not(windows))]
const NAMES: &[&str] = &["claude"];

#[cfg(windows)]
const NODE: &str = "node.exe";
#[cfg(not(windows))]
const NODE: &str = "node";

fn locate_in(
    override_path: &str,
    path_dirs: &[PathBuf],
    home: Option<&Path>,
    appdata: Option<&Path>,
) -> Option<Launch> {
    let override_path = override_path.trim();
    if !override_path.is_empty() {
        let path = PathBuf::from(override_path);
        return path.is_file().then(|| from_file(path, path_dirs)).flatten();
    }
    let mut candidates: Vec<PathBuf> = path_dirs
        .iter()
        .flat_map(|dir| NAMES.iter().map(move |name| dir.join(name)))
        .collect();
    if let Some(home) = home {
        candidates.extend(
            NAMES
                .iter()
                .map(|n| home.join(".local").join("bin").join(n)),
        );
        candidates.extend(
            NAMES
                .iter()
                .map(|n| home.join(".claude").join("local").join(n)),
        );
    }
    if let Some(appdata) = appdata {
        candidates.push(appdata.join("npm").join("claude.cmd"));
    }
    // WinGet's links folder: on PATH, but only for programs started after
    // the install, which the launcher (started at login) may not be.
    if let Some(local) = home.map(|h| h.join("AppData").join("Local")) {
        candidates.push(
            local
                .join("Microsoft")
                .join("WinGet")
                .join("Links")
                .join(NAMES[0]),
        );
    }
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .and_then(|c| from_file(c, path_dirs))
}

fn from_file(path: PathBuf, path_dirs: &[PathBuf]) -> Option<Launch> {
    let ext = path
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        "js" | "mjs" | "cjs" => {
            find_node(path.parent(), path_dirs).map(|node| Launch::Node { node, cli: path })
        }
        "cmd" | "bat" => {
            // npm puts the package next to its shim.
            let dir = path.parent()?;
            let cli = dir
                .join("node_modules")
                .join("@anthropic-ai")
                .join("claude-code")
                .join("cli.js");
            match (cli.is_file(), find_node(Some(dir), path_dirs)) {
                (true, Some(node)) => Some(Launch::Node { node, cli }),
                // Last resort: run the shim itself.
                _ => Some(Launch::Exe(path)),
            }
        }
        _ => Some(Launch::Exe(path)),
    }
}

fn find_node(near: Option<&Path>, path_dirs: &[PathBuf]) -> Option<PathBuf> {
    near.into_iter()
        .chain(path_dirs.iter().map(PathBuf::as_path))
        .map(|dir| dir.join(NODE))
        .find(|p| p.is_file())
}

/// Everything that decides how a conversation's process starts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
    pub model: String,
    pub approvals: Approvals,
    pub billing: BillingMode,
    pub cwd: PathBuf,
    /// Continue an earlier session (after the idle process was shut down).
    pub resume: Option<String>,
    pub system_prompt: String,
}

/// Command-line arguments for `claude` (after any `node cli.js`).
pub fn args(options: &Options) -> Vec<String> {
    let mut args: Vec<String> = [
        "-p",
        "--output-format",
        "stream-json",
        "--verbose",
        "--input-format",
        "stream-json",
        "--include-partial-messages",
        // Permission prompts come to us as control requests on stdout.
        "--permission-prompt-tool",
        "stdio",
    ]
    .map(String::from)
    .to_vec();
    if !options.model.trim().is_empty() {
        args.extend(["--model".into(), options.model.trim().into()]);
    }
    if options.approvals == Approvals::Auto {
        args.extend(["--permission-mode".into(), "auto".into()]);
    }
    if let Some(session) = &options.resume {
        args.extend(["--resume".into(), session.clone()]);
    }
    if !options.system_prompt.is_empty() {
        args.extend([
            "--append-system-prompt".into(),
            options.system_prompt.clone(),
        ]);
    }
    // Never `--bare`: it ignores the subscription login.
    args
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

/// `claude` with API key variables removed in subscription mode, so the
/// plan pays. `window_flags` are Windows process creation flags.
fn base_command(
    launch: &Launch,
    billing: BillingMode,
    window_flags: u32,
) -> Result<tokio::process::Command, AiError> {
    let mut cmd = launch.command();
    match billing {
        BillingMode::Subscription => {
            cmd.env_remove("ANTHROPIC_API_KEY")
                .env_remove("ANTHROPIC_AUTH_TOKEN");
        }
        BillingMode::ApiKey => {
            if std::env::var_os("ANTHROPIC_API_KEY").is_none() {
                return Err(AiError::MissingApiKey);
            }
        }
    }
    #[cfg(windows)]
    cmd.creation_flags(window_flags);
    #[cfg(not(windows))]
    let _ = window_flags;
    Ok(cmd)
}

/// The ready-to-spawn conversation process: pipes, working folder, no
/// console window.
pub fn command(launch: &Launch, options: &Options) -> Result<tokio::process::Command, AiError> {
    let mut cmd = base_command(launch, options.billing, CREATE_NO_WINDOW)?;
    cmd.args(args(options))
        .current_dir(&options.cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    Ok(cmd)
}

/// Whether Claude Code can answer.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum ClaudeStatus {
    Ready,
    NotInstalled,
    SignedOut,
    /// The check didn't work (timed out, odd output): assume it can answer
    /// and let a real error say otherwise.
    Unknown,
}

/// Reads `claude auth status --json`.
pub fn parse_status(stdout: &str) -> ClaudeStatus {
    let parsed: Option<serde_json::Value> = serde_json::from_str(stdout.trim()).ok();
    match parsed.as_ref().and_then(|v| v["loggedIn"].as_bool()) {
        Some(true) => ClaudeStatus::Ready,
        Some(false) => ClaudeStatus::SignedOut,
        None => ClaudeStatus::Unknown,
    }
}

/// Checks that Claude Code is installed and signed in. Takes about a
/// second (it starts Claude Code), so run it in the background.
pub async fn check_status(settings: &ClaudeSettings) -> ClaudeStatus {
    let Some(launch) = locate(&settings.path) else {
        return ClaudeStatus::NotInstalled;
    };
    if settings.billing == BillingMode::ApiKey {
        // An API key needs no sign-in; a missing key is reported on asking.
        return ClaudeStatus::Ready;
    }
    let Ok(mut cmd) = base_command(&launch, settings.billing, CREATE_NO_WINDOW) else {
        return ClaudeStatus::Unknown;
    };
    cmd.args(["auth", "status", "--json"])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(output)) => parse_status(&String::from_utf8_lossy(&output.stdout)),
        Ok(Err(e)) => {
            tracing::warn!("could not check Claude Code's sign-in: {e}");
            ClaudeStatus::Unknown
        }
        Err(_) => {
            tracing::warn!("Claude Code's sign-in check timed out");
            ClaudeStatus::Unknown
        }
    }
}

/// Starts `claude auth login` in its own console window, where it opens the
/// browser to sign in. Returns the running process.
pub fn sign_in(settings: &ClaudeSettings) -> Result<tokio::process::Child, AiError> {
    let launch = locate(&settings.path).ok_or(AiError::NotInstalled)?;
    let mut cmd = base_command(&launch, BillingMode::Subscription, CREATE_NEW_CONSOLE)?;
    cmd.args(["auth", "login", "--claudeai"]);
    cmd.spawn().map_err(|e| AiError::Spawn(e.to_string()))
}

/// The last status check, so searching never waits for one.
pub struct StatusCache {
    last: Mutex<Option<(ClaudeStatus, Instant)>>,
    checking: std::sync::atomic::AtomicBool,
}

/// Re-check this often when everything was fine...
const RECHECK_READY: Duration = Duration::from_secs(30 * 60);
/// ...and this often while something needs fixing, so a fix shows up soon.
const RECHECK_BROKEN: Duration = Duration::from_secs(15);

impl Default for StatusCache {
    fn default() -> Self {
        Self::new()
    }
}

impl StatusCache {
    pub fn new() -> Self {
        Self {
            last: Mutex::new(None),
            checking: std::sync::atomic::AtomicBool::new(false),
        }
    }

    /// Claims the right to run a check; `false` if one is already running.
    pub fn begin_check(&self) -> bool {
        !self
            .checking
            .swap(true, std::sync::atomic::Ordering::AcqRel)
    }

    /// Stores a check's result and releases the claim.
    pub fn finish_check(&self, status: ClaudeStatus) {
        self.set(status);
        self.checking
            .store(false, std::sync::atomic::Ordering::Release);
    }

    /// The last result; `Unknown` before the first check finishes.
    pub fn get(&self) -> ClaudeStatus {
        self.last
            .lock()
            .ok()
            .and_then(|last| last.map(|(status, _)| status))
            .unwrap_or(ClaudeStatus::Unknown)
    }

    pub fn set(&self, status: ClaudeStatus) {
        if let Ok(mut last) = self.last.lock() {
            *last = Some((status, Instant::now()));
        }
    }

    /// Whether it's time to check again.
    pub fn is_stale(&self) -> bool {
        let Ok(last) = self.last.lock() else {
            return true;
        };
        match *last {
            None => true,
            Some((ClaudeStatus::Ready, at)) => at.elapsed() > RECHECK_READY,
            Some((_, at)) => at.elapsed() > RECHECK_BROKEN,
        }
    }
}

/// Where Claude Code is and how to start it, from the settings.
pub fn prepare(settings: &ClaudeSettings) -> Result<(Launch, Options), AiError> {
    let launch = locate(&settings.path).ok_or(AiError::NotInstalled)?;
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let options = Options {
        model: settings.model.clone(),
        approvals: settings.approvals,
        billing: settings.billing,
        system_prompt: system_prompt(&home),
        cwd: home,
        resume: None,
    };
    Ok((launch, options))
}

/// Tells Claude where it's running and how to answer.
pub fn system_prompt(home: &Path) -> String {
    format!(
        "You are answering inside a small desktop launcher popup on the user's PC, \
         not a terminal. Keep answers short and easy to skim; use Markdown sparingly. \
         The user's home folder is {}. When the user asks you to do something with \
         their files or apps (move, rename, sort, clean up), do it with your tools \
         rather than explaining how; the launcher asks the user to approve changes \
         when needed. A message may start with a <desktop_context> block, added \
         automatically: the window the user was in when they opened the launcher, \
         and for File Explorer the folder shown and the selected files. Words like \
         \"this\", \"these files\" and \"here\" refer to it. Use it when it's \
         relevant; don't mention it otherwise.",
        home.display()
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn options() -> Options {
        Options {
            model: "sonnet".into(),
            approvals: Approvals::Ask,
            billing: BillingMode::Subscription,
            cwd: PathBuf::from("."),
            resume: None,
            system_prompt: String::new(),
        }
    }

    #[test]
    fn builds_the_stream_json_command_line() {
        let a = args(&options());
        assert!(a
            .windows(2)
            .any(|w| w == ["--output-format", "stream-json"]));
        assert!(a.windows(2).any(|w| w == ["--input-format", "stream-json"]));
        assert!(a
            .windows(2)
            .any(|w| w == ["--permission-prompt-tool", "stdio"]));
        assert!(a.windows(2).any(|w| w == ["--model", "sonnet"]));
        assert!(a.contains(&"--include-partial-messages".to_string()));
        assert!(!a.contains(&"--permission-mode".to_string()));
        assert!(!a.contains(&"--bare".to_string()));
    }

    #[test]
    fn auto_approvals_resume_and_prompt() {
        let a = args(&Options {
            approvals: Approvals::Auto,
            resume: Some("abc".into()),
            system_prompt: "be brief".into(),
            model: " ".into(),
            ..options()
        });
        assert!(a.windows(2).any(|w| w == ["--permission-mode", "auto"]));
        assert!(a.windows(2).any(|w| w == ["--resume", "abc"]));
        assert!(a
            .windows(2)
            .any(|w| w == ["--append-system-prompt", "be brief"]));
        assert!(
            !a.contains(&"--model".to_string()),
            "blank model uses Claude's default"
        );
    }

    #[test]
    fn finds_claude_on_path_first() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(NAMES[0]);
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_in("", &[bin], Some(dir.path()), None),
            Some(Launch::Exe(exe))
        );
    }

    #[test]
    fn falls_back_to_the_native_install_folder() {
        let dir = tempfile::tempdir().unwrap();
        let local = dir.path().join(".local").join("bin");
        fs::create_dir_all(&local).unwrap();
        let exe = local.join(NAMES[0]);
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_in("", &[], Some(dir.path()), None),
            Some(Launch::Exe(exe))
        );
        assert_eq!(locate_in("", &[], None, None), None);
    }

    #[test]
    fn runs_npm_installs_through_node() {
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        let cli_dir = npm
            .join("node_modules")
            .join("@anthropic-ai")
            .join("claude-code");
        fs::create_dir_all(&cli_dir).unwrap();
        fs::write(npm.join("claude.cmd"), b"").unwrap();
        fs::write(cli_dir.join("cli.js"), b"").unwrap();
        let node_dir = dir.path().join("nodejs");
        fs::create_dir_all(&node_dir).unwrap();
        fs::write(node_dir.join(NODE), b"").unwrap();

        let launch = from_file(npm.join("claude.cmd"), std::slice::from_ref(&node_dir));
        assert_eq!(
            launch,
            Some(Launch::Node {
                node: node_dir.join(NODE),
                cli: cli_dir.join("cli.js")
            })
        );
    }

    #[test]
    fn reads_the_sign_in_status() {
        assert_eq!(
            parse_status(r#"{"loggedIn": true, "authMethod": "oauth_token"}"#),
            ClaudeStatus::Ready
        );
        assert_eq!(
            parse_status("{\n  \"loggedIn\": false,\n  \"authMethod\": \"none\"\n}\n"),
            ClaudeStatus::SignedOut
        );
        assert_eq!(parse_status("Usage: claude ..."), ClaudeStatus::Unknown);
    }

    #[test]
    fn rechecks_sooner_while_broken() {
        let cache = StatusCache::new();
        assert!(cache.is_stale());
        assert_eq!(cache.get(), ClaudeStatus::Unknown);
        cache.set(ClaudeStatus::SignedOut);
        assert_eq!(cache.get(), ClaudeStatus::SignedOut);
        assert!(!cache.is_stale());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn checks_status_with_the_real_command_line() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("claude");
        // Answers like `claude auth status --json` only for those arguments.
        fs::write(
            &script,
            "#!/bin/sh\n[ \"$*\" = \"auth status --json\" ] && echo '{\"loggedIn\": false}' && exit 1\nexit 2\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let settings = ClaudeSettings {
            path: script.display().to_string(),
            ..ClaudeSettings::default()
        };
        assert_eq!(check_status(&settings).await, ClaudeStatus::SignedOut);
        let missing = ClaudeSettings {
            path: "/nope/claude".into(),
            ..ClaudeSettings::default()
        };
        assert_eq!(check_status(&missing).await, ClaudeStatus::NotInstalled);
    }

    #[test]
    fn honors_an_override_path() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("my-claude");
        fs::write(&custom, b"").unwrap();
        let found = locate_in(&custom.display().to_string(), &[], None, None);
        assert_eq!(found, Some(Launch::Exe(custom)));
        assert_eq!(locate_in("/nope/claude", &[], None, None), None);
    }
}
