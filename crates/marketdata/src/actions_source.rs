//! Real upcoming corporate actions from Alpha Vantage. Optional, and off by default.
//!
//! Alpha Vantage's documentation (checked in Part 19) lists two corporate-action endpoints
//! and neither is marked premium:
//!
//! - `DIVIDENDS` returns "historical and future (declared) dividend distributions", so it
//!   can supply real *upcoming* cash dividends. This module loads those.
//! - `SPLITS` returns "historical split events" only. It has no upcoming splits, so nothing
//!   in this module fetches it: a split cannot be scheduled from it.
//!
//! The free key allows 25 requests a day, so the loader makes one `DIVIDENDS` request per
//! symbol on the discrete model, spaced by the client's interval, stops at the first rate
//! limit, and archives each raw body under `corporate-actions/alpha-vantage/<SYMBOL>/` in
//! the storage folder. That folder is separate from `market-data/`, whose archive reader
//! treats every file in it as a daily series.

use chrono::{DateTime, NaiveDate, Utc};
use domain::{CashDividend, CorporateAction, Symbol};
use serde::{Deserialize, Serialize};
use storage::{StorageRoot, write_json_new};

use crate::corporate_actions::ScheduledAction;
use crate::error::MarketDataError;
use crate::source::{AlphaVantageSource, error_body};

/// The archive folder, under the storage root.
const FOLDER: [&str; 2] = ["corporate-actions", "alpha-vantage"];

/// One archived download. The API key is never stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ActionsArchiveRecord {
    /// The symbol asked about.
    pub symbol: Symbol,
    /// When the body was fetched.
    pub fetched_at: DateTime<Utc>,
    /// The Alpha Vantage function, `DIVIDENDS`.
    pub function: String,
    /// The raw response body.
    pub body: String,
}

/// Parses a `DIVIDENDS` body into dividends, in the order given (newest first at Alpha
/// Vantage). Rows are `{"ex_dividend_date": "2026-11-19", "amount": "0.98", ...}`, with
/// numbers as strings and other dates possibly the string `"None"`.
///
/// # Errors
///
/// The Alpha Vantage error variants of [`MarketDataError`] for an error body,
/// [`MarketDataError::BadBody`] when `data` is missing and [`MarketDataError::InvalidData`]
/// for a row whose ex-date or amount cannot be read.
pub fn parse_dividends_body(body: &str) -> Result<Vec<CashDividend>, MarketDataError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| MarketDataError::BadBody(e.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| MarketDataError::BadBody("body is not a JSON object".into()))?;
    if let Some(error) = error_body(object) {
        return Err(error);
    }
    let rows = object
        .get("data")
        .and_then(serde_json::Value::as_array)
        .ok_or_else(|| MarketDataError::BadBody("missing \"data\"".into()))?;
    let mut dividends = Vec::with_capacity(rows.len());
    for row in rows {
        let field = |name: &str| row.get(name).and_then(serde_json::Value::as_str);
        let date_text = field("ex_dividend_date")
            .ok_or_else(|| MarketDataError::InvalidData("row without ex_dividend_date".into()))?;
        let ex_date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|e| {
            MarketDataError::InvalidData(format!("ex_dividend_date {date_text:?}: {e}"))
        })?;
        let amount_text = field("amount")
            .ok_or_else(|| MarketDataError::InvalidData(format!("no amount on {ex_date}")))?;
        let amount: f64 = amount_text.parse().map_err(|_| {
            MarketDataError::InvalidData(format!("amount {amount_text:?} on {ex_date}"))
        })?;
        if !amount.is_finite() || amount <= 0.0 {
            return Err(MarketDataError::InvalidData(format!(
                "amount {amount_text:?} on {ex_date} is not positive"
            )));
        }
        dividends.push(CashDividend { ex_date, amount });
    }
    Ok(dividends)
}

/// The dividends going ex after `start`, as scheduled actions with ids of the form
/// `av-dividend-SPY-2026-11-19`.
pub fn upcoming_dividend_actions(
    symbol: &Symbol,
    dividends: &[CashDividend],
    start: NaiveDate,
) -> Vec<ScheduledAction> {
    dividends
        .iter()
        .filter(|d| d.ex_date > start)
        .filter_map(|d| {
            let action =
                CorporateAction::cash_dividend(symbol.clone(), d.amount, d.ex_date).ok()?;
            Some(ScheduledAction {
                id: format!("av-dividend-{symbol}-{}", d.ex_date),
                action,
            })
        })
        .collect()
}

/// Fetches, archives and returns the upcoming real dividends of `symbols`.
///
/// One request per symbol. A failure for one symbol is logged and the rest carry on,
/// except a rate limit, which stops the loop: the remaining requests would be refused too
/// and each counts against the day's 25. The result may be empty. Nothing here can stop the
/// app starting.
pub async fn load_upcoming_dividends(
    source: &AlphaVantageSource,
    storage: &StorageRoot,
    symbols: &[Symbol],
    start: NaiveDate,
    now: DateTime<Utc>,
) -> Vec<ScheduledAction> {
    let mut found = Vec::new();
    for symbol in symbols {
        let fetched = match source.dividends_body(symbol).await {
            Ok(body) => parse_dividends_body(&body).map(|dividends| (body, dividends)),
            Err(error) => Err(error),
        };
        let (body, dividends) = match fetched {
            Ok(result) => result,
            Err(MarketDataError::RateLimited(message)) => {
                tracing::warn!(%symbol, %message, "Alpha Vantage rate limit: skipping the remaining dividend lookups");
                break;
            }
            Err(error) => {
                tracing::warn!(%symbol, %error, "cannot load dividends from Alpha Vantage");
                continue;
            }
        };
        let record = ActionsArchiveRecord {
            symbol: symbol.clone(),
            fetched_at: now,
            function: "DIVIDENDS".to_owned(),
            body,
        };
        let saved = storage
            .subdir(&[FOLDER[0], FOLDER[1], symbol.as_str()])
            .and_then(|dir| write_json_new(&dir, "dividends", now, &record));
        if let Err(error) = saved {
            tracing::warn!(%symbol, %error, "cannot archive the dividends download");
        }
        found.extend(upcoming_dividend_actions(symbol, &dividends, start));
    }
    found
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use chrono::TimeZone;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    /// The shape Alpha Vantage returned for `function=DIVIDENDS&symbol=MSFT`, cut to three rows.
    const BODY: &str = r#"{
        "symbol": "MSFT",
        "data": [
            {"ex_dividend_date": "2026-11-19", "declaration_date": "2026-09-14", "record_date": "2026-11-19", "payment_date": "2026-12-10", "amount": "0.98"},
            {"ex_dividend_date": "2026-08-20", "declaration_date": "2026-06-10", "record_date": "2026-08-20", "payment_date": "2026-09-10", "amount": "0.91"},
            {"ex_dividend_date": "2003-02-19", "declaration_date": "None", "record_date": "None", "payment_date": "None", "amount": "0.08"}
        ]
    }"#;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn parses_the_documented_shape() {
        let dividends = parse_dividends_body(BODY).unwrap();
        assert_eq!(dividends.len(), 3);
        assert_eq!(dividends[0].ex_date, date(2026, 11, 19));
        assert!((dividends[0].amount - 0.98).abs() < 1e-12);
    }

    #[test]
    fn error_bodies_map_to_the_usual_errors() {
        let burst = r#"{"Information": "Please consider spreading out your free API requests more sparingly (1 request per second). ... (25 requests per day)"}"#;
        assert!(matches!(
            parse_dividends_body(burst),
            Err(MarketDataError::RateLimited(_))
        ));
        assert!(matches!(
            parse_dividends_body(r#"{"Error Message": "Invalid API call."}"#),
            Err(MarketDataError::InvalidSymbol(_))
        ));
        assert!(matches!(
            parse_dividends_body(r#"{"symbol": "X"}"#),
            Err(MarketDataError::BadBody(_))
        ));
        assert!(matches!(
            parse_dividends_body("not json"),
            Err(MarketDataError::BadBody(_))
        ));
        let bad_amount = r#"{"data": [{"ex_dividend_date": "2026-11-19", "amount": "abc"}]}"#;
        assert!(matches!(
            parse_dividends_body(bad_amount),
            Err(MarketDataError::InvalidData(_))
        ));
        let zero = r#"{"data": [{"ex_dividend_date": "2026-11-19", "amount": "0"}]}"#;
        assert!(parse_dividends_body(zero).is_err());
    }

    #[test]
    fn only_dividends_after_the_start_become_actions() {
        let dividends = parse_dividends_body(BODY).unwrap();
        let actions = upcoming_dividend_actions(&sym("MSFT"), &dividends, date(2026, 9, 30));
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].id, "av-dividend-MSFT-2026-11-19");
        assert_eq!(actions[0].action.date(), date(2026, 11, 19));
    }

    #[tokio::test]
    async fn the_loader_fetches_archives_and_returns_upcoming_dividends() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("function", "DIVIDENDS"))
            .and(query_param("symbol", "MSFT"))
            .and(query_param("apikey", "secret-key"))
            .respond_with(ResponseTemplate::new(200).set_body_string(BODY))
            .expect(1)
            .mount(&server)
            .await;
        let source = AlphaVantageSource::new("secret-key", server.uri(), Duration::ZERO).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let storage = StorageRoot::at(dir.path());
        let now = Utc.with_ymd_and_hms(2026, 9, 30, 14, 0, 0).unwrap();
        let found =
            load_upcoming_dividends(&source, &storage, &[sym("MSFT")], date(2026, 9, 30), now)
                .await;
        assert_eq!(found.len(), 1);
        let folder = dir.path().join("corporate-actions/alpha-vantage/MSFT");
        let files: Vec<_> = std::fs::read_dir(&folder).unwrap().collect();
        assert_eq!(files.len(), 1);
        let text = std::fs::read_to_string(files[0].as_ref().unwrap().path()).unwrap();
        assert!(
            !text.contains("secret-key"),
            "the key must never be archived"
        );
        assert!(text.contains("DIVIDENDS"));
    }

    #[tokio::test]
    async fn a_rate_limit_stops_the_loop_and_a_bad_symbol_does_not() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(query_param("symbol", "AAA"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(r#"{"Error Message": "Invalid API call."}"#),
            )
            .expect(1)
            .mount(&server)
            .await;
        Mock::given(method("GET"))
            .and(query_param("symbol", "BBB"))
            .respond_with(ResponseTemplate::new(200).set_body_string(
                r#"{"Information": "Thank you! standard API rate limit is 25 requests per day."}"#,
            ))
            .expect(1)
            .mount(&server)
            .await;
        // CCC is never asked for: the limit on BBB ended the loop.
        Mock::given(method("GET"))
            .and(query_param("symbol", "CCC"))
            .respond_with(ResponseTemplate::new(200).set_body_string(BODY))
            .expect(0)
            .mount(&server)
            .await;
        let source = AlphaVantageSource::new("k", server.uri(), Duration::ZERO).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let storage = StorageRoot::at(dir.path());
        let now = Utc.with_ymd_and_hms(2026, 9, 30, 14, 0, 0).unwrap();
        let symbols = [sym("AAA"), sym("BBB"), sym("CCC")];
        let found =
            load_upcoming_dividends(&source, &storage, &symbols, date(2026, 9, 30), now).await;
        assert!(found.is_empty());
    }
}
