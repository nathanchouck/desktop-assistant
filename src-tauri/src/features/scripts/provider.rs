//! Script commands in search (rows from `rows.rs`), and running one when
//! it's picked: in the background for silent scripts, else waiting for
//! what it prints.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::header::{ScriptMode, WorkDir};
use super::index::Script;
use super::language::command_line;
use super::library::Library;
use super::output::{self, Outcome};
use super::rows::{parse_id, reload_message, rows, OPEN_FOLDER_ID, PREFIX, RELOAD_ID};
use super::run::{self, Request};
use crate::desktop::DesktopTracker;
use crate::platform::Shell;
use crate::search::{Action, Activation, ResultItem, SearchError, SearchProvider};
use crate::store::Store;

const NAME: &str = "scripts";
/// How long `@cwd explorer` waits for File Explorer's folder.
const EXPLORER_WAIT: Duration = Duration::from_millis(800);

pub struct ScriptProvider {
    library: Arc<Library>,
    shell: Arc<Shell>,
    desktop: Arc<DesktopTracker>,
    store: Arc<Store>,
}

impl ScriptProvider {
    pub fn new(
        library: Arc<Library>,
        shell: Arc<Shell>,
        desktop: Arc<DesktopTracker>,
        store: Arc<Store>,
    ) -> Self {
        Self {
            library,
            shell,
            desktop,
            store,
        }
    }
}

fn failed(message: impl ToString) -> SearchError {
    SearchError::Provider {
        provider: NAME,
        message: message.to_string(),
    }
}

fn notice(message: impl Into<String>) -> Activation {
    Activation::Notice {
        message: message.into(),
    }
}

/// Opens the scripts folder in File Explorer, making it first if needed
/// (then it has the README and the example, a starting point).
pub async fn open_folder(library: &Library, shell: &Shell) -> Result<(), String> {
    let folder = library.ensure_folder()?;
    shell
        .open_path(folder.display().to_string())
        .await
        .map_err(|e| format!("Couldn't open {}: {e}", folder.display()))
}

impl ScriptProvider {
    /// Runs `script`: in the background for silent scripts (a toast tells
    /// how it went), else waiting for its output.
    async fn run(&self, id: &str, script: &Script, argument: Option<&str>) -> Activation {
        let header = &script.header;
        let cwd = match header.cwd {
            WorkDir::Script => script
                .path
                .parent()
                .map(PathBuf::from)
                .unwrap_or_else(|| self.library.folder()),
            WorkDir::Explorer => match self.desktop.current(EXPLORER_WAIT).await.folder() {
                Some(folder) => PathBuf::from(folder),
                None => {
                    return notice(format!(
                        "{} runs in File Explorer's folder: open the launcher from File Explorer.",
                        header.title
                    ))
                }
            },
        };
        let Some(running) = self.library.claim(&script.file) else {
            return notice(format!("{} is still running.", header.title));
        };
        let (program, args) = command_line(&script.runner, &script.path, argument);
        let timeout = Duration::from_secs(u64::from(self.library.settings().timeout_secs.max(1)));
        let request = Request {
            program,
            args,
            cwd,
            timeout,
        };
        tracing::info!("running script {}", script.file);
        let title = header.title.clone();
        if header.mode == ScriptMode::Silent {
            let library = Arc::clone(&self.library);
            tauri::async_runtime::spawn(async move {
                let outcome = run::run(request).await;
                drop(running);
                match output::result(&title, &outcome) {
                    Ok(stdout) => library.toast_info(output::last_line(&title, &stdout)),
                    Err(message) => library.toast_error(message),
                }
            });
            return Activation::Hide;
        }
        let outcome = run::run(request).await;
        drop(running);
        self.finish(id, script, &outcome).await
    }

    /// What follows a copy, paste or output script.
    async fn finish(&self, id: &str, script: &Script, outcome: &Outcome) -> Activation {
        let title = &script.header.title;
        let text = match output::result(title, outcome) {
            Ok(text) => text,
            Err(message) => {
                tracing::warn!("{message}");
                return notice(message);
            }
        };
        let cut = matches!(outcome, Outcome::Finished { cut: true, .. });
        match script.header.mode {
            ScriptMode::Copy | ScriptMode::Paste if text.trim().is_empty() => {
                notice(format!("{title} didn't print anything."))
            }
            ScriptMode::Copy => Activation::Copy { text },
            ScriptMode::Paste => Activation::Paste { text: Some(text) },
            ScriptMode::Silent | ScriptMode::Output => {
                let short = output::as_notice(&text).or_else(|| {
                    text.trim()
                        .is_empty()
                        .then(|| format!("{title} finished without printing anything."))
                });
                if let Some(message) = short {
                    // A notice keeps the launcher open, and such opens
                    // aren't counted for ranking: count this run here.
                    if let Err(e) = self.store.record_open(id) {
                        tracing::debug!("could not record a script run: {e}");
                    }
                    return notice(message);
                }
                match self.open_as_file(script, &text, cut).await {
                    Ok(()) => Activation::Hide,
                    Err(message) => notice(message),
                }
            }
        }
    }

    /// Long output goes to a text file, opened in your text editor.
    async fn open_as_file(&self, script: &Script, text: &str, cut: bool) -> Result<(), String> {
        let path = self.library.output_file(&script.file);
        let mut body = text.to_owned();
        if cut {
            body.push_str("\n\n[The output was cut at 1 MB.]\n");
        }
        let write = |path: &std::path::Path| -> std::io::Result<()> {
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)?;
            }
            std::fs::write(path, body.as_bytes())
        };
        write(&path)
            .map_err(|e| format!("Couldn't save the output of {}: {e}", script.header.title))?;
        self.shell
            .open_path(path.display().to_string())
            .await
            .map_err(|e| format!("Couldn't open the output of {}: {e}", script.header.title))
    }

    async fn open_folder(&self) -> Result<Activation, SearchError> {
        open_folder(&self.library, &self.shell)
            .await
            .map_err(failed)?;
        Ok(Activation::Hide)
    }
}

#[async_trait]
impl SearchProvider for ScriptProvider {
    fn name(&self) -> &'static str {
        NAME
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        if !self.library.settings().enabled {
            return Ok(Vec::new());
        }
        let mut found = rows(&self.library.index(), query);
        found.truncate(limit);
        Ok(found)
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(PREFIX) || id.starts_with("scripts:")
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        match id {
            OPEN_FOLDER_ID => return self.open_folder().await,
            RELOAD_ID => {
                let index = self.library.rescan().await;
                return Ok(notice(reload_message(&index)));
            }
            _ => {}
        }
        let (file, argument) =
            parse_id(id).ok_or_else(|| failed(format!("{id} isn't a script")))?;
        let index = self.library.index();
        let script = index
            .scripts
            .iter()
            .find(|s| s.file == file)
            .ok_or_else(|| failed(format!("{file} isn't a script command any more")))?;
        match action {
            Action::RevealInExplorer => {
                self.shell
                    .reveal_path(script.path.display().to_string())
                    .await
                    .map_err(failed)?;
                Ok(Activation::Hide)
            }
            Action::CopyPath => Ok(Activation::Copy {
                text: script.path.display().to_string(),
            }),
            _ => Ok(self.run(id, script, argument).await),
        }
    }
}

#[cfg(test)]
mod tests;
