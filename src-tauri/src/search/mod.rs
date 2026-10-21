//! Search: providers produce results, the aggregator merges them.

pub mod apps;
pub mod commands;
pub mod files;
pub mod item;
pub mod rank;

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::future::BoxFuture;
use futures::FutureExt;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::ai::ProviderId;
use commands::BuiltinCommand;
pub use item::{Action, ItemKind, ResultItem};

/// What the last row offers: asking the AI, or fixing what stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskRow {
    Ask(ProviderId),
    InstallClaude,
    SignInToClaude,
}

/// Time the providers get per keystroke before results are shown. A slower
/// provider keeps going, and its results follow (see `Outcome::complete`).
pub const PROVIDER_BUDGET: Duration = Duration::from_millis(40);
/// A provider still running this long after the first results is given up on.
const LATE_LIMIT: Duration = Duration::from_secs(3);
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

/// How often each result id has been opened (see `store`).
pub type Frecency = Arc<dyn Fn(&str) -> f64 + Send + Sync>;

pub struct Outcome {
    /// Ranked results from every provider that answered within the budget,
    /// plus the "Ask" row.
    pub items: Vec<ResultItem>,
    /// Set when a provider missed the budget: the full ranked list, once it
    /// finishes. Await it and send it to the UI as an update.
    pub complete: Option<BoxFuture<'static, Vec<ResultItem>>>,
}

pub struct Aggregator {
    providers: Vec<Arc<dyn SearchProvider>>,
    budget: Duration,
    late_limit: Duration,
}

impl Aggregator {
    pub fn new(providers: Vec<Arc<dyn SearchProvider>>) -> Self {
        Self {
            providers,
            budget: PROVIDER_BUDGET,
            late_limit: LATE_LIMIT,
        }
    }

    #[cfg(test)]
    fn with_budget(mut self, budget: Duration) -> Self {
        self.budget = budget;
        self
    }

    #[cfg(test)]
    fn with_late_limit(mut self, late_limit: Duration) -> Self {
        self.late_limit = late_limit;
        self
    }

    /// Runs every provider in parallel, ranks the merged results (see
    /// `rank.rs`), and always appends an "Ask <provider>" row so unmatched
    /// text can go to the AI. Providers that miss the budget aren't dropped:
    /// their results come in `Outcome::complete`.
    pub async fn search(&self, raw: &str, limit: usize, ai: AskRow, frecency: Frecency) -> Outcome {
        let query = raw.trim().to_owned();
        if query.is_empty() {
            return Outcome {
                items: Vec::new(),
                complete: None,
            };
        }

        let pool = limit.saturating_mul(CANDIDATE_FACTOR);
        let runs: Vec<(&'static str, JoinHandle<Vec<ResultItem>>)> = self
            .providers
            .iter()
            .map(|p| {
                let name = p.name();
                let p = Arc::clone(p);
                let q = query.clone();
                let run = tokio::spawn(async move {
                    p.query(&q, pool).await.unwrap_or_else(|e| {
                        tracing::warn!("{e}");
                        Vec::new()
                    })
                });
                (name, run)
            })
            .collect();

        let deadline = Instant::now() + self.budget;
        let (found, pending) = collect_until(runs, deadline).await;
        let items = finish(found.clone(), &query, limit, ai, &*frecency);
        let complete = if pending.is_empty() {
            None
        } else {
            for (name, _) in &pending {
                tracing::debug!("{name} missed the {:?} budget; results follow", self.budget);
            }
            let late_limit = self.late_limit;
            Some(
                async move {
                    let (late, gave_up) = collect_until(pending, Instant::now() + late_limit).await;
                    for (name, run) in gave_up {
                        tracing::warn!("{name} took over {late_limit:?}; giving up");
                        run.abort();
                    }
                    let mut all = found;
                    all.extend(late);
                    finish(all, &query, limit, ai, &*frecency)
                }
                .boxed(),
            )
        };
        Outcome { items, complete }
    }
}

type Run = (&'static str, JoinHandle<Vec<ResultItem>>);

/// Waits for each run until `deadline`: returns the results that came in and
/// the runs still going.
async fn collect_until(runs: Vec<Run>, deadline: Instant) -> (Vec<ResultItem>, Vec<Run>) {
    let mut found = Vec::new();
    let mut pending = Vec::new();
    for (name, mut run) in runs {
        match tokio::time::timeout_at(deadline, &mut run).await {
            Ok(Ok(items)) => found.extend(items),
            Ok(Err(e)) => tracing::warn!("{name} search task failed: {e}"),
            Err(_) => pending.push((name, run)),
        }
    }
    (found, pending)
}

/// Ranks, trims to `limit`, and adds the "Ask" row.
fn finish(
    mut items: Vec<ResultItem>,
    query: &str,
    limit: usize,
    ai: AskRow,
    frecency: &(dyn Fn(&str) -> f64 + Send + Sync),
) -> Vec<ResultItem> {
    rank::rank(&mut items, query, frecency);
    items.truncate(limit);
    items.push(ask_ai_item(query, ai));
    items
}

fn ask_ai_item(query: &str, ai: AskRow) -> ResultItem {
    let fix = |cmd: BuiltinCommand, title: String, subtitle: &str| ResultItem {
        id: cmd.id().into(),
        kind: ItemKind::Command,
        title,
        subtitle: subtitle.into(),
        icon: None,
        score: 0.0,
        actions: vec![Action::Run],
    };
    let name = match ai {
        AskRow::Ask(ProviderId::Claude) => "Claude",
        AskRow::Ask(ProviderId::Chatgpt) => "ChatGPT",
        AskRow::InstallClaude => {
            return fix(
                BuiltinCommand::InstallClaude,
                format!("Install Claude Code to ask: {query}"),
                "Opens the install page",
            )
        }
        AskRow::SignInToClaude => {
            return fix(
                BuiltinCommand::SignInToClaude,
                format!("Sign in to Claude Code to ask: {query}"),
                "Opens a sign-in window",
            )
        }
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

    fn no_history() -> Frecency {
        Arc::new(|_| 0.0)
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
        let outcome = a
            .search("   ", 8, AskRow::Ask(ProviderId::Claude), no_history())
            .await;
        assert!(outcome.items.is_empty() && outcome.complete.is_none());
    }

    #[tokio::test]
    async fn merges_by_score_and_appends_ask_row() {
        let a = agg(vec![
            Arc::new(Fixed("a", vec![0.2, 0.9], Duration::ZERO)),
            Arc::new(Fixed("b", vec![0.5], Duration::ZERO)),
        ]);
        let items = a
            .search("x", 8, AskRow::Ask(ProviderId::Claude), no_history())
            .await
            .items;
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
        let items = a
            .search("x", 2, AskRow::Ask(ProviderId::Chatgpt), no_history())
            .await
            .items;
        assert_eq!(ids(&items), ["a:2", "a:1", "ask-ai"]);
        assert_eq!(items[2].title, "Ask ChatGPT: x");
    }

    #[tokio::test]
    async fn history_reorders_results() {
        let a = agg(vec![Arc::new(Fixed("a", vec![0.9, 0.8], Duration::ZERO))]);
        let used: Frecency = Arc::new(|id: &str| if id == "a:1" { 20.0 } else { 0.0 });
        let items = a
            .search("x", 8, AskRow::Ask(ProviderId::Claude), used)
            .await
            .items;
        assert_eq!(ids(&items), ["a:1", "a:0", "ask-ai"]);
    }

    #[tokio::test]
    async fn slow_results_follow_instead_of_being_dropped() {
        let a = agg(vec![
            Arc::new(Fixed("fast", vec![0.5], Duration::ZERO)),
            Arc::new(Fixed("slow", vec![1.0], Duration::from_millis(200))),
            Arc::new(Failing),
        ]);
        let outcome = a
            .search("x", 8, AskRow::Ask(ProviderId::Claude), no_history())
            .await;
        assert_eq!(ids(&outcome.items), ["fast:0", "ask-ai"]);
        let complete = outcome.complete.expect("slow provider still running").await;
        assert_eq!(ids(&complete), ["slow:0", "fast:0", "ask-ai"]);
    }

    #[tokio::test]
    async fn no_update_when_everyone_is_on_time() {
        let a = agg(vec![Arc::new(Fixed("fast", vec![0.5], Duration::ZERO))]);
        assert!(a
            .search("x", 8, AskRow::Ask(ProviderId::Claude), no_history())
            .await
            .complete
            .is_none());
    }

    #[tokio::test]
    async fn gives_up_on_a_provider_that_never_finishes() {
        let a = agg(vec![
            Arc::new(Fixed("fast", vec![0.5], Duration::ZERO)),
            Arc::new(Fixed("stuck", vec![1.0], Duration::from_secs(3600))),
        ])
        .with_late_limit(Duration::from_millis(100));
        let outcome = a
            .search("x", 8, AskRow::Ask(ProviderId::Claude), no_history())
            .await;
        let complete = outcome.complete.expect("stuck provider pending").await;
        assert_eq!(ids(&complete), ["fast:0", "ask-ai"]);
    }

    #[tokio::test]
    async fn offers_a_fix_when_claude_cant_answer() {
        let a = agg(vec![]);
        let items = a
            .search("sort my files", 8, AskRow::SignInToClaude, no_history())
            .await
            .items;
        assert_eq!(ids(&items), ["cmd:claude-sign-in"]);
        assert_eq!(
            items[0].title,
            "Sign in to Claude Code to ask: sort my files"
        );
        let items = a
            .search("x", 8, AskRow::InstallClaude, no_history())
            .await
            .items;
        assert_eq!(ids(&items), ["cmd:claude-install"]);
    }
}
