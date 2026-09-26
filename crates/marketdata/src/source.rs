//! The price-history source trait and the Alpha Vantage client.

use std::fmt;
use std::future::Future;
use std::time::Duration;

use chrono::NaiveDate;
use domain::Symbol;
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;
use tokio::time::Instant;

use crate::error::MarketDataError;

/// The Alpha Vantage query endpoint.
pub const DEFAULT_BASE_URL: &str = "https://www.alphavantage.co/query";

/// Default gap between requests. The free-tier documentation states no per-minute limit,
/// only 25 requests a day, so this is a cheap safety margin and not a documented rule.
pub const DEFAULT_REQUEST_INTERVAL: Duration = Duration::from_secs(1);

const FUNCTION: &str = "TIME_SERIES_DAILY";
const OUTPUT_SIZE: &str = "compact";
const DATA_TYPE: &str = "json";
const SERIES_KEY: &str = "Time Series (Daily)";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One trading day: its date and raw as-traded close.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct DailyBar {
    /// The trading date.
    pub date: NaiveDate,
    /// The close in dollars. Not adjusted for splits or dividends.
    pub close: f64,
}

/// The parameters of a request, without the API key. Archived next to the raw body.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RequestParams {
    /// The Alpha Vantage function, `TIME_SERIES_DAILY`.
    pub function: String,
    /// The symbol asked for.
    pub symbol: String,
    /// `compact` (latest 100 points) or `full` (premium only).
    pub outputsize: String,
    /// The response format, `json`.
    pub datatype: String,
}

/// A successful download: the parsed bars plus what is needed to archive it.
#[derive(Debug, Clone, PartialEq)]
pub struct Download {
    /// The daily bars, oldest first.
    pub bars: Vec<DailyBar>,
    /// The response body exactly as received.
    pub raw_body: String,
    /// The request parameters, without the API key.
    pub params: RequestParams,
}

/// Something that can supply daily closes for a symbol.
pub trait HistorySource: Send + Sync {
    /// Fetches the daily history with the raw response body and request parameters,
    /// which the archive stores.
    fn download(
        &self,
        symbol: &Symbol,
    ) -> impl Future<Output = Result<Download, MarketDataError>> + Send;

    /// Fetches just the daily closes, oldest first.
    fn daily_closes(
        &self,
        symbol: &Symbol,
    ) -> impl Future<Output = Result<Vec<DailyBar>, MarketDataError>> + Send {
        async move { Ok(self.download(symbol).await?.bars) }
    }
}

/// A client for the Alpha Vantage `TIME_SERIES_DAILY` endpoint.
///
/// The API key is passed in, never read from the environment here. Requests are spaced
/// by a minimum interval. The key is hidden from `Debug` output and from error messages.
pub struct AlphaVantageSource {
    client: reqwest::Client,
    api_key: String,
    base_url: String,
    min_interval: Duration,
    last_request: Mutex<Option<Instant>>,
}

impl fmt::Debug for AlphaVantageSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AlphaVantageSource")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url)
            .field("min_interval", &self.min_interval)
            .finish_non_exhaustive()
    }
}

impl AlphaVantageSource {
    /// Builds a client. `base_url` is [`DEFAULT_BASE_URL`] outside tests and
    /// `min_interval` is [`DEFAULT_REQUEST_INTERVAL`].
    ///
    /// # Errors
    ///
    /// [`MarketDataError::Transport`] when the HTTP client cannot be built.
    pub fn new(
        api_key: impl Into<String>,
        base_url: impl Into<String>,
        min_interval: Duration,
    ) -> Result<Self, MarketDataError> {
        let client = reqwest::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .build()
            .map_err(|e| MarketDataError::Transport(e.without_url().to_string()))?;
        Ok(Self {
            client,
            api_key: api_key.into(),
            base_url: base_url.into(),
            min_interval,
            last_request: Mutex::new(None),
        })
    }

    /// Waits until `min_interval` has passed since the previous request started.
    async fn pace(&self) {
        let mut last = self.last_request.lock().await;
        if let Some(previous) = *last {
            if let Some(wait) = self.min_interval.checked_sub(previous.elapsed()) {
                tokio::time::sleep(wait).await;
            }
        }
        *last = Some(Instant::now());
    }
}

impl HistorySource for AlphaVantageSource {
    async fn download(&self, symbol: &Symbol) -> Result<Download, MarketDataError> {
        self.pace().await;
        let params = RequestParams {
            function: FUNCTION.into(),
            symbol: symbol.as_str().into(),
            outputsize: OUTPUT_SIZE.into(),
            datatype: DATA_TYPE.into(),
        };
        let response = self
            .client
            .get(&self.base_url)
            .query(&[
                ("function", params.function.as_str()),
                ("symbol", params.symbol.as_str()),
                ("outputsize", params.outputsize.as_str()),
                ("datatype", params.datatype.as_str()),
                ("apikey", self.api_key.as_str()),
            ])
            .send()
            .await
            // `without_url` matters: the URL carries the API key.
            .map_err(|e| MarketDataError::Transport(e.without_url().to_string()))?;
        let status = response.status();
        if !status.is_success() {
            return Err(MarketDataError::HttpStatus(status.as_u16()));
        }
        let raw_body = response
            .text()
            .await
            .map_err(|e| MarketDataError::Transport(e.without_url().to_string()))?;
        let bars = parse_daily_body(&raw_body)?;
        Ok(Download {
            bars,
            raw_body,
            params,
        })
    }
}

/// Phrases that mark an `Information` body as being about request limits.
const LIMIT_PHRASES: [&str; 5] = [
    "rate limit",
    "requests per day",
    "request limit",
    "call frequency",
    "per day",
];

/// Parses an Alpha Vantage `TIME_SERIES_DAILY` body into bars, oldest first.
///
/// Alpha Vantage answers HTTP 200 for failures too, with one of these keys in place of
/// data: `Note` becomes [`MarketDataError::RateLimited`], `Error Message` becomes
/// [`MarketDataError::InvalidSymbol`], and `Information` becomes
/// [`MarketDataError::RateLimited`] when it talks about request limits (how the free tier
/// reports its daily cap) and [`MarketDataError::Information`] otherwise. Closes are read
/// from `4. close`, or `close` as the documentation names it.
///
/// # Errors
///
/// The errors above, [`MarketDataError::BadBody`] for invalid JSON or a missing or empty
/// series, and [`MarketDataError::InvalidData`] for a bad date or close.
pub fn parse_daily_body(body: &str) -> Result<Vec<DailyBar>, MarketDataError> {
    let value: serde_json::Value =
        serde_json::from_str(body).map_err(|e| MarketDataError::BadBody(e.to_string()))?;
    let object = value
        .as_object()
        .ok_or_else(|| MarketDataError::BadBody("body is not a JSON object".into()))?;
    let text = |key: &str| {
        object
            .get(key)
            .map(|v| v.as_str().unwrap_or_default().to_owned())
    };
    if let Some(message) = text("Error Message") {
        return Err(MarketDataError::InvalidSymbol(message));
    }
    if let Some(message) = text("Note") {
        return Err(MarketDataError::RateLimited(message));
    }
    if let Some(message) = text("Information") {
        let lower = message.to_ascii_lowercase();
        return Err(if LIMIT_PHRASES.iter().any(|p| lower.contains(p)) {
            MarketDataError::RateLimited(message)
        } else {
            MarketDataError::Information(message)
        });
    }
    let series = object
        .get(SERIES_KEY)
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| MarketDataError::BadBody(format!("missing \"{SERIES_KEY}\"")))?;
    let mut bars = Vec::with_capacity(series.len());
    for (day, fields) in series {
        let date = NaiveDate::parse_from_str(day, "%Y-%m-%d")
            .map_err(|e| MarketDataError::InvalidData(format!("date {day:?}: {e}")))?;
        let close_text = fields
            .get("4. close")
            .or_else(|| fields.get("close"))
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| MarketDataError::InvalidData(format!("no close on {day}")))?;
        let close: f64 = close_text
            .parse()
            .map_err(|_| MarketDataError::InvalidData(format!("close {close_text:?} on {day}")))?;
        if !close.is_finite() || close <= 0.0 {
            return Err(MarketDataError::InvalidData(format!(
                "close {close_text:?} on {day} is not a positive number"
            )));
        }
        bars.push(DailyBar { date, close });
    }
    if bars.is_empty() {
        return Err(MarketDataError::BadBody("the daily series is empty".into()));
    }
    bars.sort_by_key(|b| b.date);
    Ok(bars)
}

/// Builds an Alpha Vantage style body for tests.
#[cfg(test)]
pub(crate) fn sample_body(symbol: &str, bars: &[(&str, f64)]) -> String {
    let series: serde_json::Map<String, serde_json::Value> = bars
        .iter()
        .map(|(date, close)| {
            (
                (*date).to_owned(),
                serde_json::json!({
                    "1. open": format!("{close}"),
                    "2. high": format!("{close}"),
                    "3. low": format!("{close}"),
                    "4. close": format!("{close}"),
                    "5. volume": "1000",
                }),
            )
        })
        .collect();
    serde_json::json!({
        "Meta Data": {"1. Information": "Daily Prices", "2. Symbol": symbol},
        "Time Series (Daily)": series,
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use std::time::Instant as StdInstant;

    use approx::assert_relative_eq;
    use wiremock::matchers::{method, query_param};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;

    const KEY: &str = "SECRET-TEST-KEY-123";

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    fn source(server: &MockServer, interval: Duration) -> AlphaVantageSource {
        AlphaVantageSource::new(KEY, server.uri(), interval).unwrap()
    }

    async fn serve(server: &MockServer, template: ResponseTemplate) {
        Mock::given(method("GET"))
            .respond_with(template)
            .mount(server)
            .await;
    }

    #[tokio::test]
    async fn success_returns_sorted_bars_and_sends_the_expected_query() {
        let server = MockServer::start().await;
        let body = sample_body("SPY", &[("2026-09-25", 600.5), ("2026-09-24", 598.25)]);
        Mock::given(method("GET"))
            .and(query_param("function", "TIME_SERIES_DAILY"))
            .and(query_param("symbol", "SPY"))
            .and(query_param("outputsize", "compact"))
            .and(query_param("apikey", KEY))
            .respond_with(ResponseTemplate::new(200).set_body_string(body.clone()))
            .expect(1)
            .mount(&server)
            .await;
        let dl = source(&server, Duration::ZERO)
            .download(&sym("SPY"))
            .await
            .unwrap();
        assert_eq!(
            dl.bars,
            [
                DailyBar {
                    date: date("2026-09-24"),
                    close: 598.25
                },
                DailyBar {
                    date: date("2026-09-25"),
                    close: 600.5
                },
            ]
        );
        assert_eq!(dl.raw_body, body);
        assert_eq!(dl.params.symbol, "SPY");
        assert!(!format!("{:?}", dl.params).contains(KEY));
    }

    #[tokio::test]
    async fn daily_closes_returns_only_the_bars() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_string(sample_body("SPY", &[("2026-09-25", 1.5)])),
        )
        .await;
        let bars = source(&server, Duration::ZERO)
            .daily_closes(&sym("SPY"))
            .await
            .unwrap();
        assert_eq!(bars.len(), 1);
    }

    async fn error_for(body: &str) -> MarketDataError {
        let server = MockServer::start().await;
        serve(&server, ResponseTemplate::new(200).set_body_string(body)).await;
        source(&server, Duration::ZERO)
            .download(&sym("SPY"))
            .await
            .unwrap_err()
    }

    #[tokio::test]
    async fn note_is_rate_limited() {
        let err = error_for(
            r#"{"Note": "Thank you for using Alpha Vantage! Call frequency is 5 per minute."}"#,
        )
        .await;
        assert!(matches!(err, MarketDataError::RateLimited(_)), "{err:?}");
    }

    #[tokio::test]
    async fn information_about_limits_is_rate_limited() {
        let err = error_for(
            r#"{"Information": "We have detected your API key and our standard API rate limit is 25 requests per day."}"#,
        )
        .await;
        assert!(matches!(err, MarketDataError::RateLimited(_)), "{err:?}");
    }

    #[tokio::test]
    async fn other_information_is_its_own_error() {
        let err = error_for(r#"{"Information": "This is a premium endpoint."}"#).await;
        assert!(matches!(err, MarketDataError::Information(_)), "{err:?}");
    }

    #[tokio::test]
    async fn error_message_is_invalid_symbol() {
        let err = error_for(r#"{"Error Message": "Invalid API call."}"#).await;
        assert!(matches!(err, MarketDataError::InvalidSymbol(_)), "{err:?}");
    }

    #[tokio::test]
    async fn bad_bodies_are_typed_errors() {
        for body in [
            "not json",
            "[1, 2]",
            "{}",
            r#"{"Time Series (Daily)": {}}"#,
            r#"{"Time Series (Daily)": {"not-a-date": {"4. close": "1"}}}"#,
            r#"{"Time Series (Daily)": {"2026-09-25": {"4. close": "abc"}}}"#,
            r#"{"Time Series (Daily)": {"2026-09-25": {"4. close": "-3"}}}"#,
            r#"{"Time Series (Daily)": {"2026-09-25": {"4. close": "0"}}}"#,
            r#"{"Time Series (Daily)": {"2026-09-25": {"4. close": "NaN"}}}"#,
            r#"{"Time Series (Daily)": {"2026-09-25": {"volume": "1"}}}"#,
        ] {
            let err = error_for(body).await;
            assert!(
                matches!(
                    err,
                    MarketDataError::BadBody(_) | MarketDataError::InvalidData(_)
                ),
                "{body}: {err:?}"
            );
        }
    }

    #[tokio::test]
    async fn plain_close_key_is_accepted() {
        let bars =
            parse_daily_body(r#"{"Time Series (Daily)": {"2026-09-25": {"close": "12.5"}}}"#)
                .unwrap();
        assert_relative_eq!(bars[0].close, 12.5);
    }

    #[tokio::test]
    async fn http_500_is_a_status_error() {
        let server = MockServer::start().await;
        serve(&server, ResponseTemplate::new(500).set_body_string("oops")).await;
        let err = source(&server, Duration::ZERO)
            .download(&sym("SPY"))
            .await
            .unwrap_err();
        assert!(matches!(err, MarketDataError::HttpStatus(500)), "{err:?}");
    }

    #[tokio::test]
    async fn transport_errors_do_not_leak_the_key() {
        // Nothing listens on port 1.
        let source =
            AlphaVantageSource::new(KEY, "http://127.0.0.1:1/query", Duration::ZERO).unwrap();
        let err = source.download(&sym("SPY")).await.unwrap_err();
        assert!(matches!(err, MarketDataError::Transport(_)), "{err:?}");
        assert!(!err.to_string().contains(KEY));
        assert!(!format!("{err:?}").contains(KEY));
    }

    #[test]
    fn debug_hides_the_key() {
        let source = AlphaVantageSource::new(KEY, DEFAULT_BASE_URL, Duration::ZERO).unwrap();
        let shown = format!("{source:?}");
        assert!(!shown.contains(KEY));
        assert!(shown.contains("redacted"));
    }

    #[tokio::test]
    async fn requests_are_spaced_by_the_minimum_interval() {
        let server = MockServer::start().await;
        serve(
            &server,
            ResponseTemplate::new(200).set_body_string(sample_body("SPY", &[("2026-09-25", 1.5)])),
        )
        .await;
        let source = source(&server, Duration::from_millis(150));
        let start = StdInstant::now();
        source.download(&sym("SPY")).await.unwrap();
        source.download(&sym("SPY")).await.unwrap();
        assert!(start.elapsed() >= Duration::from_millis(150));
    }
}
