//! Calculator and unit / currency conversion right in search (`2^10`,
//! `5 mi to km`, `100 usd in eur`).
//!
//! - `parse` decides what counts as a calculation (`tokens` splits the
//!   query, `rewrite` turns everyday notation into fend's);
//! - `eval` runs fend with a time limit, `format` makes the answer readable;
//! - `rates` keeps exchange rates fresh in the background (`crate::net`
//!   downloads).

mod eval;
mod format;
mod parse;
mod rates;
mod rewrite;
mod tokens;

use std::collections::VecDeque;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};

use super::ProviderContext;
use crate::search::{glyph, Action, Activation, ItemKind, ResultItem, SearchError, SearchProvider};
use crate::settings::SharedSettings;
use crate::state::AppState;
use eval::Outcome;
use rates::ExchangeRates;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, specta::Type)]
pub struct CalcSettings {
    pub enabled: bool,
    /// Convert currencies, with rates downloaded once a day.
    pub currency: bool,
}

impl Default for CalcSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            currency: true,
        }
    }
}

const ID_PREFIX: &str = "calc:";
/// The "rates aren't downloaded yet" row: Enter tries now.
const DOWNLOAD_ID: &str = "calc-rates:download";
/// The "no rate for this currency" row.
const MISSING_ID: &str = "calc-rates:missing";
/// fend's share of the 40 ms search budget.
const EVAL_BUDGET: Duration = Duration::from_millis(25);
/// Answers remembered so Enter copies exactly what was shown.
const REMEMBERED: usize = 16;

/// Shared with `start`, which keeps them fresh.
static RATES: OnceLock<Arc<ExchangeRates>> = OnceLock::new();

/// This feature's search results.
pub fn provider(ctx: &ProviderContext) -> Option<Arc<dyn SearchProvider>> {
    let file = ctx.paths.data_dir.join("exchange-rates.json");
    let rates = RATES
        .get_or_init(|| Arc::new(ExchangeRates::load(file)))
        .clone();
    Some(Arc::new(CalcProvider::new(
        Arc::clone(&ctx.settings),
        rates,
    )))
}

/// Downloads exchange rates in the background (at most once a day).
pub fn start(app: &AppHandle) {
    let (Some(rates), Some(state)) = (RATES.get(), app.try_state::<AppState>()) else {
        return;
    };
    tauri::async_runtime::spawn(rates::keep_fresh(
        Arc::clone(rates),
        Arc::clone(&state.settings),
    ));
}

pub struct CalcProvider {
    settings: SharedSettings,
    rates: Arc<ExchangeRates>,
    /// Recent answers: (id, text to copy).
    recent: Mutex<VecDeque<(String, String)>>,
}

impl CalcProvider {
    fn new(settings: SharedSettings, rates: Arc<ExchangeRates>) -> Self {
        Self {
            settings,
            rates,
            recent: Mutex::new(VecDeque::new()),
        }
    }

    /// The answer to `query` and the text Enter copies, if it's a calculation.
    async fn answer(&self, query: &str) -> Option<(ResultItem, Option<String>)> {
        let calc = match self.settings.read() {
            Ok(s) => s.calculator.clone(),
            Err(poisoned) => poisoned.into_inner().calculator.clone(),
        };
        if !calc.enabled {
            return None;
        }
        let prepared = parse::prepare(query)?;
        if prepared.currency && !calc.currency {
            return None;
        }
        let rates = self.rates.current().filter(|_| calc.currency);
        let expr = prepared.expr.clone();
        let table = rates.clone();
        let outcome =
            tokio::task::spawn_blocking(move || eval::evaluate(&expr, table, EVAL_BUDGET))
                .await
                .unwrap_or(Outcome::Nothing);
        match outcome {
            Outcome::Value(raw) => {
                let answer = format::answer(&raw, &prepared)?;
                let mut subtitle = prepared.shown.clone();
                if let (true, Some(rates)) = (prepared.currency, &rates) {
                    subtitle.push_str(&format!(" · rates of {}", rates.date));
                }
                let icon = if prepared.currency {
                    "currency"
                } else if prepared.conversion {
                    "convert"
                } else {
                    "calculator"
                };
                let item = ResultItem::new(
                    format!("{ID_PREFIX}{}", query.trim()),
                    ItemKind::Answer,
                    answer.title,
                )
                .with_subtitle(subtitle)
                .with_icon(glyph(icon))
                .with_score(1.0)
                .with_actions([Action::Copy, Action::Paste]);
                Some((item, Some(answer.text)))
            }
            Outcome::NoRate(_) if !prepared.currency => None,
            Outcome::NoRate(None) => {
                let item = ResultItem::new(
                    DOWNLOAD_ID,
                    ItemKind::Answer,
                    "Exchange rates aren't downloaded yet",
                )
                .with_subtitle("Press Enter to try now; they download once a day")
                .with_icon(glyph("currency"))
                .with_score(1.0)
                .with_actions([Action::Run]);
                Some((item, None))
            }
            Outcome::NoRate(Some(code)) => {
                let item = ResultItem::new(
                    format!("{MISSING_ID}:{code}"),
                    ItemKind::Answer,
                    format!("No exchange rate for {code}"),
                )
                .with_subtitle("The European Central Bank's daily rates cover about 30 currencies")
                .with_icon(glyph("currency"))
                .with_score(1.0)
                .with_actions([Action::Run]);
                Some((item, None))
            }
            Outcome::Nothing => None,
        }
    }

    fn remember(&self, id: &str, text: String) {
        if let Ok(mut recent) = self.recent.lock() {
            recent.retain(|(seen, _)| seen != id);
            recent.push_front((id.to_owned(), text));
            recent.truncate(REMEMBERED);
        }
    }

    fn remembered(&self, id: &str) -> Option<String> {
        let recent = self.recent.lock().ok()?;
        recent
            .iter()
            .find(|(seen, _)| seen == id)
            .map(|(_, text)| text.clone())
    }
}

#[async_trait]
impl SearchProvider for CalcProvider {
    fn name(&self) -> &'static str {
        "calculator"
    }

    async fn query(&self, query: &str, _limit: usize) -> Result<Vec<ResultItem>, SearchError> {
        let Some((item, text)) = self.answer(query).await else {
            return Ok(Vec::new());
        };
        if let Some(text) = text {
            self.remember(&item.id, text);
        }
        Ok(vec![item])
    }

    fn owns(&self, id: &str) -> bool {
        id.starts_with(ID_PREFIX) || id.starts_with("calc-rates:")
    }

    async fn activate(&self, id: &str, action: Action) -> Result<Activation, SearchError> {
        if id == DOWNLOAD_ID {
            self.rates.request_download();
            return Ok(Activation::Notice {
                message: "Downloading exchange rates…".into(),
            });
        }
        if let Some(code) = id
            .strip_prefix(MISSING_ID)
            .and_then(|r| r.strip_prefix(':'))
        {
            return Ok(Activation::Notice {
                message: format!("There's no daily rate for {code}."),
            });
        }
        let query = id.strip_prefix(ID_PREFIX).unwrap_or(id);
        let text = match self.remembered(id) {
            Some(text) => Some(text),
            // Not seen lately (a restart in between): work it out again.
            None => self.answer(query).await.and_then(|(_, text)| text),
        };
        let Some(text) = text else {
            return Err(SearchError::Provider {
                provider: self.name(),
                message: format!("couldn't work out {query}"),
            });
        };
        Ok(match action {
            Action::Paste => Activation::Paste { text: Some(text) },
            _ => Activation::Copy { text },
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Settings;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::RwLock;

    fn provider_with(rates: Option<rates::RateTable>, settings: Settings) -> CalcProvider {
        let rates = ExchangeRates::with_table(PathBuf::new(), rates.map(Arc::new));
        CalcProvider::new(Arc::new(RwLock::new(settings)), Arc::new(rates))
    }

    fn table() -> rates::RateTable {
        rates::RateTable {
            date: "2026-10-20".into(),
            fetched_at: 0,
            per_euro: BTreeMap::from([("USD".into(), 1.25)]),
        }
    }

    async fn titles(p: &CalcProvider, query: &str) -> Vec<String> {
        p.query(query, 32)
            .await
            .unwrap()
            .into_iter()
            .map(|i| i.title)
            .collect()
    }

    #[tokio::test]
    async fn answers_calculations_only() {
        let p = provider_with(None, Settings::default());
        assert_eq!(titles(&p, "2^10").await, ["1024"]);
        assert_eq!(titles(&p, "5 mi to km").await, ["8.04672 km"]);
        assert_eq!(titles(&p, "72 f to c").await, ["≈ 22.2222 °C"]);
        assert_eq!(titles(&p, "3 cups in ml").await, ["709.7647095 ml"]);
        assert_eq!(titles(&p, "255 in binary").await, ["0b11111111"]);
        assert_eq!(titles(&p, "200 + 10%").await, ["220"]);
        // Searches for apps and files, end to end (parse and fend).
        for query in [
            "chrome",
            "code",
            "notes",
            "tax 2024",
            "e",
            "a",
            "2024-01-01",
            "7-zip",
            "notepad++",
            "c++",
            "blink-182",
            "windows 11",
            "office 365",
            "f1 2024",
            "4k video",
            "60 fps",
            "5 minutes",
            "readme.md",
            "img_1234.jpg",
            "invoice-2024-03.pdf",
            "screenshot 2024-05-01",
            "v1.2.3",
            "localhost:3000",
            "C:\\Users",
            "~/Downloads",
            "t-1000",
            "b-52s",
            "x-men 2",
            "k-9",
            "day-1",
            "1-800-flowers",
            "2 in 1 laptop",
            "how many cm in an inch",
        ] {
            assert!(titles(&p, query).await.is_empty(), "{query}");
        }
    }

    #[tokio::test]
    async fn the_answer_row() {
        let p = provider_with(None, Settings::default());
        let items = p.query("2^16", 32).await.unwrap();
        let item = &items[0];
        assert_eq!(item.kind, ItemKind::Answer);
        assert_eq!(item.title, "65,536");
        assert_eq!(item.subtitle, "2^16");
        assert_eq!(item.actions, [Action::Copy, Action::Paste]);
        assert!(p.owns(&item.id));
        assert_eq!(
            p.activate(&item.id, Action::Copy).await.unwrap(),
            Activation::Copy {
                text: "65536".into()
            }
        );
        assert_eq!(
            p.activate(&item.id, Action::Paste).await.unwrap(),
            Activation::Paste {
                text: Some("65536".into())
            }
        );
        // Works out answers it hasn't shown lately too.
        assert_eq!(
            p.activate("calc:3 x 4", Action::Copy).await.unwrap(),
            Activation::Copy { text: "12".into() }
        );
        assert!(p.activate("calc:chrome", Action::Copy).await.is_err());
    }

    #[tokio::test]
    async fn currencies_need_rates() {
        let p = provider_with(None, Settings::default());
        let items = p.query("100 usd to eur", 32).await.unwrap();
        assert_eq!(items[0].id, DOWNLOAD_ID);
        assert_eq!(
            p.activate(DOWNLOAD_ID, Action::Run).await.unwrap(),
            Activation::Notice {
                message: "Downloading exchange rates…".into()
            }
        );

        let p = provider_with(Some(table()), Settings::default());
        let items = p.query("€100 in $", 32).await.unwrap();
        assert_eq!(items[0].title, "125.00 USD");
        assert_eq!(items[0].subtitle, "100 EUR in USD · rates of 2026-10-20");
        let items = p.query("100 aed to eur", 32).await.unwrap();
        assert_eq!(items[0].title, "No exchange rate for AED");
    }

    #[tokio::test]
    async fn settings_turn_it_off() {
        let mut settings = Settings::default();
        settings.calculator.currency = false;
        let p = provider_with(Some(table()), settings.clone());
        assert!(titles(&p, "100 eur to usd").await.is_empty());
        assert_eq!(titles(&p, "1 + 1").await, ["2"]);
        settings.calculator.enabled = false;
        let p = provider_with(Some(table()), settings);
        assert!(titles(&p, "1 + 1").await.is_empty());
    }
}
