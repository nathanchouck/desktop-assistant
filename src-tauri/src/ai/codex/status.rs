//! Whether Codex is installed and signed in to ChatGPT (`codex login
//! status`), and signing in (`codex login` in a console window).

use std::process::Stdio;
use std::time::Duration;

use serde::Serialize;

use super::{base_command, locate, CREATE_NEW_CONSOLE, CREATE_NO_WINDOW, NAME};
use crate::ai::status::Readiness;
use crate::ai::{AiError, BillingMode};
use crate::settings::ChatGptSettings;

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

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

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
