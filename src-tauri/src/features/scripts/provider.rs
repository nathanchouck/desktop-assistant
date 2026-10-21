//! Script commands in search, found by title or keyword ("say hello"),
//! with an argument after the title ("say hello Ana") for scripts that
//! take one. Also "Open Scripts Folder" and "Reload Scripts".

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;

use super::header::{Header, ScriptMode, WorkDir};
use super::index::{Index, Script};
use super::language::command_line;
use super::library::Library;
use super::output::{self, Outcome};
use super::run::{self, Request};
use crate::desktop::DesktopTracker;
use crate::fuzzy::FuzzyQuery;
use crate::platform::Shell;
use crate::search::{
    glyph, text_icon, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider,
};
use crate::store::Store;

const NAME: &str = "scripts";
/// A script: `script:<file>`, or `script:<file>:<text>` with an argument.
const PREFIX: &str = "script:";
const OPEN_FOLDER_ID: &str = "scripts:open-folder";
const RELOAD_ID: &str = "scripts:reload";
/// "say hello Ana": you typed the title on purpose, so above apps.
const ARGUMENT_SCORE: f32 = 1.5;
/// The same through a keyword ("hi Ana").
const KEYWORD_ARGUMENT_SCORE: f32 = 1.4;
/// Keyword matches count a little less than the title.
const KEYWORD_WEIGHT: f32 = 0.9;
/// "scripts" lists every script, below anything named like that.
const LIST_SCORE: f32 = 0.4;
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

/// A row's icon: `@icon` as a glyph name (`script`, `globe`) or as an
/// emoji; the script glyph without one.
pub fn icon(header: &Header) -> String {
    match header.icon.as_deref() {
        Some(name)
            if name
                .chars()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-') =>
        {
            glyph(name)
        }
        Some(text) => text_icon(&text.chars().take(8).collect::<String>()),
        None => glyph("script"),
    }
}

/// The script id for `file`, with `argument` if given.
pub fn script_id(file: &str, argument: Option<&str>) -> String {
    match argument {
        Some(text) => format!("{PREFIX}{file}:{text}"),
        None => format!("{PREFIX}{file}"),
    }
}

/// (file, argument) from a script id.
pub fn parse_id(id: &str) -> Option<(&str, Option<&str>)> {
    let rest = id.strip_prefix(PREFIX)?;
    Some(match rest.split_once(':') {
        Some((file, text)) => (file, Some(text)),
        None => (rest, None),
    })
}

/// The text after `head` in `query` ("say hello Ana" after "Say Hello"),
/// ignoring case; `None` unless a space and some text follow.
pub fn text_after<'q>(query: &'q str, head: &str) -> Option<&'q str> {
    let mut rest = query.char_indices();
    for h in head.trim().chars() {
        let (_, q) = rest.next()?;
        if !q.to_lowercase().eq(h.to_lowercase()) {
            return None;
        }
    }
    let (i, sep) = rest.next()?;
    if !sep.is_whitespace() {
        return None;
    }
    let text = query[i..].trim();
    (!text.is_empty()).then_some(text)
}

fn row(script: &Script, score: f32, argument: Option<&str>) -> ResultItem {
    let header = &script.header;
    let subtitle = match argument {
        Some(text) => format!("Run with “{text}”"),
        None if header.description.is_empty() => script.file.clone(),
        None => header.description.clone(),
    };
    let first = match header.mode {
        ScriptMode::Copy => Action::Copy,
        ScriptMode::Paste => Action::Paste,
        ScriptMode::Silent | ScriptMode::Output => Action::Run,
    };
    let item = ResultItem::new(
        script_id(&script.file, argument),
        ItemKind::Script,
        &header.title,
    )
    .with_subtitle(subtitle)
    .with_icon(icon(header))
    .with_score(score)
    .with_actions([first, Action::RevealInExplorer, Action::CopyPath]);
    match (&header.argument, argument) {
        (Some(name), None) => item.with_accessory(format!("‹{name}›")),
        _ => item,
    }
}

/// "Open Scripts Folder" and "Reload Scripts", if the query finds them.
fn commands(fuzzy: &mut FuzzyQuery, folder: &str) -> Vec<ResultItem> {
    let commands = [
        (
            OPEN_FOLDER_ID,
            "Open Scripts Folder",
            folder.to_owned(),
            "folder",
            Action::Open,
            &["script commands", "edit scripts", "add script"][..],
        ),
        (
            RELOAD_ID,
            "Reload Scripts",
            "Read the scripts folder again".to_owned(),
            "restart",
            Action::Run,
            &["refresh scripts", "rescan scripts"][..],
        ),
    ];
    commands
        .into_iter()
        .filter_map(|(id, title, subtitle, icon, action, keywords)| {
            let by_keyword = keywords
                .iter()
                .filter_map(|k| fuzzy.score(k))
                .map(|s| s * KEYWORD_WEIGHT)
                .reduce(f32::max);
            let score = fuzzy
                .score(title)
                .into_iter()
                .chain(by_keyword)
                .reduce(f32::max)?;
            Some(
                ResultItem::new(id, ItemKind::Command, title)
                    .with_subtitle(subtitle)
                    .with_icon(glyph(icon))
                    .with_score(score)
                    .with_actions([action]),
            )
        })
        .collect()
}

/// Every row `query` finds, best first.
pub fn rows(index: &Index, query: &str) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(query);
    let listing = ["script", "scripts"]
        .iter()
        .any(|w| query.trim().eq_ignore_ascii_case(w));
    let mut found: Vec<ResultItem> = index
        .scripts
        .iter()
        .filter_map(|script| {
            let header = &script.header;
            if header.argument.is_some() {
                let typed = text_after(query, &header.title)
                    .map(|text| (text, ARGUMENT_SCORE))
                    .or_else(|| {
                        header.keywords.iter().find_map(|k| {
                            text_after(query, k).map(|text| (text, KEYWORD_ARGUMENT_SCORE))
                        })
                    });
                if let Some((text, score)) = typed {
                    return Some(row(script, score, Some(text)));
                }
            }
            let by_keyword = header
                .keywords
                .iter()
                .filter_map(|k| fuzzy.score(k))
                .map(|s| s * KEYWORD_WEIGHT)
                .reduce(f32::max);
            let score = fuzzy
                .score(&header.title)
                .into_iter()
                .chain(by_keyword)
                .chain(listing.then_some(LIST_SCORE))
                .reduce(f32::max)?;
            Some(row(script, score, None))
        })
        .collect();
    found.extend(commands(&mut fuzzy, &index.folder.display().to_string()));
    found.sort_by(|a, b| b.score.total_cmp(&a.score));
    found
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

/// "Found 3 script commands", and whether some files aren't commands.
pub fn reload_message(index: &Index) -> String {
    if let Some(error) = &index.folder_error {
        return format!("Scripts folder: {error}");
    }
    let found = match index.scripts.len() {
        1 => "Found 1 script command".to_owned(),
        n => format!("Found {n} script commands"),
    };
    match index.problems.len() {
        0 => format!("{found}."),
        1 => format!("{found}; 1 file isn't one (see Settings)."),
        n => format!("{found}; {n} files aren't (see Settings)."),
    }
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
