//! ChatGPT through OpenAI's Codex CLI: where it's installed, how a
//! conversation's `codex app-server` starts, and the sign-in check.
//! `protocol.rs` and `parser.rs` speak its JSON-RPC; `session.rs` drives
//! one process.
//!
//! `codex app-server` is the interface with streamed text, approvals we can
//! answer, interrupts and resuming a thread (`codex exec --json` has none
//! of those).

pub mod parser;
pub mod protocol;
pub mod session;

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

use super::launch::{find_node, has_extension, path_dirs, read_shim, Launch};
use super::status::Readiness;
use super::{AiError, Approvals, BillingMode};
use crate::settings::ChatGptSettings;

/// How messages name it.
pub const NAME: &str = "Codex";

/// Variables that would sign Codex in with an API key (or token) instead of
/// ChatGPT, or that commands Codex runs could read. Always removed from its
/// environment; in API key mode the key goes over stdin instead.
pub const SECRET_ENV: [&str; 3] = ["OPENAI_API_KEY", "CODEX_API_KEY", "CODEX_ACCESS_TOKEN"];

/// Where API key mode reads the key from, in order.
const KEY_ENV: [&str; 2] = ["OPENAI_API_KEY", "CODEX_API_KEY"];

#[cfg(windows)]
const NAMES: &[&str] = &["codex.exe", "codex.cmd"];
#[cfg(not(windows))]
const NAMES: &[&str] = &["codex"];

/// Finds Codex: `override_path` if set, else PATH, else the usual install
/// locations (standalone installer, npm, WinGet).
pub fn locate(override_path: &str) -> Option<Launch> {
    locate_in(
        override_path,
        &path_dirs(),
        dirs::home_dir().as_deref(),
        dirs::data_dir().as_deref(),
    )
}

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
        let local = home.join("AppData").join("Local");
        // The standalone installer's bin folder, and the release it links to.
        candidates.push(local.join("Programs/OpenAI/Codex/bin").join(NAMES[0]));
        candidates.push(
            home.join(".codex/packages/standalone/current/bin")
                .join(NAMES[0]),
        );
        // WinGet's links folder: on PATH, but only for programs started
        // after the install, which the launcher (started at login) may not be.
        candidates.push(local.join("Microsoft/WinGet/Links").join(NAMES[0]));
    }
    if let Some(appdata) = appdata {
        candidates.push(appdata.join("npm").join("codex.cmd"));
    }
    candidates
        .into_iter()
        .find(|c| c.is_file())
        .and_then(|c| from_file(c, path_dirs))
}

fn from_file(path: PathBuf, path_dirs: &[PathBuf]) -> Option<Launch> {
    if has_extension(&path, &["cmd", "bat"]) {
        // npm's shim runs `node …\@openai\codex\bin\codex.js`, which starts
        // the native exe npm installed beside it. Start that exe directly:
        // through node it would open a console window of its own.
        return match read_shim(&path) {
            Some(script) if has_extension(&script, &["js", "mjs", "cjs"]) => native_beside(&script)
                .map(Launch::Exe)
                .or_else(|| {
                    find_node(path.parent(), path_dirs)
                        .map(|node| Launch::Node { node, cli: script })
                })
                .or(Some(Launch::Exe(path))),
            Some(exe) => Some(Launch::Exe(exe)),
            None => Some(Launch::Exe(path)),
        };
    }
    if has_extension(&path, &["js", "mjs", "cjs"]) {
        return native_beside(&path).map(Launch::Exe).or_else(|| {
            find_node(path.parent(), path_dirs).map(|node| Launch::Node { node, cli: path })
        });
    }
    Some(Launch::Exe(path))
}

/// `…\@openai\codex\bin\codex.js` → the native exe of the platform package
/// npm installed with it (nested for global installs, hoisted otherwise).
fn native_beside(codex_js: &Path) -> Option<PathBuf> {
    let package = codex_js.parent()?.parent()?;
    let (name, triple, exe) = platform_package();
    let inside = Path::new("vendor").join(triple).join("bin").join(exe);
    [
        package
            .join("node_modules")
            .join("@openai")
            .join(name)
            .join(&inside),
        package.parent()?.join(name).join(&inside),
    ]
    .into_iter()
    .find(|p| p.is_file())
}

/// (npm package, target triple, exe name) of Codex's native build here.
fn platform_package() -> (&'static str, &'static str, &'static str) {
    if cfg!(windows) {
        if cfg!(target_arch = "aarch64") {
            ("codex-win32-arm64", "aarch64-pc-windows-msvc", "codex.exe")
        } else {
            ("codex-win32-x64", "x86_64-pc-windows-msvc", "codex.exe")
        }
    } else if cfg!(target_os = "macos") {
        ("codex-darwin-arm64", "aarch64-apple-darwin", "codex")
    } else {
        ("codex-linux-x64", "x86_64-unknown-linux-musl", "codex")
    }
}

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

/// Whether Codex can answer with ChatGPT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, specta::Type)]
#[serde(rename_all = "snake_case")]
pub enum CodexStatus {
    Ready,
    NotInstalled,
    /// Not signed in, or signed in with an API key (which would bill per
    /// token, not the ChatGPT plan).
    SignedOut,
    /// The check didn't work: assume it can answer and let a real error
    /// say otherwise.
    Unknown,
}

impl Readiness for CodexStatus {
    const UNKNOWN: Self = CodexStatus::Unknown;
    fn is_ready(self) -> bool {
        self == CodexStatus::Ready
    }
}

/// Reads `codex login status`, which answers on stderr ("Logged in using
/// ChatGPT", "Not logged in", ...) and exits 0 only when signed in.
pub fn parse_login_status(success: bool, stderr: &str) -> CodexStatus {
    let line = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("Logged in") || l.starts_with("Not logged in"))
        .unwrap_or_default();
    if line.starts_with("Not logged in") {
        return CodexStatus::SignedOut;
    }
    if !success || line.is_empty() {
        return CodexStatus::Unknown;
    }
    // An API key login bills per token; ChatGPT, access tokens and the
    // like use the plan.
    if line.contains("API key") {
        CodexStatus::SignedOut
    } else {
        CodexStatus::Ready
    }
}

/// Checks that Codex is installed and signed in to ChatGPT. Fast (it reads
/// a local file), but it starts Codex, so run it in the background.
pub async fn check_status(settings: &ChatGptSettings) -> CodexStatus {
    let Some(launch) = locate(&settings.path) else {
        return CodexStatus::NotInstalled;
    };
    if settings.billing == BillingMode::ApiKey {
        // An API key needs no sign-in; a missing key is reported on asking.
        return CodexStatus::Ready;
    }
    let mut cmd = base_command(&launch, CREATE_NO_WINDOW);
    cmd.args(["login", "status"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    match tokio::time::timeout(Duration::from_secs(20), cmd.output()).await {
        Ok(Ok(output)) => parse_login_status(
            output.status.success(),
            &String::from_utf8_lossy(&output.stderr),
        ),
        Ok(Err(e)) => {
            tracing::warn!("could not check Codex's sign-in: {e}");
            CodexStatus::Unknown
        }
        Err(_) => {
            tracing::warn!("Codex's sign-in check timed out");
            CodexStatus::Unknown
        }
    }
}

/// Starts `codex login` in its own console window, where it opens the
/// browser to sign in to ChatGPT. Returns the running process.
pub fn sign_in(settings: &ChatGptSettings) -> Result<tokio::process::Child, AiError> {
    let launch = locate(&settings.path).ok_or(AiError::CodexNotInstalled)?;
    let mut cmd = base_command(&launch, CREATE_NEW_CONSOLE);
    cmd.arg("login");
    cmd.spawn().map_err(|e| AiError::Spawn(NAME, e.to_string()))
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
    use std::fs;

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
    fn reads_the_login_status() {
        assert_eq!(
            parse_login_status(true, "Logged in using ChatGPT\n"),
            CodexStatus::Ready
        );
        assert_eq!(
            parse_login_status(false, "Not logged in\n"),
            CodexStatus::SignedOut
        );
        assert_eq!(
            parse_login_status(true, "Logged in using an API key - sk-test-***bcdef\n"),
            CodexStatus::SignedOut,
            "an API key login doesn't use the ChatGPT plan"
        );
        assert_eq!(
            parse_login_status(true, "WARNING: something\nLogged in using access token\n"),
            CodexStatus::Ready
        );
        assert_eq!(
            parse_login_status(
                false,
                "Error checking login status: invalid agent identity JWT format"
            ),
            CodexStatus::Unknown
        );
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

    #[test]
    fn finds_codex_on_path_first() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(NAMES[0]);
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_in("", &[bin], Some(dir.path()), None),
            Some(Launch::Exe(exe))
        );
        assert_eq!(locate_in("", &[], None, None), None);
    }

    #[test]
    fn falls_back_to_the_standalone_install() {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("AppData/Local/Programs/OpenAI/Codex/bin");
        fs::create_dir_all(&bin).unwrap();
        let exe = bin.join(NAMES[0]);
        fs::write(&exe, b"").unwrap();
        assert_eq!(
            locate_in("", &[], Some(dir.path()), None),
            Some(Launch::Exe(exe))
        );
    }

    /// What npm's cmd-shim writes for `@openai/codex`.
    const SHIM: &str = "@ECHO off\r\nGOTO start\r\n:find_dp0\r\nSET dp0=%~dp0\r\nEXIT /b\r\n:start\r\nSETLOCAL\r\nCALL :find_dp0\r\n\r\nIF EXIST \"%dp0%\\node.exe\" (\r\n  SET \"_prog=%dp0%\\node.exe\"\r\n) ELSE (\r\n  SET \"_prog=node\"\r\n  SET PATHEXT=%PATHEXT:;.JS;=;%\r\n)\r\n\r\nendLocal & goto #_undefined_# 2>NUL || title %COMSPEC% & \"%_prog%\"  \"%dp0%\\node_modules\\@openai\\codex\\bin\\codex.js\" %*\r\n";

    #[test]
    fn runs_the_native_exe_behind_the_npm_shim() {
        let dir = tempfile::tempdir().unwrap();
        let npm = dir.path().join("npm");
        let package = npm.join("node_modules/@openai/codex");
        fs::create_dir_all(package.join("bin")).unwrap();
        fs::write(package.join("bin/codex.js"), b"").unwrap();
        fs::write(npm.join("codex.cmd"), SHIM).unwrap();

        // Without the platform package: node runs the script.
        let node_dir = dir.path().join("nodejs");
        fs::create_dir_all(&node_dir).unwrap();
        fs::write(node_dir.join(crate::ai::launch::NODE), b"").unwrap();
        assert_eq!(
            from_file(npm.join("codex.cmd"), std::slice::from_ref(&node_dir)),
            Some(Launch::Node {
                node: node_dir.join(crate::ai::launch::NODE),
                cli: package.join("bin/codex.js"),
            })
        );

        // A global install nests the native build inside the package.
        let (name, triple, exe) = platform_package();
        let native = package
            .join("node_modules/@openai")
            .join(name)
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(exe);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"").unwrap();
        assert_eq!(
            from_file(npm.join("codex.cmd"), &[]),
            Some(Launch::Exe(native))
        );
    }

    #[test]
    fn finds_a_hoisted_native_build() {
        let dir = tempfile::tempdir().unwrap();
        let modules = dir.path().join("node_modules/@openai");
        let script = modules.join("codex/bin/codex.js");
        fs::create_dir_all(script.parent().unwrap()).unwrap();
        fs::write(&script, b"").unwrap();
        let (name, triple, exe) = platform_package();
        let native = modules
            .join(name)
            .join("vendor")
            .join(triple)
            .join("bin")
            .join(exe);
        fs::create_dir_all(native.parent().unwrap()).unwrap();
        fs::write(&native, b"").unwrap();
        assert_eq!(native_beside(&script), Some(native));
    }

    #[test]
    fn honors_an_override_path() {
        let dir = tempfile::tempdir().unwrap();
        let custom = dir.path().join("my-codex");
        fs::write(&custom, b"").unwrap();
        assert_eq!(
            locate_in(&custom.display().to_string(), &[], None, None),
            Some(Launch::Exe(custom))
        );
        assert_eq!(locate_in("/nope/codex", &[], None, None), None);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn checks_status_with_the_real_command_line() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("codex");
        // Answers like `codex login status` only for those arguments, and
        // fails if a key leaked into its environment.
        fs::write(
            &script,
            "#!/bin/sh\n[ -n \"$OPENAI_API_KEY$CODEX_API_KEY\" ] && exit 3\n[ \"$*\" = \"login status\" ] && echo 'Logged in using ChatGPT' >&2 && exit 0\nexit 2\n",
        )
        .unwrap();
        fs::set_permissions(&script, fs::Permissions::from_mode(0o755)).unwrap();
        let settings = ChatGptSettings {
            path: script.display().to_string(),
            ..ChatGptSettings::default()
        };
        // Exec'ing a file just written can fail now and then ("text file
        // busy") while another test thread starts a process: that's Unknown.
        let mut status = CodexStatus::Unknown;
        for _ in 0..5 {
            status = check_status(&settings).await;
            if status != CodexStatus::Unknown {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        assert_eq!(status, CodexStatus::Ready);
        let missing = ChatGptSettings {
            path: "/nope/codex".into(),
            ..ChatGptSettings::default()
        };
        assert_eq!(check_status(&missing).await, CodexStatus::NotInstalled);
    }
}
