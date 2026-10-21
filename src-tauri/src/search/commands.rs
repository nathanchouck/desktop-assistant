//! Built-in commands such as "Quit" and "Open Settings".

use async_trait::async_trait;

use super::{Action, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::fuzzy::FuzzyQuery;

/// Keyword matches count a little less than the title.
const KEYWORD_WEIGHT: f32 = 0.9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BuiltinCommand {
    OpenSettings,
    OpenLogFolder,
    ReindexApps,
    SignInToClaude,
    InstallClaude,
    Quit,
}

impl BuiltinCommand {
    pub const ALL: [Self; 6] = [
        Self::OpenSettings,
        Self::OpenLogFolder,
        Self::ReindexApps,
        Self::SignInToClaude,
        Self::InstallClaude,
        Self::Quit,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::OpenSettings => "cmd:settings",
            Self::OpenLogFolder => "cmd:logs",
            Self::ReindexApps => "cmd:reindex",
            Self::SignInToClaude => "cmd:claude-sign-in",
            Self::InstallClaude => "cmd:claude-install",
            Self::Quit => "cmd:quit",
        }
    }

    pub fn from_id(id: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|c| c.id() == id)
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::OpenSettings => "Open Settings",
            Self::OpenLogFolder => "Open Log Folder",
            Self::ReindexApps => "Reindex Apps",
            Self::SignInToClaude => "Sign In to Claude Code",
            Self::InstallClaude => "Install Claude Code",
            Self::Quit => "Quit Desktop Assistant",
        }
    }

    /// Extra words that should find this command.
    fn keywords(self) -> &'static [&'static str] {
        match self {
            Self::OpenSettings => &["preferences", "config", "options"],
            Self::OpenLogFolder => &["debug", "logs"],
            Self::ReindexApps => &["refresh", "rescan", "rebuild"],
            Self::SignInToClaude => &["login", "log in", "claude account"],
            Self::InstallClaude => &["setup", "download claude"],
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
        let mut fuzzy = FuzzyQuery::new(query);
        let mut items: Vec<ResultItem> = BuiltinCommand::ALL
            .into_iter()
            .filter_map(|cmd| {
                let title = fuzzy.score(cmd.title());
                let keyword = cmd
                    .keywords()
                    .iter()
                    .filter_map(|k| fuzzy.score(k))
                    .map(|s| s * KEYWORD_WEIGHT)
                    .reduce(f32::max);
                let score = title.into_iter().chain(keyword).reduce(f32::max)?;
                Some(ResultItem {
                    id: cmd.id().into(),
                    kind: ItemKind::Command,
                    title: cmd.title().into(),
                    subtitle: "Command".into(),
                    icon: None,
                    score,
                    actions: vec![Action::Run],
                })
            })
            .collect();
        items.sort_by(|a, b| b.score.total_cmp(&a.score));
        items.truncate(limit);
        Ok(items)
    }
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
    async fn matches_title_and_keywords() {
        assert_eq!(titles("open s").await[0], "Open Settings");
        assert_eq!(titles("quit").await[0], "Quit Desktop Assistant");
        assert_eq!(titles("exit").await[0], "Quit Desktop Assistant");
        assert_eq!(titles("log").await[0], "Open Log Folder");
        assert_eq!(titles("reindex").await[0], "Reindex Apps");
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
