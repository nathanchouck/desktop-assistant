//! "Check for Updates" and, once an update is downloaded, "Install Update
//! and Restart" in search.

use std::sync::Arc;

use async_trait::async_trait;

use super::manager::Updates;
use super::tracker::check_notice;
use crate::fuzzy::FuzzyQuery;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};

pub const CHECK_ID: &str = "update:check";
pub const INSTALL_ID: &str = "update:install";
/// Keyword matches count a little less than the title (as for commands).
const KEYWORD_WEIGHT: f32 = 0.9;

pub struct UpdateProvider {
    updates: Arc<Updates>,
}

impl UpdateProvider {
    pub fn new(updates: Arc<Updates>) -> Self {
        Self { updates }
    }
}

/// The rows offered: check always, install only with an update ready.
/// `ready` is the downloaded version.
fn rows(ready: Option<&str>, check_subtitle: String) -> Vec<(ResultItem, &'static [&'static str])> {
    let check = ResultItem::new(CHECK_ID, ItemKind::Command, "Check for Updates")
        .with_subtitle(check_subtitle)
        .with_icon(glyph("update"))
        .with_actions([Action::Run]);
    let mut rows = vec![(
        check,
        &["update", "updates", "upgrade", "new version", "version"][..],
    )];
    if let Some(version) = ready {
        let install = ResultItem::new(INSTALL_ID, ItemKind::Command, "Install Update and Restart")
            .with_subtitle(format!("Installs v{version}, then opens it again"))
            .with_icon(glyph("restart"))
            .with_actions([Action::Run]);
        rows.push((install, &["update", "upgrade", "restart", "install"][..]));
    }
    rows
}

/// Scores each row by its title or keywords; rows that don't match are left out.
fn matching(query: &str, rows: Vec<(ResultItem, &[&str])>) -> Vec<ResultItem> {
    let mut fuzzy = FuzzyQuery::new(query);
    rows.into_iter()
        .filter_map(|(item, keywords)| {
            let title = fuzzy.score(&item.title);
            let keyword = keywords
                .iter()
                .filter_map(|k| fuzzy.score(k))
                .map(|s| s * KEYWORD_WEIGHT)
                .reduce(f32::max);
            let score = title.into_iter().chain(keyword).reduce(f32::max)?;
            Some(item.with_score(score))
        })
        .collect()
}

#[async_trait]
impl SearchProvider for UpdateProvider {
    fn name(&self) -> &'static str {
        "updater"
    }

    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let status = self.updates.status();
        let subtitle = format!("Version {} · {}", status.current_version, status.summary);
        let mut items = matching(query, rows(self.updates.ready().as_deref(), subtitle));
        items.truncate(limit);
        Ok(items)
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with("update:")
    }

    async fn activate(&self, id: &str, _action: Action) -> Result<Activation, SearchError> {
        let message = match id {
            CHECK_ID => {
                let result = self.updates.check().await;
                check_notice(&result, &self.updates.current_version())
            }
            INSTALL_ID => match self.updates.ready() {
                Some(version) => {
                    self.updates.install_in_background();
                    format!("Installing v{version}… Desktop Assistant will restart.")
                }
                None => "No update is downloaded yet.".into(),
            },
            _ => {
                return Err(SearchError::Provider {
                    provider: self.name(),
                    message: format!("{id} can't be opened"),
                })
            }
        };
        Ok(Activation::Notice { message })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(items: &[ResultItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[test]
    fn install_row_only_with_an_update_ready() {
        let found = |q: &str, ready: Option<&str>| matching(q, rows(ready, String::new()));
        assert_eq!(ids(&found("check for updates", None)), [CHECK_ID]);
        assert_eq!(ids(&found("update", None)), [CHECK_ID]);
        assert!(found("install update", None).is_empty());
        let both = found("update", Some("0.2.0"));
        assert_eq!(ids(&both), [CHECK_ID, INSTALL_ID]);
        assert_eq!(both[1].subtitle, "Installs v0.2.0, then opens it again");
        assert_eq!(ids(&found("restart", Some("0.2.0"))), [INSTALL_ID]);
        assert!(found("zzz", Some("0.2.0")).is_empty());
    }

    #[test]
    fn rows_run_on_enter() {
        for (item, _) in rows(Some("0.2.0"), "s".into()) {
            assert_eq!(item.actions, [Action::Run]);
            assert!(item.id.starts_with("update:"));
        }
    }
}
