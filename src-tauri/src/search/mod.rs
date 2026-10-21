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
use serde::Serialize;
use tokio::task::JoinHandle;
use tokio::time::Instant;

use crate::ai::ProviderId;
use commands::BuiltinCommand;
pub use item::{color_icon, glyph, text_icon, thumb_icon, Action, ItemKind, ResultItem};

/// What the last row offers: asking the AI, or fixing what stops it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AskRow {
    Ask(ProviderId),
    InstallClaude,
    SignInToClaude,
    InstallCodex,
    SignInToChatgpt,
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

/// What happens after a result is activated, besides what its provider
/// did itself. `actions::activate` carries out the first four; the UI
/// handles the rest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, specta::Type)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Activation {
    /// Done: hide the launcher.
    Hide,
    /// Copy this text to the clipboard, then hide.
    Copy { text: String },
    /// Hide, then paste into the window you were in: this text, or
    /// whatever is on the clipboard already when `None`.
    Paste { text: Option<String> },
    /// Open a web link (http or https only) in the browser, then hide.
    OpenUrl { url: String },
    /// Replace the search text and search again; the launcher stays open.
    SetQuery { query: String },
    /// Ask the AI this, in a new chat (the default provider when `None`).
    Ask {
        prompt: String,
        provider: Option<ProviderId>,
    },
    /// Stay open and show this in the footer.
    Notice { message: String },
    /// Stay open and run the same search again (the result changed, e.g.
    /// it was pinned or deleted).
    Refresh,
    /// Show the settings menu.
    OpenSettings,
}

#[async_trait]
pub trait SearchProvider: Send + Sync {
    fn name(&self) -> &'static str;
    /// `query` is already trimmed and non-empty.
    async fn query(&self, query: &str, limit: usize) -> Result<Vec<ResultItem>, SearchError>;

    /// True when this query is meant for this provider alone, e.g. a prefix
    /// like `kill ` or `clip`. If any provider claims a query, only the
    /// providers claiming it run.
    fn claims(&self, _query: &str) -> bool {
        false
    }

    /// Whether `id` is one of this provider's results, so its activation
    /// comes here. Use an id prefix only this provider makes ("calc:").
    fn owns(&self, _id: &str) -> bool {
        false
    }

    /// Carries out `action` on one of this provider's results.
    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        let _ = action;
        Err(SearchError::Provider {
            provider: self.name(),
            message: format!("{id} can't be opened"),
        })
    }
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

    /// The provider whose result `id` is, if one claims it.
    pub fn owner(&self, id: &str) -> Option<Arc<dyn SearchProvider>> {
        self.providers.iter().find(|p| p.owns(id)).cloned()
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
        // A prefix meant for one provider ("kill ", "clip") goes to it alone.
        let claimed: Vec<&Arc<dyn SearchProvider>> =
            self.providers.iter().filter(|p| p.claims(&query)).collect();
        let providers = if claimed.is_empty() {
            self.providers.iter().collect()
        } else {
            claimed
        };
        let runs: Vec<(&'static str, JoinHandle<Vec<ResultItem>>)> = providers
            .into_iter()
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

/// The "Ask" row for `query` (or the fix that stops that AI answering),
/// its icon showing which AI it is.
pub fn ask_ai_item(query: &str, ai: AskRow) -> ResultItem {
    let fix = |cmd: BuiltinCommand, title: String, subtitle: &str| ResultItem {
        id: cmd.id().into(),
        kind: ItemKind::Command,
        title,
        subtitle: subtitle.into(),
        icon: None,
        score: 0.0,
        actions: vec![Action::Run],
        accessory: None,
        confirm: None,
    };
    let (provider, icon) = match ai {
        AskRow::Ask(ProviderId::Claude) => (ProviderId::Claude, "sparkle"),
        AskRow::Ask(ProviderId::Chatgpt) => (ProviderId::Chatgpt, "chatgpt"),
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
        AskRow::InstallCodex => {
            return fix(
                BuiltinCommand::InstallCodex,
                format!("Install Codex to ask ChatGPT: {query}"),
                "Opens the install page",
            )
        }
        AskRow::SignInToChatgpt => {
            return fix(
                BuiltinCommand::SignInToChatgpt,
                format!("Sign in to ChatGPT to ask: {query}"),
                "Opens a sign-in window",
            )
        }
    };
    ResultItem {
        id: "ask-ai".into(),
        kind: ItemKind::AskAi,
        title: format!("Ask {}: {query}", provider.name()),
        subtitle: "Tab".into(),
        icon: Some(glyph(icon)),
        score: 0.0,
        actions: vec![Action::AskAi],
        accessory: None,
        confirm: None,
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
                    accessory: None,
                    confirm: None,
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
        assert_eq!(items[2].icon.as_deref(), Some("glyph:chatgpt"));
    }

    #[test]
    fn the_ask_row_shows_which_ai() {
        let claude = ask_ai_item("hi", AskRow::Ask(ProviderId::Claude));
        assert_eq!(claude.icon.as_deref(), Some("glyph:sparkle"));
        assert_eq!(claude.title, "Ask Claude: hi");
        let install = ask_ai_item("hi", AskRow::InstallCodex);
        assert_eq!(install.id, "cmd:codex-install");
        assert_eq!(install.title, "Install Codex to ask ChatGPT: hi");
        let sign_in = ask_ai_item("hi", AskRow::SignInToChatgpt);
        assert_eq!(sign_in.id, "cmd:chatgpt-sign-in");
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

    /// Answers only queries starting with `kill `, and owns `kill:` ids.
    struct Prefixed;

    #[async_trait]
    impl SearchProvider for Prefixed {
        fn name(&self) -> &'static str {
            "prefixed"
        }
        async fn query(&self, q: &str, _: usize) -> Result<Vec<ResultItem>, SearchError> {
            Ok(vec![ResultItem::new(
                format!("kill:{q}"),
                ItemKind::Process,
                q,
            )
            .with_score(0.5)
            .with_actions([Action::Run])])
        }
        fn claims(&self, query: &str) -> bool {
            query.starts_with("kill ")
        }
        fn owns(&self, id: &str) -> bool {
            id.starts_with("kill:")
        }
        async fn activate(&self, id: &str, _: Action) -> Result<Activation, SearchError> {
            Ok(Activation::Notice {
                message: format!("ran {id}"),
            })
        }
    }

    #[tokio::test]
    async fn a_claimed_query_goes_to_that_provider_alone() {
        let a = agg(vec![
            Arc::new(Fixed("a", vec![0.9], Duration::ZERO)),
            Arc::new(Prefixed),
        ]);
        let items = a
            .search(
                "kill chrome",
                8,
                AskRow::Ask(ProviderId::Claude),
                no_history(),
            )
            .await
            .items;
        assert_eq!(ids(&items), ["kill:kill chrome", "ask-ai"]);
        // Unclaimed queries reach everyone.
        let items = a
            .search("chrome", 8, AskRow::Ask(ProviderId::Claude), no_history())
            .await
            .items;
        // (The provider's row titled "chrome" is an exact match, so first.)
        assert_eq!(ids(&items), ["kill:chrome", "a:0", "ask-ai"]);
    }

    #[tokio::test]
    async fn activation_goes_to_the_owner() {
        let a = agg(vec![
            Arc::new(Fixed("a", vec![0.9], Duration::ZERO)),
            Arc::new(Prefixed),
        ]);
        assert!(a.owner("a:0").is_none());
        let owner = a.owner("kill:42").expect("owned");
        assert_eq!(
            owner.activate("kill:42", Action::Run).await.unwrap(),
            Activation::Notice {
                message: "ran kill:42".into()
            }
        );
        // Providers that don't override `activate` refuse politely.
        assert!(Fixed("a", vec![], Duration::ZERO)
            .activate("a:0", Action::Run)
            .await
            .is_err());
    }
}
