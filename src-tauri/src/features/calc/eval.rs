//! Evaluating a prepared query with fend, within a time limit.

use std::sync::Arc;
use std::time::{Duration, Instant};

use super::rates::RateTable;

/// What fend made of a query.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// fend's answer, as it prints it ("approx. 8.04672 km").
    Value(String),
    /// A currency without a rate: `None` when there are no rates at all.
    NoRate(Option<String>),
    /// Not a calculation after all (unknown words, a syntax error, too slow).
    Nothing,
}

/// Stops fend once the time is up ("9^9^9" would otherwise run for ages).
struct Deadline(Instant);

impl fend_core::Interrupt for Deadline {
    fn should_interrupt(&self) -> bool {
        Instant::now() >= self.0
    }
}

/// Exchange rates for fend, relative to the euro.
struct Rates(Arc<RateTable>);

impl fend_core::ExchangeRateFnV2 for Rates {
    fn relative_to_base_currency(
        &self,
        currency: &str,
        _options: &fend_core::ExchangeRateFnV2Options,
    ) -> Result<f64, Box<dyn std::error::Error + Send + Sync + 'static>> {
        self.0
            .per_euro(currency)
            .ok_or_else(|| format!("no rate for {currency}").into())
    }
}

/// Evaluates `expr`, giving up after `budget`. Without `rates`, currency
/// conversions come back as `NoRate(None)`.
pub fn evaluate(expr: &str, rates: Option<Arc<RateTable>>, budget: Duration) -> Outcome {
    let mut context = fend_core::Context::new();
    // "roll d6" would answer differently on every keystroke.
    context.disable_rng();
    let have_rates = rates.is_some();
    if let Some(rates) = rates {
        context.set_exchange_rate_handler_v2(Rates(rates));
    }
    let deadline = Deadline(Instant::now() + budget);
    match fend_core::evaluate_with_interrupt(expr, &mut context, &deadline) {
        Ok(result) if !result.output_is_empty() && !result.get_main_result().is_empty() => {
            Outcome::Value(result.get_main_result().to_owned())
        }
        Ok(_) => Outcome::Nothing,
        Err(message) if message.contains("exchange rate") => {
            Outcome::NoRate(have_rates.then(|| missing_currency(&message)).flatten())
        }
        Err(_) => Outcome::Nothing,
    }
}

/// "failed to retrieve AED exchange rate: …" → "AED".
fn missing_currency(message: &str) -> Option<String> {
    let rest = message.strip_prefix("failed to retrieve ")?;
    let code = rest.split_whitespace().next()?;
    Some(code.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    const BUDGET: Duration = Duration::from_millis(500);

    fn rates() -> Arc<RateTable> {
        Arc::new(RateTable {
            date: "2026-10-20".into(),
            fetched_at: 0,
            per_euro: BTreeMap::from([("USD".into(), 1.25), ("GBP".into(), 0.8)]),
        })
    }

    fn value(expr: &str) -> String {
        match evaluate(expr, Some(rates()), BUDGET) {
            Outcome::Value(v) => v,
            other => panic!("{expr}: {other:?}"),
        }
    }

    #[test]
    fn math_units_and_bases() {
        assert_eq!(value("2^10"), "1024");
        assert_eq!(value("5 mi to km"), "8.04672 km");
        assert_eq!(value("100 °C to °F"), "212 °F");
        assert_eq!(value("1 MB in kB"), "1000 kB");
        assert_eq!(value("255 to hex"), "ff");
        assert_eq!(value("0xff to decimal"), "255");
        assert_eq!(value("(200) * (1 + 10 / 100)"), "220");
        assert_eq!(value("80 * (15 / 100)"), "12");
    }

    #[test]
    fn currencies_use_the_rates() {
        assert_eq!(value("100 EUR to USD"), "125 USD");
        assert_eq!(value("125 USD to GBP"), "80 GBP");
    }

    #[test]
    fn missing_rates_are_reported() {
        assert_eq!(
            evaluate("1 USD to EUR", None, BUDGET),
            Outcome::NoRate(None)
        );
        assert_eq!(
            evaluate("1 AED to EUR", Some(rates()), BUDGET),
            Outcome::NoRate(Some("AED".into()))
        );
    }

    #[test]
    fn nonsense_and_slow_input_give_nothing() {
        assert_eq!(evaluate("chrome", None, BUDGET), Outcome::Nothing);
        assert_eq!(evaluate("1 +", None, BUDGET), Outcome::Nothing);
        assert_eq!(evaluate("km", None, BUDGET), Outcome::Value("1 km".into()));
        let start = Instant::now();
        assert_eq!(
            evaluate("9^9^9", None, Duration::from_millis(30)),
            Outcome::Nothing
        );
        assert!(start.elapsed() < Duration::from_millis(500));
    }
}
