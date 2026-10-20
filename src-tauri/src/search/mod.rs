//! Search: providers produce results, the aggregator merges them.

pub mod apps;
pub mod commands;
pub mod files;
pub mod item;
pub mod rank;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::join_all;

use crate::ai::ProviderId;
pub use item::{Action, ItemKind, ResultItem};

/// Time each provider gets per keystroke. Slower providers are skipped for
/// that keystroke rather than holding up the results.
pub const PROVIDER_BUDGET: Duration = Duration::from_millis(40);
/// Providers return this many times the visible limit, so frecency can lift
/// a weaker match that's opened often.
const CANDIDATE_FACTOR: usize = 4;

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("{provider} failed: {message}")]
    Provider {
        provider: &'static str,
        message: String,
    },
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// `query` is already trimmed and non-empty.
    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError>;
}

pub struct Aggregator {
    providers: Vec<Arc<dyn SearchProvider>>,
    budget: Duration,
}

impl Aggregator {
    pub fn new(providers: Vec<Arc<dyn SearchProvider>>) -> Self {
        Self {
            providers,
            budget: PROVIDER_BUDGET,
        }
    }

    #[cfg(test)]
    fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    /// Runs every provider in parallel, ranks the merged results (see
    /// `rank.rs`), and always appends an "Ask <provider>" row so unmatched
    /// text can go to the AI. `frecency` gives each id's open history.
    pub async fn search(
        &self,
        raw: &str,
        limit: usize,
        ai: ProviderId,
        frecency: &(dyn Fn(&str) -> f64 + Sync),
    ) -> Vec<ResultItem> {
        let query = raw.trim();
        if query.is_empty() {
            return Vec::new();
        }

        let pool = limit.saturating_mul(CANDIDATE_FACTOR);
        let runs = self.providers.iter().map(|p| {
            let p = Arc::clone(p);
            async move {
                match tokio::time::timeout(self.budget, p.query(query, pool)).await {
                    Ok(Ok(items)) => items,
                    Ok(Err(e)) => {
                        tracing::warn!("{e}");
                        Vec::new()
                    }
                    Err(_) => {
                        tracing::debug!("{} skipped: over {:?} budget", p.name(), self.budget);
                        Vec::new()
                    }
                }
            }
        });

        let mut items: Vec<ResultItem> = join_all(runs).await.into_iter().flatten().collect();
        rank::rank(&mut items, query, frecency);
        items.truncate(limit);
        items.push(ask_ai_item(query, ai));
        items
    }
}

fn ask_ai_item(query: &str, ai: ProviderId) -> ResultItem {
    let name = match ai {
        ProviderId::Claude => "Claude",
        ProviderId::Chatgpt => "ChatGPT",
    };
    ResultItem {
        id: "ask-ai".into(),
        kind: ItemKind::AskAi,
        title: format!("Ask {name}: {query}"),
        subtitle: "Tab".into(),
        icon: None,
        score: 0.0,
        actions: vec![Action::AskAi],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fixed(&'static str, Vec<f32>, Duration);

    #[async_trait]
    impl SearchProvider for Fixed {
        fn name(&self) -> &'static str {
            self.0
        }
        async fn query(&self, _: &str, _: usize) -> Result<Vec<ResultItem>, SearchError> {
            tokio::time::sleep(self.2).await;
            Ok(self
                .1
                .iter()
                .enumerate()
                .map(|(i, &score)| ResultItem {
                    id: format!("{}:{i}", self.0),
                    kind: ItemKind::Command,
                    title: format!("{} {i}", self.0),
                    subtitle: String::new(),
                    icon: None,
                    score,
                    actions: vec![Action::Run],
                })
                .collect())
        }
    }

    struct Failing;

    #[async_trait]
    impl SearchProvider for Failing {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn query(&self, _: &str, _: usize) -> Result<Vec<ResultItem>, SearchError> {
            Err(SearchError::Provider {
                provider: "failing",
                message: "boom".into(),
            })
        }
    }

    fn no_history(_: &str) -> f64 {
        0.0
    }

    fn agg(providers: Vec<Arc<dyn SearchProvider>>) -> Aggregator {
        Aggregator::new(providers).with_budget(Duration::from_millis(50))
    }

    fn ids(items: &[ResultItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    #[tokio::test]
    async fn empty_query_returns_nothing() {
        let a = agg(vec![Arc::new(Fixed("a", vec![1.0], Duration::ZERO))]);
        assert!(a
            .search("   ", 8, ProviderId::Claude, &no_history)
            .await
            .is_empty());
    }

    #[tokio::test]
    async fn merges_by_score_and_appends_ask_row() {
        let a = agg(vec![
            Arc::new(Fixed("a", vec![0.2, 0.9], Duration::ZERO)),
            Arc::new(Fixed("b", vec![0.5], Duration::ZERO)),
        ]);
        let items = a.search("x", 8, ProviderId::Claude, &no_history).await;
        assert_eq!(ids(&items), ["a:1", "b:0", "a:0", "ask-ai"]);
        assert_eq!(items[3].title, "Ask Claude: x");
    }

    #[tokio::test]
    async fn truncates_before_ask_row() {
        let a = agg(vec![Arc::new(Fixed(
            "a",
            vec![0.1, 0.2, 0.3],
            Duration::ZERO,
        ))]);
        let items = a.search("x", 2, ProviderId::Chatgpt, &no_history).await;
        assert_eq!(ids(&items), ["a:2", "a:1", "ask-ai"]);
        assert_eq!(items[2].title, "Ask ChatGPT: x");
    }

    #[tokio::test]
    async fn history_reorders_results() {
        let a = agg(vec![Arc::new(Fixed("a", vec![0.9, 0.8], Duration::ZERO))]);
        let used = |id: &str| if id == "a:1" { 20.0 } else { 0.0 };
        let items = a.search("x", 8, ProviderId::Claude, &used).await;
        assert_eq!(ids(&items), ["a:1", "a:0", "ask-ai"]);
    }

    #[tokio::test]
    async fn slow_and_failing_providers_are_skipped() {
        let a = agg(vec![
            Arc::new(Fixed("fast", vec![0.5], Duration::ZERO)),
            Arc::new(Fixed("slow", vec![1.0], Duration::from_secs(5))),
            Arc::new(Failing),
        ]);
        let items = a.search("x", 8, ProviderId::Claude, &no_history).await;
        assert_eq!(ids(&items), ["fast:0", "ask-ai"]);
    }
}
