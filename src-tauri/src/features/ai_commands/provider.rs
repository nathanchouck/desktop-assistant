//! Search rows: saved AI commands, "Ask About Screen Region" and
//! "Dictate"; and what Enter does on them.

use std::time::{Duration, Instant};

use async_trait::async_trait;
use tauri::AppHandle;

use super::saved::{self, AiCommand};
use super::template::{self, Missing, Source, Values};
use super::{screen, selection};
use crate::features::clipboard::system;
use crate::features::system::matching;
use crate::fuzzy::FuzzyQuery;
use crate::platform::{vk, Shell};
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::settings::{self, SharedSettings};
use crate::window;

pub const SCREEN_REGION_ID: &str = "ai-input:screen-region";
pub const DICTATE_ID: &str = "ai-input:dictate";
const INPUT_PREFIX: &str = "ai-input:";
/// Between the search box clearing and Win+H, so voice typing starts in
/// the empty box.
const DICTATE_DELAY: Duration = Duration::from_millis(150);

/// The fixed commands: (id, title, subtitle, glyph, keywords).
const INPUTS: [(&str, &str, &str, &str, &[&str]); 2] = [
    (
        SCREEN_REGION_ID,
        "Ask About Screen Region",
        "Snip part of the screen and ask the AI about it",
        "screenshot",
        &["screenshot", "snip", "screen capture", "ask screen", "region"],
    ),
    (
        DICTATE_ID,
        "Dictate",
        "Windows voice typing in the search box (Win+H)",
        "mic",
        &["voice typing", "voice input", "speech", "talk", "microphone"],
    ),
];

pub struct AiCommandProvider {
    settings: SharedSettings,
    shell: std::sync::Arc<Shell>,
}

impl AiCommandProvider {
    pub fn new(settings: SharedSettings, shell: std::sync::Arc<Shell>) -> Self {
        Self { settings, shell }
    }

    /// Fills a saved command's prompt and asks; or says what's missing.
    async fn run(&self, app: Option<&AppHandle>, name: &str, text: Option<&str>) -> Activation {
        let settings = settings::current(&self.settings).ai_commands;
        let Some(command) = settings.commands.iter().find(|c| c.name == name) else {
            return notice(format!("“{name}” isn't a saved AI command any more."));
        };
        if text.is_none() && template::requires_text(&command.prompt) {
            return Activation::SetQuery {
                query: saved::typing_prompt(command),
            };
        }
        let needs = template::needs(&command.prompt, text.is_some());
        let clipboard = if needs.clipboard {
            tokio::task::spawn_blocking(system::read_text)
                .await
                .ok()
                .flatten()
        } else {
            None
        };
        let picked = match (needs.selection, app) {
            (true, Some(app)) => {
                let grabbed = selection::grab_behind_launcher(app, settings.restore_clipboard).await;
                // Back to the launcher, where the answer shows.
                window::show(app, Instant::now());
                match grabbed {
                    Ok(copied) => copied.and_then(selection::as_text),
                    Err(e) => {
                        tracing::warn!("could not copy the selection: {e}");
                        None
                    }
                }
            }
            _ => None,
        };
        let values = Values {
            selection: picked.as_deref(),
            clipboard: clipboard.as_deref(),
            text,
            now: chrono::Local::now().naive_local(),
        };
        match template::fill(&command.prompt, &values) {
            Ok(prompt) => Activation::Ask {
                prompt,
                provider: command.provider.provider(),
            },
            Err(missing) => missing_activation(command, &missing),
        }
    }
}

fn notice(message: impl Into<String>) -> Activation {
    Activation::Notice {
        message: message.into(),
    }
}

/// What to do when a placeholder had nothing: let you type the text, or
/// say what's missing.
fn missing_activation(command: &AiCommand, Missing(sources): &Missing) -> Activation {
    if sources.contains(&Source::Text) && sources.len() == 1 {
        return Activation::SetQuery {
            query: saved::typing_prompt(command),
        };
    }
    let wants = |s| sources.contains(&s);
    let message = match (wants(Source::Selection), wants(Source::Clipboard), wants(Source::Text)) {
        (true, true, _) => "Nothing is selected, and the clipboard has no text.",
        (true, false, _) => "Nothing is selected in the window you came from.",
        (false, true, true) => "Type the text after the command's name, or copy some first.",
        _ => "The clipboard has no text to use.",
    };
    notice(message)
}

fn input_rows(query: &str) -> Vec<ResultItem> {
    if !matching::long_enough(query) {
        return Vec::new();
    }
    let mut fuzzy = FuzzyQuery::new(query);
    INPUTS
        .iter()
        .filter_map(|(id, title, subtitle, icon, keywords)| {
            let score = matching::score(&mut fuzzy, title, keywords)?;
            Some(
                ResultItem::new(*id, ItemKind::Command, *title)
                    .with_subtitle(*subtitle)
                    .with_icon(glyph(icon))
                    .with_score(score)
                    .with_actions([Action::Run]),
            )
        })
        .collect()
}

/// Presses Win+H (Windows voice typing) into whatever has the keyboard:
/// the launcher's search box.
pub async fn voice_typing(shell: &Shell) -> Result<(), String> {
    shell
        .press_keys(vec![vk::LWIN, vk::H])
        .await
        .map_err(|e| format!("Couldn't start voice typing: {e}"))
}

#[async_trait]
impl SearchProvider for AiCommandProvider {
    fn name(&self) -> &'static str {
        "ai commands"
    }

    async fn query(&self, query: &str, _limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let settings = settings::current(&self.settings);
        if !settings.ai_commands.enabled {
            return Ok(Vec::new());
        }
        let mut rows = saved::rows(
            &settings.ai_commands.commands,
            query,
            settings.ai.default_provider,
        );
        rows.extend(input_rows(query.trim()));
        Ok(rows)
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(saved::PREFIX) || id.starts_with(INPUT_PREFIX)
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        let app = super::app();
        match id {
            SCREEN_REGION_ID => Ok(match app {
                Some(app) if cfg!(windows) => {
                    screen::start(&app);
                    Activation::Hide
                }
                _ => notice("Snipping the screen needs Windows."),
            }),
            DICTATE_ID => {
                if !cfg!(windows) {
                    return Ok(notice("Voice typing needs Windows."));
                }
                let shell = std::sync::Arc::clone(&self.shell);
                tauri::async_runtime::spawn(async move {
                    tokio::time::sleep(DICTATE_DELAY).await;
                    if let Err(e) = voice_typing(&shell).await {
                        tracing::warn!("{e}");
                    }
                });
                // An empty search box for what you say.
                Ok(Activation::SetQuery {
                    query: String::new(),
                })
            }
            _ => match saved::parse_id(id) {
                Some((name, text)) => Ok(self.run(app.as_ref(), name, text).await),
                None => Err(SearchError::Provider {
                    provider: self.name(),
                    message: format!("{id} isn't an AI command"),
                }),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, RwLock};

    fn provider() -> AiCommandProvider {
        AiCommandProvider::new(
            Arc::new(RwLock::new(crate::settings::Settings::default())),
            Arc::new(Shell::new()),
        )
    }

    fn ids(items: &[ResultItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[tokio::test]
    async fn finds_commands_and_inputs() {
        let p = provider();
        let found = p.query("screenshot", 8).await.unwrap();
        assert_eq!(ids(&found), [SCREEN_REGION_ID]);
        let found = p.query("dictate", 8).await.unwrap();
        assert_eq!(ids(&found), [DICTATE_ID]);
        assert_eq!(found[0].icon.as_deref(), Some("glyph:mic"));
        let found = p.query("unread", 8).await.unwrap();
        assert_eq!(ids(&found), ["ai-cmd:Unread Email"]);
        for (_, _, _, icon, _) in INPUTS {
            assert!(matching::glyph_exists(icon), "{icon}");
        }
        assert!(p.owns("ai-cmd:Fix Grammar") && p.owns(DICTATE_ID) && !p.owns("cmd:quit"));
    }

    #[tokio::test]
    async fn turned_off_finds_nothing() {
        let p = provider();
        if let Ok(mut s) = p.settings.write() {
            s.ai_commands.enabled = false;
        }
        assert!(p.query("dictate", 8).await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn typed_text_fills_the_prompt() {
        let p = provider();
        let activation = p
            .activate("ai-cmd:Search Drive\tbudget 2026", Action::AskAi)
            .await
            .unwrap();
        let Activation::Ask { prompt, provider } = activation else {
            panic!("{activation:?}");
        };
        assert!(prompt.starts_with("Search my Google Drive for: budget 2026\n"), "{prompt}");
        assert_eq!(provider, Some(crate::ai::ProviderId::Claude));
        // Calendar needs nothing fetched.
        let activation = p.activate("ai-cmd:Today's Calendar", Action::AskAi).await.unwrap();
        assert!(matches!(activation, Activation::Ask { .. }), "{activation:?}");
    }

    #[tokio::test]
    async fn missing_text_lets_you_type_it() {
        let p = provider();
        assert_eq!(
            p.activate("ai-cmd:Search Drive", Action::AskAi).await.unwrap(),
            Activation::SetQuery {
                query: "drive ".into()
            }
        );
        assert!(matches!(
            p.activate("ai-cmd:Gone", Action::AskAi).await.unwrap(),
            Activation::Notice { .. }
        ));
        assert!(p.activate("ai-input:nope", Action::Run).await.is_err());
    }

    #[test]
    fn says_what_is_missing() {
        let command = saved::defaults().remove(0);
        let ask = |sources: Vec<Source>| missing_activation(&command, &Missing(sources));
        assert_eq!(
            ask(vec![Source::Text]),
            Activation::SetQuery {
                query: "fix ".into()
            }
        );
        let Activation::Notice { message } = ask(vec![Source::Text, Source::Clipboard]) else {
            panic!("not a notice");
        };
        assert!(message.contains("copy some"), "{message}");
        let Activation::Notice { message } = ask(vec![Source::Selection]) else {
            panic!("not a notice");
        };
        assert!(message.contains("Nothing is selected"), "{message}");
    }
}
