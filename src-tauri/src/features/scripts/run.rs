//! Running one script: no window, output captured (capped), stopped
//! together with what it started when it runs too long.

use std::ffi::OsString;
use std::path::PathBuf;
use std::process::Stdio;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::task::JoinHandle;

use super::output::{Capped, Outcome};

/// Output kept. A result may be pasted somewhere, so it can be long, but
/// a runaway loop mustn't fill memory.
const STDOUT_CAP: usize = 1024 * 1024;
/// The end of stderr, where the error is.
const STDERR_CAP: usize = 64 * 1024;
/// After the script exits, how long its last output may take to arrive.
/// A program it started and left running (an editor, a server) can hold
/// the pipes open for good, so their end isn't waited for.
const DRAIN: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Request {
    pub program: PathBuf,
    pub args: Vec<OsString>,
    pub cwd: PathBuf,
    pub timeout: Duration,
}

pub async fn run(request: Request) -> Outcome {
    let mut cmd = tokio::process::Command::new(&request.program);
    cmd.args(&request.args)
        .current_dir(&request.cwd)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    // Python writes to a pipe in the ANSI code page (and fails on emoji)
    // unless told otherwise.
    if std::env::var_os("PYTHONIOENCODING").is_none() {
        cmd.env("PYTHONIOENCODING", "utf-8");
    }
    #[cfg(windows)]
    {
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = match cmd.spawn() {
        Ok(child) => child,
        Err(e) => return Outcome::NotStarted(e.to_string()),
    };
    let stdout = Arc::new(Mutex::new(Capped::start(STDOUT_CAP)));
    let stderr = Arc::new(Mutex::new(Capped::end(STDERR_CAP)));
    let readers: Vec<JoinHandle<()>> = [
        child
            .stdout
            .take()
            .map(|s| tokio::spawn(pump(s, Arc::clone(&stdout)))),
        child
            .stderr
            .take()
            .map(|s| tokio::spawn(pump(s, Arc::clone(&stderr)))),
    ]
    .into_iter()
    .flatten()
    .collect();

    let ended = tokio::time::timeout(request.timeout, child.wait()).await;
    let code = match ended {
        Ok(Ok(status)) => status.code(),
        Ok(Err(e)) => {
            stop(&mut child).await;
            finish(readers).await;
            return Outcome::NotStarted(e.to_string());
        }
        Err(_) => {
            stop(&mut child).await;
            finish(readers).await;
            return Outcome::TimedOut {
                after: request.timeout,
            };
        }
    };
    finish(readers).await;
    let (stdout, cut) = text(&stdout);
    let (stderr, _) = text(&stderr);
    Outcome::Finished {
        code,
        stdout,
        stderr,
        cut,
    }
}

/// Copies a stream into `into` until it ends.
async fn pump(mut from: impl AsyncRead + Unpin, into: Arc<Mutex<Capped>>) {
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match from.read(&mut buf).await {
            Ok(0) | Err(_) => break,
            Ok(n) => match into.lock() {
                Ok(mut capped) => capped.push(&buf[..n]),
                Err(_) => break,
            },
        }
    }
}

/// Lets the readers take in what's left in the pipes, briefly.
async fn finish(readers: Vec<JoinHandle<()>>) {
    let deadline = tokio::time::Instant::now() + DRAIN;
    for mut reader in readers {
        if tokio::time::timeout_at(deadline, &mut reader)
            .await
            .is_err()
        {
            reader.abort();
        }
    }
}

fn text(capped: &Mutex<Capped>) -> (String, bool) {
    capped.lock().map(|c| c.text()).unwrap_or_default()
}

/// Ends the script, and on Windows everything it started.
async fn stop(child: &mut tokio::process::Child) {
    #[cfg(windows)]
    if let Some(pid) = child.id() {
        super::win::kill_tree(pid).await;
    }
    if let Err(e) = child.start_kill() {
        tracing::debug!("could not stop a script: {e}");
    }
    let _ = tokio::time::timeout(Duration::from_secs(2), child.wait()).await;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::time::Instant;

    fn bash(script: &str, timeout: Duration) -> Request {
        Request {
            program: "bash".into(),
            args: vec!["-c".into(), script.into(), "bash".into(), "Ana & co".into()],
            cwd: std::env::temp_dir(),
            timeout,
        }
    }

    #[tokio::test]
    async fn captures_output_argument_and_folder() {
        let dir = tempfile::tempdir().unwrap();
        let mut request = bash(
            "echo \"hi $1\"; pwd; echo oops >&2",
            Duration::from_secs(10),
        );
        request.cwd = dir.path().to_path_buf();
        let Outcome::Finished {
            code,
            stdout,
            stderr,
            cut,
        } = run(request).await
        else {
            panic!("expected it to finish");
        };
        assert_eq!(code, Some(0));
        let folder = dir.path().canonicalize().unwrap();
        assert_eq!(stdout, format!("hi Ana & co\n{}\n", folder.display()));
        assert_eq!(stderr, "oops\n");
        assert!(!cut);
    }

    #[tokio::test]
    async fn reports_the_exit_code() {
        let outcome = run(bash("echo bad >&2; exit 3", Duration::from_secs(10))).await;
        assert!(
            matches!(outcome, Outcome::Finished { code: Some(3), ref stderr, .. } if stderr == "bad\n")
        );
    }

    #[tokio::test]
    async fn stops_a_script_that_runs_too_long() {
        let started = Instant::now();
        let outcome = run(bash("exec sleep 30", Duration::from_millis(200))).await;
        assert_eq!(
            outcome,
            Outcome::TimedOut {
                after: Duration::from_millis(200)
            }
        );
        assert!(started.elapsed() < Duration::from_secs(5));
    }

    #[tokio::test]
    async fn doesnt_wait_for_programs_the_script_left_running() {
        let started = Instant::now();
        let outcome = run(bash("sleep 3 & echo started", Duration::from_secs(10))).await;
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "{:?}",
            started.elapsed()
        );
        assert!(
            matches!(outcome, Outcome::Finished { code: Some(0), ref stdout, .. } if stdout == "started\n")
        );
    }

    #[tokio::test]
    async fn caps_long_output() {
        let outcome = run(bash(
            "head -c 2000000 /dev/zero | tr '\\0' 'x'",
            Duration::from_secs(10),
        ))
        .await;
        let Outcome::Finished { stdout, cut, .. } = outcome else {
            panic!("expected it to finish");
        };
        assert!(cut);
        assert_eq!(stdout.len(), STDOUT_CAP);
    }

    #[tokio::test]
    async fn a_missing_program_doesnt_start() {
        let mut request = bash("", Duration::from_secs(1));
        request.program = "/nonexistent/interpreter".into();
        assert!(matches!(run(request).await, Outcome::NotStarted(_)));
    }
}
