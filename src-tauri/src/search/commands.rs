//! Built-in commands such as "Quit" and "Open Settings".

use async_trait::async_trait;

use super::{Action, ItemKind, ResultItem, SearchError, SearchProvider};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinCommand {
    OpenSettings,
    OpenLogFolder,
    Quit,
}

impl BuiltinCommand {
    pub const ALL: [Self; 3] = [Self::OpenSettings, Self::OpenLogFolder, Self::Quit];

    pub fn id(self) -> &'static str {
        match self {
            Self::OpenSettings => "cmd:settings",
            Self::OpenLogFolder => "cmd:logs",
            Self::Quit => "cmd:quit",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    fn title(self) -> &'static str {
        match self {
            Self::OpenSettings => "Open Settings",
            Self::OpenLogFolder => "Open Log Folder",
            Self::Quit => "Quit Desktop Assistant",
        }
    }

    /// Extra words that should find this command.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::OpenSettings => &["preferences", "config", "options"],
            Self::OpenLogFolder => &["debug", "logs"],
            Self::Quit => &["exit", "close"],
        }
    }
}

pub struct CommandProvider;

#[async_trait]
impl SearchProvider for CommandProvider {
    fn name(&self) -> &'static str {
        "commands"
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let q = query.to_lowercase();
        let mut items: Vec<ResultItem> = BuiltinCommand::ALL
            .into_iter()
            .filter_map(|cmd| {
                let score = score(&q, cmd.title(), cmd.keywords())?;
                Some(ResultItem {
                    id: cmd.id().into(),
                    kind: ItemKind::Command,
                    title: cmd.title().into(),
                    subtitle: "Command".into(),
                    score,
                    actions: vec![Action::Run],
                })
            })
            .collect();
        items.truncate(limit);
        Ok(items)
    }
}

/// Simple word matching. Real fuzzy ranking arrives with the app launcher (M2).
fn score(q: &str, title: &str, keywords: &[&str]) -> Option<f32> {
    let title = title.to_lowercase();
    if title.starts_with(q) {
        return Some(0.9);
    }
    let words = title.split_whitespace().chain(keywords.iter().copied());
    if words.into_iter().any(|w| w.starts_with(q)) {
        return Some(0.7);
    }
    title.contains(q).then_some(0.4)
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn titles(q: &str) -> Vec<String> {
        CommandProvider
            .query(q, 8)
            .await
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect()
    }

    #[tokio::test]
    async fn matches_title_prefix_word_and_keyword() {
        assert_eq!(titles("open s").await, ["Open Settings"]);
        assert_eq!(titles("quit").await, ["Quit Desktop Assistant"]);
        assert_eq!(titles("exit").await, ["Quit Desktop Assistant"]);
        assert_eq!(titles("log").await, ["Open Log Folder"]);
    }

    #[tokio::test]
    async fn no_match_is_empty() {
        assert!(titles("zzz").await.is_empty());
    }

    #[test]
    fn ids_round_trip() {
        for cmd in BuiltinCommand::ALL {
            assert_eq!(BuiltinCommand::from_id(cmd.id()), Some(cmd));
        }
    }
}
