//! Exchange rates for currency conversion: the European Central Bank's
//! daily reference rates (about 30 currencies against the euro, no key
//! needed). Downloaded in the background at most once a day and kept on
//! disk, so conversions work offline. Searching only reads what's in memory.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tokio::sync::Notify;

use crate::settings::{self, SharedSettings};

const HOST: &str = "www.ecb.europa.eu";
const PATH: &str = "/stats/eurofxref/eurofxref-daily.xml";
/// Rates older than this are downloaded again.
const MAX_AGE_SECS: i64 = 24 * 3600;
/// How often the background task looks at the rates' age.
const CHECK_EVERY: Duration = Duration::from_secs(3600);
/// After a failed download (offline, asleep), try again sooner.
const RETRY_AFTER: Duration = Duration::from_secs(10 * 60);
/// A download asked for from search waits at least this long after the last.
const MIN_GAP: Duration = Duration::from_secs(60);

/// One day's rates.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RateTable {
    /// The day the rates are for ("2026-10-20").
    pub date: String,
    /// When they were downloaded (Unix seconds).
    pub fetched_at: i64,
    /// Units of each currency per euro.
    pub per_euro: BTreeMap<String, f64>,
}

impl RateTable {
    /// Units of `code` per euro (the euro is fend's base currency here).
    pub fn per_euro(&self, code: &str) -> Option<f64> {
        if code == "EUR" {
            return Some(1.0);
        }
        self.per_euro.get(code).copied()
    }

    fn is_stale(&self, now: i64) -> bool {
        now - self.fetched_at >= MAX_AGE_SECS || now < self.fetched_at
    }
}

/// Reads the ECB's `eurofxref-daily.xml`:
/// `<Cube time='2026-10-20'><Cube currency='USD' rate='1.0866'/>...`.
pub fn parse_ecb(xml: &str, fetched_at: i64) -> Result<RateTable, String> {
    let mut date = None;
    let mut per_euro = BTreeMap::new();
    for tag in xml.split("<Cube").skip(1) {
        let tag = tag.split('>').next().unwrap_or_default();
        if let Some(time) = attribute(tag, "time") {
            date.get_or_insert_with(|| time.to_owned());
        }
        let (Some(code), Some(rate)) = (attribute(tag, "currency"), attribute(tag, "rate")) else {
            continue;
        };
        let valid_code = code.len() == 3 && code.chars().all(|c| c.is_ascii_uppercase());
        match rate.parse::<f64>() {
            Ok(rate) if valid_code && rate.is_finite() && rate > 0.0 => {
                per_euro.insert(code.to_owned(), rate);
            }
            _ => {}
        }
    }
    let date = date.ok_or("no date in the rates file")?;
    if per_euro.is_empty() {
        return Err("no rates in the rates file".into());
    }
    Ok(RateTable {
        date,
        fetched_at,
        per_euro,
    })
}

/// The value of `name='…'` (or `name="…"`) in a tag's text.
fn attribute<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let mut rest = tag;
    while let Some(at) = rest.find(name) {
        let before = rest[..at].chars().next_back();
        let after = &rest[at + name.len()..];
        rest = after;
        if before.is_some_and(|c| !c.is_whitespace()) {
            continue;
        }
        let after = after.trim_start().strip_prefix('=')?.trim_start();
        let quote = after.chars().next().filter(|c| matches!(c, '\'' | '"'))?;
        let value = &after[1..];
        return value.find(quote).map(|end| &value[..end]);
    }
    None
}

fn load(path: &Path) -> Option<RateTable> {
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(table) => Some(table),
        Err(e) => {
            tracing::warn!("ignoring the saved exchange rates: {e}");
            None
        }
    }
}

fn save(path: &Path, table: &RateTable) -> Result<(), String> {
    let text = serde_json::to_string_pretty(table).map_err(|e| e.to_string())?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())
}

/// The rates in memory, shared by search (reads) and the background
/// download (writes).
pub struct ExchangeRates {
    table: RwLock<Option<Arc<RateTable>>>,
    file: PathBuf,
    wake: Notify,
    last_attempt: Mutex<Option<Instant>>,
}

impl ExchangeRates {
    /// Starts with the rates saved last time, if any.
    pub fn load(file: PathBuf) -> Self {
        let table = load(&file).map(Arc::new);
        Self::with_table(file, table)
    }

    pub fn with_table(file: PathBuf, table: Option<Arc<RateTable>>) -> Self {
        Self {
            table: RwLock::new(table),
            file,
            wake: Notify::new(),
            last_attempt: Mutex::new(None),
        }
    }

    pub fn current(&self) -> Option<Arc<RateTable>> {
        match self.table.read() {
            Ok(t) => t.clone(),
            Err(poisoned) => poisoned.into_inner().clone(),
        }
    }

    /// Asks the background task to download now (if it hasn't just tried).
    pub fn request_download(&self) {
        self.wake.notify_one();
    }

    fn replace(&self, table: RateTable) {
        if let Err(e) = save(&self.file, &table) {
            tracing::warn!("could not save exchange rates: {e}");
        }
        if let Ok(mut t) = self.table.write() {
            *t = Some(Arc::new(table));
        }
    }

    /// Whether a download may start now; records the attempt if so.
    fn may_try(&self) -> bool {
        let Ok(mut last) = self.last_attempt.lock() else {
            return false;
        };
        if last.is_some_and(|at| at.elapsed() < MIN_GAP) {
            return false;
        }
        *last = Some(Instant::now());
        true
    }
}

/// Keeps the rates at most a day old while currency conversion is on.
/// Runs for the life of the app.
pub async fn keep_fresh(rates: Arc<ExchangeRates>, settings: SharedSettings) {
    loop {
        let calc = settings::current(&settings).calculator;
        let stale = rates.current().is_none_or(|t| t.is_stale(unix_now()));
        let mut wait = CHECK_EVERY;
        if calc.enabled && calc.currency && stale && rates.may_try() {
            match tokio::task::spawn_blocking(download).await {
                Ok(Ok(table)) => {
                    tracing::info!(
                        "exchange rates for {} downloaded ({} currencies)",
                        table.date,
                        table.per_euro.len()
                    );
                    rates.replace(table);
                }
                Ok(Err(e)) => {
                    tracing::warn!("could not download exchange rates: {e}");
                    wait = RETRY_AFTER;
                }
                Err(e) => tracing::warn!("exchange rate download crashed: {e}"),
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(wait) => {}
            _ = rates.wake.notified() => {}
        }
    }
}

/// Only the Windows build downloads; elsewhere the saved file is used.
fn download() -> Result<RateTable, String> {
    let body = crate::net::https_get(HOST, PATH)?;
    let text = String::from_utf8(body).map_err(|_| "the rates file isn't text".to_owned())?;
    parse_ecb(&text, unix_now())
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The shape of the real file (trimmed to a few currencies).
    const SAMPLE: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<gesmes:Envelope xmlns:gesmes="http://www.gesmes.org/xml/2002-08-01" xmlns="http://www.ecb.int/vocabulary/2002-08-01/eurofxref">
	<gesmes:subject>Reference rates</gesmes:subject>
	<gesmes:Sender>
		<gesmes:name>European Central Bank</gesmes:name>
	</gesmes:Sender>
	<Cube>
		<Cube time='2026-10-20'>
			<Cube currency='USD' rate='1.0866'/>
			<Cube currency='JPY' rate='169.08'/>
			<Cube currency='GBP' rate='0.85590'/>
			<Cube currency="CHF" rate="0.9870"/>
			<Cube currency='bad' rate='1'/>
			<Cube currency='NOK' rate='oops'/>
		</Cube>
	</Cube>
</gesmes:Envelope>"#;

    #[test]
    fn reads_the_ecb_file() {
        let table = parse_ecb(SAMPLE, 42).unwrap();
        assert_eq!(table.date, "2026-10-20");
        assert_eq!(table.fetched_at, 42);
        assert_eq!(
            table.per_euro.keys().collect::<Vec<_>>(),
            ["CHF", "GBP", "JPY", "USD"]
        );
        assert_eq!(table.per_euro("USD"), Some(1.0866));
        assert_eq!(table.per_euro("EUR"), Some(1.0));
        assert_eq!(table.per_euro("XYZ"), None);
    }

    #[test]
    fn rejects_files_without_rates() {
        assert!(parse_ecb("<html>maintenance</html>", 0).is_err());
        assert!(parse_ecb("<Cube time='2026-10-20'></Cube>", 0).is_err());
        assert!(parse_ecb("<Cube currency='USD' rate='1.1'/>", 0).is_err());
    }

    #[test]
    fn attributes_match_whole_names() {
        assert_eq!(
            attribute(" currency='USD' rate='1.5'/", "rate"),
            Some("1.5")
        );
        assert_eq!(attribute(" xrate='2' rate = \"3\"", "rate"), Some("3"));
        assert_eq!(attribute(" currency='USD'", "rate"), None);
    }

    #[test]
    fn stale_after_a_day_or_when_the_clock_went_back() {
        let table = parse_ecb(SAMPLE, 1_000_000).unwrap();
        assert!(!table.is_stale(1_000_000 + 3600));
        assert!(table.is_stale(1_000_000 + MAX_AGE_SECS));
        assert!(table.is_stale(999_000));
    }

    #[test]
    fn saved_rates_load_again() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("exchange-rates.json");
        let rates = ExchangeRates::load(file.clone());
        assert!(rates.current().is_none());
        rates.replace(parse_ecb(SAMPLE, 7).unwrap());
        let again = ExchangeRates::load(file.clone());
        assert_eq!(again.current().unwrap().per_euro("GBP"), Some(0.8559));
        // A damaged file is ignored rather than failing.
        std::fs::write(&file, "{not json").unwrap();
        assert!(ExchangeRates::load(file).current().is_none());
    }

    #[test]
    fn downloads_are_spaced_out() {
        let rates = ExchangeRates::with_table(PathBuf::new(), None);
        assert!(rates.may_try());
        assert!(!rates.may_try());
    }
}
