//! A conversation's CLI process, whichever provider it is: its stdin, the
//! conversation its events belong to, and what's left of stderr when it
//! crashes. The provider sessions (`claude/session.rs`, `codex/session.rs`)
//! read its stdout and speak their own protocol over it.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;

use super::{AiError, AiEvent};

/// Where events go: (conversation id, event).
pub type EventSink = mpsc::UnboundedSender<(u32, AiEvent)>;

/// Lines of stderr kept for error messages.
const STDERR_LINES: usize = 20;

/// Which conversation a process belongs to. `id` 0: an unclaimed spare,
/// whose events are held until a conversation claims it.
#[derive(Default)]
struct Owner {
    id: u32,
    held: Vec<AiEvent>,
}

pub struct Process {
    /// How messages name the program ("Claude Code", "Codex").
    pub name: &'static str,
    owner: StdMutex<Owner>,
    /// `None` once closed, which asks the CLI to exit.
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
    sink: EventSink,
    /// A turn is running (between sending a message and its end).
    pub busy: AtomicBool,
    pub alive: AtomicBool,
}

/// The last lines a process wrote to stderr, collected in the background.
pub struct StderrTail {
    lines: Arc<StdMutex<VecDeque<String>>>,
    task: JoinHandle<()>,
}

impl StderrTail {
    /// The tail as text, after giving the reader a moment to catch up so a
    /// crash message is complete.
    pub async fn text(self) -> String {
        let _ = tokio::time::timeout(Duration::from_millis(500), self.task).await;
        self.lines
            .lock()
            .map(|t| t.iter().cloned().collect::<Vec<_>>().join("\n"))
            .unwrap_or_default()
    }
}

/// Starts `cmd` (with piped stdio) for `conversation` (0: a spare).
pub fn spawn(
    name: &'static str,
    mut cmd: Command,
    conversation: u32,
    sink: EventSink,
) -> Result<(Arc<Process>, ChildStdout, StderrTail), AiError> {
    let mut child = cmd
        .spawn()
        .map_err(|e| AiError::Spawn(name, e.to_string()))?;
    let (Some(stdin), Some(stdout), Some(stderr)) =
        (child.stdin.take(), child.stdout.take(), child.stderr.take())
    else {
        return Err(AiError::Spawn(name, "missing stdio pipes".into()));
    };
    if conversation == 0 {
        tracing::info!("started a warm {name} process");
    } else {
        tracing::info!("started {name} for conversation {conversation}");
    }
    let process = Arc::new(Process {
        name,
        owner: StdMutex::new(Owner {
            id: conversation,
            held: Vec::new(),
        }),
        stdin: Mutex::new(Some(stdin)),
        child: Mutex::new(child),
        sink,
        busy: AtomicBool::new(false),
        alive: AtomicBool::new(true),
    });

    let lines = Arc::new(StdMutex::new(VecDeque::new()));
    let tail = Arc::clone(&lines);
    let task = tokio::spawn(async move {
        let mut reader = BufReader::new(stderr).lines();
        while let Ok(Some(line)) = reader.next_line().await {
            tracing::debug!("{name} stderr: {line}");
            if let Ok(mut tail) = tail.lock() {
                if tail.len() == STDERR_LINES {
                    tail.pop_front();
                }
                tail.push_back(line);
            }
        }
    });
    Ok((process, stdout, StderrTail { lines, task }))
}

impl Process {
    pub fn is_alive(&self) -> bool {
        self.alive.load(Ordering::Acquire)
    }

    pub fn is_busy(&self) -> bool {
        self.busy.load(Ordering::Acquire)
    }

    /// Sends `event` to its conversation, or holds it while unclaimed.
    pub fn emit(&self, event: AiEvent) {
        let Ok(mut owner) = self.owner.lock() else {
            return;
        };
        if owner.id == 0 {
            owner.held.push(event);
        } else {
            let _ = self.sink.send((owner.id, event));
        }
    }

    /// Hands a spare to conversation `id`, with what it said so far.
    pub fn adopt(&self, id: u32) {
        if let Ok(mut owner) = self.owner.lock() {
            owner.id = id;
            for event in owner.held.drain(..) {
                let _ = self.sink.send((id, event));
            }
        }
    }

    pub async fn send_line(&self, line: &str) -> Result<(), AiError> {
        let mut stdin = self.stdin.lock().await;
        let Some(stdin) = stdin.as_mut() else {
            return Err(AiError::Process(self.name, "it was closed".into()));
        };
        let write = async {
            stdin.write_all(line.as_bytes()).await?;
            stdin.write_all(b"\n").await?;
            stdin.flush().await
        };
        write
            .await
            .map_err(|e| AiError::Process(self.name, format!("could not send to it: {e}")))
    }

    pub async fn kill(&self) {
        self.alive.store(false, Ordering::Release);
        let _ = self.child.lock().await.kill().await;
    }

    /// Closes stdin, which asks the CLI to exit (and take its own child
    /// processes with it); kills it if it's still there after `grace`.
    pub async fn close(&self, grace: Duration) {
        self.alive.store(false, Ordering::Release);
        self.stdin.lock().await.take();
        let mut child = self.child.lock().await;
        if tokio::time::timeout(grace, child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
    }

    /// Called when stdout ends: the process is gone. A turn that was
    /// running ends with what stderr said.
    pub async fn exited(&self, tail: StderrTail) {
        self.alive.store(false, Ordering::Release);
        let detail = tail.text().await;
        if self.busy.swap(false, Ordering::AcqRel) {
            let message = if detail.trim().is_empty() {
                format!("{} exited unexpectedly.", self.name)
            } else {
                format!("{} exited unexpectedly: {}", self.name, detail.trim())
            };
            self.emit(AiEvent::Error { message });
        }
    }
}

/// The stdout lines of a process, as they come.
pub fn lines(stdout: ChildStdout) -> tokio::io::Lines<BufReader<ChildStdout>> {
    BufReader::new(stdout).lines()
}
