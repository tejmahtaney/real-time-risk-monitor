//! The data-source chain: local archive, then the API, then the committed fallback files.

use std::collections::BTreeMap;
use std::path::PathBuf;

use chrono::Duration;
use domain::{Clock, Symbol};
use serde::{Deserialize, Serialize};

use crate::archive::Archive;
use crate::config::DataMode;
use crate::error::MarketDataError;
use crate::fallback::read_fallback;
use crate::source::{DailyBar, HistorySource};

/// Where one symbol's history came from. Files are named without their folder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DataSource {
    /// An archived download within the cache age.
    Archive {
        /// The archive file.
        file: String,
    },
    /// A download made now, archived as a new file (`None` if saving it failed).
    Api {
        /// The archive file the download was saved to.
        archive_file: Option<String>,
    },
    /// An archived download older than the cache age, used because the API was not
    /// available.
    StaleArchive {
        /// The archive file.
        file: String,
    },
    /// The committed fallback CSV.
    Fallback {
        /// The fallback file.
        file: String,
    },
    /// Prices replayed by the backtester: an uploaded file or the merged local archive.
    Replay {
        /// Where the replayed prices came from, such as `upload` or `archive`.
        origin: String,
    },
}

/// One symbol's daily bars and where they came from.
#[derive(Debug, Clone, PartialEq)]
pub struct LoadedHistory {
    /// The bars, oldest first.
    pub bars: Vec<DailyBar>,
    /// Where they came from.
    pub source: DataSource,
}

/// Loads price history through the archive, API and fallback chain.
///
/// The API source is optional so the app runs without a key. The storage archive is
/// checked first so the free tier's 25 requests a day are not spent on every start.
pub struct HistoryLoader<S> {
    archive: Archive,
    fallback_dir: PathBuf,
    max_age: Duration,
    source: Option<S>,
    clock: Box<dyn Clock>,
}

fn file_name(path: &std::path::Path) -> String {
    path.file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned())
}

impl<S: HistorySource> HistoryLoader<S> {
    /// Builds a loader. `source` is `None` when no API key is available.
    pub fn new(
        archive: Archive,
        fallback_dir: impl Into<PathBuf>,
        cache_max_age_hours: u64,
        source: Option<S>,
        clock: Box<dyn Clock>,
    ) -> Self {
        Self {
            archive,
            fallback_dir: fallback_dir.into(),
            max_age: i64::try_from(cache_max_age_hours)
                .ok()
                .and_then(Duration::try_hours)
                .unwrap_or(Duration::MAX),
            source,
            clock,
        }
    }

    /// Loads history for each symbol.
    ///
    /// Per symbol the order is: the newest archive file within the cache age; the API
    /// (`auto` mode only), archiving the result as a new file; the newest archive file of
    /// any age; the fallback CSV. `offline` mode skips the API step. Every choice is
    /// logged with its source and file. After the API reports a rate limit it is not
    /// tried again for the remaining symbols of this call.
    ///
    /// # Errors
    ///
    /// [`MarketDataError::NoHistory`] for the first symbol that no source can supply, and
    /// [`MarketDataError::Fallback`] for a fallback file that exists but is unusable.
    pub async fn load_history(
        &self,
        symbols: &[Symbol],
        mode: DataMode,
    ) -> Result<BTreeMap<Symbol, LoadedHistory>, MarketDataError> {
        let mut api_blocked = false;
        let mut out = BTreeMap::new();
        for symbol in symbols {
            let loaded = self.load_one(symbol, mode, &mut api_blocked).await?;
            out.insert(symbol.clone(), loaded);
        }
        Ok(out)
    }

    async fn load_one(
        &self,
        symbol: &Symbol,
        mode: DataMode,
        api_blocked: &mut bool,
    ) -> Result<LoadedHistory, MarketDataError> {
        let now = self.clock.now();

        if let Some(hit) = self.archived(symbol, Some(self.max_age)) {
            let file = file_name(&hit.path);
            tracing::info!(%symbol, %file, "using cached archive");
            return Ok(LoadedHistory {
                bars: hit.bars,
                source: DataSource::Archive { file },
            });
        }

        if mode == DataMode::Auto {
            match (&self.source, *api_blocked) {
                (None, _) => tracing::info!(%symbol, "no API source configured, skipping the API"),
                (Some(_), true) => {
                    tracing::info!(%symbol, "API rate limited earlier in this load, skipping it");
                }
                (Some(source), false) => match source.download(symbol).await {
                    Ok(download) => {
                        let archive_file = match self.archive.save(symbol, &download, now) {
                            Ok(path) => Some(file_name(&path)),
                            Err(error) => {
                                tracing::warn!(%symbol, %error, "could not archive the download");
                                None
                            }
                        };
                        tracing::info!(%symbol, ?archive_file, bars = download.bars.len(), "fetched from Alpha Vantage");
                        return Ok(LoadedHistory {
                            bars: download.bars,
                            source: DataSource::Api { archive_file },
                        });
                    }
                    Err(error) => {
                        tracing::warn!(%symbol, %error, "Alpha Vantage fetch failed");
                        if matches!(error, MarketDataError::RateLimited(_)) {
                            *api_blocked = true;
                        }
                    }
                },
            }
        }

        if let Some(hit) = self.archived(symbol, None) {
            let file = file_name(&hit.path);
            tracing::warn!(%symbol, %file, fetched_at = %hit.record.fetched_at, "using a stale archive file");
            return Ok(LoadedHistory {
                bars: hit.bars,
                source: DataSource::StaleArchive { file },
            });
        }

        if let Some(bars) = read_fallback(&self.fallback_dir, symbol)? {
            let file = format!("{symbol}.csv");
            tracing::warn!(%symbol, %file, "using the committed fallback file");
            return Ok(LoadedHistory {
                bars,
                source: DataSource::Fallback { file },
            });
        }

        Err(MarketDataError::NoHistory(symbol.clone()))
    }

    /// The newest archive file, treating a storage failure as "nothing there" so the
    /// chain can carry on to the fallback files.
    fn archived(&self, symbol: &Symbol, max_age: Option<Duration>) -> Option<crate::ArchiveHit> {
        self.archive
            .newest(symbol, self.clock.now(), max_age)
            .unwrap_or_else(|error| {
                tracing::warn!(%symbol, %error, "cannot read the archive");
                None
            })
    }
}

#[cfg(test)]
mod tests {
    use std::collections::VecDeque;
    use std::sync::Mutex;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::Duration as StdDuration;

    use approx::assert_relative_eq;
    use chrono::{DateTime, TimeZone, Utc};
    use domain::FixedClock;
    use storage::StorageRoot;
    use wiremock::matchers::method;
    use wiremock::{Mock, MockServer, ResponseTemplate};

    use super::*;
    use crate::source::{
        AlphaVantageSource, Download, RequestParams, parse_daily_body, sample_body,
    };

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn now() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 9, 26, 12, 0, 0).unwrap()
    }

    fn download(symbol: &str, close: f64) -> Download {
        let raw_body = sample_body(symbol, &[("2026-09-25", close)]);
        Download {
            bars: parse_daily_body(&raw_body).unwrap(),
            raw_body,
            params: RequestParams {
                function: "TIME_SERIES_DAILY".into(),
                symbol: symbol.into(),
                outputsize: "compact".into(),
                datatype: "json".into(),
            },
        }
    }

    /// Answers from a queue and counts calls.
    struct Mock1 {
        queue: Mutex<VecDeque<Result<Download, MarketDataError>>>,
        calls: AtomicUsize,
    }

    impl Mock1 {
        fn new(items: Vec<Result<Download, MarketDataError>>) -> Self {
            Self {
                queue: Mutex::new(items.into()),
                calls: AtomicUsize::new(0),
            }
        }
    }

    impl HistorySource for &Mock1 {
        fn download(
            &self,
            _symbol: &Symbol,
        ) -> impl Future<Output = Result<Download, MarketDataError>> + Send {
            self.calls.fetch_add(1, Ordering::SeqCst);
            let next = self
                .queue
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(Err(MarketDataError::HttpStatus(500)));
            std::future::ready(next)
        }
    }

    struct Env {
        dir: tempfile::TempDir,
    }

    impl Env {
        fn new() -> Self {
            Self {
                dir: tempfile::tempdir().unwrap(),
            }
        }

        fn storage(&self) -> StorageRoot {
            StorageRoot::at(self.dir.path().join("storage"))
        }

        fn fallback_dir(&self) -> PathBuf {
            self.dir.path().join("fallback")
        }

        fn write_fallback(&self, symbol: &str, close: f64) {
            crate::write_fallback(
                &self.fallback_dir(),
                &sym(symbol),
                &[DailyBar {
                    date: chrono::NaiveDate::from_ymd_opt(2026, 9, 1).unwrap(),
                    close,
                }],
            )
            .unwrap();
        }

        fn archive_files(&self, symbol: &str) -> usize {
            storage::list(
                &self
                    .dir
                    .path()
                    .join("storage")
                    .join("market-data")
                    .join(symbol),
            )
            .unwrap()
            .len()
        }

        fn loader<'a>(
            &self,
            source: Option<&'a Mock1>,
            at: DateTime<Utc>,
        ) -> HistoryLoader<&'a Mock1> {
            HistoryLoader::new(
                Archive::new(self.storage()),
                self.fallback_dir(),
                24,
                source,
                Box::new(FixedClock::new(at)),
            )
        }

        fn seed_archive(&self, symbol: &str, close: f64, at: DateTime<Utc>) {
            Archive::new(self.storage())
                .save(&sym(symbol), &download(symbol, close), at)
                .unwrap();
        }
    }

    #[tokio::test]
    async fn fresh_archive_is_used_without_a_request() {
        let env = Env::new();
        env.seed_archive("SPY", 1.0, now() - Duration::hours(3));
        let mock = Mock1::new(vec![]);
        let got = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
        let h = &got[&sym("SPY")];
        assert!(matches!(h.source, DataSource::Archive { .. }));
        assert_relative_eq!(h.bars[0].close, 1.0);
    }

    #[tokio::test]
    async fn stale_archive_triggers_a_fetch_and_a_second_file() {
        let env = Env::new();
        env.seed_archive("SPY", 1.0, now() - Duration::hours(25));
        let mock = Mock1::new(vec![Ok(download("SPY", 2.0))]);
        let got = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        let h = &got[&sym("SPY")];
        assert!(matches!(
            h.source,
            DataSource::Api {
                archive_file: Some(_)
            }
        ));
        assert_relative_eq!(h.bars[0].close, 2.0);
        assert_eq!(env.archive_files("SPY"), 2, "two fetches, two files");
        // The new file now counts as the cache.
        let again = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        assert_relative_eq!(again[&sym("SPY")].bars[0].close, 2.0);
    }

    #[tokio::test]
    async fn api_failure_prefers_any_age_archive_over_fallback() {
        let env = Env::new();
        env.seed_archive("SPY", 1.0, now() - Duration::days(30));
        env.write_fallback("SPY", 9.0);
        let mock = Mock1::new(vec![Err(MarketDataError::HttpStatus(500))]);
        let got = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        let h = &got[&sym("SPY")];
        assert!(matches!(h.source, DataSource::StaleArchive { .. }));
        assert_relative_eq!(h.bars[0].close, 1.0);
    }

    #[tokio::test]
    async fn api_failure_without_archive_uses_the_fallback() {
        let env = Env::new();
        env.write_fallback("SPY", 9.0);
        let mock = Mock1::new(vec![Err(MarketDataError::InvalidSymbol("x".into()))]);
        let got = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        let h = &got[&sym("SPY")];
        assert_eq!(
            h.source,
            DataSource::Fallback {
                file: "SPY.csv".into()
            }
        );
        assert_relative_eq!(h.bars[0].close, 9.0);
        assert_eq!(env.archive_files("SPY"), 0);
    }

    #[tokio::test]
    async fn no_source_at_all_is_a_typed_error() {
        let env = Env::new();
        let mock = Mock1::new(vec![Err(MarketDataError::HttpStatus(500))]);
        let err = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap_err();
        assert!(matches!(err, MarketDataError::NoHistory(s) if s == sym("SPY")));
    }

    #[tokio::test]
    async fn offline_never_calls_the_api() {
        let env = Env::new();
        env.seed_archive("SPY", 1.0, now() - Duration::days(30));
        env.write_fallback("AAPL", 5.0);
        let mock = Mock1::new(vec![Ok(download("SPY", 2.0)), Ok(download("AAPL", 2.0))]);
        let got = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY"), sym("AAPL")], DataMode::Offline)
            .await
            .unwrap();
        assert_eq!(mock.calls.load(Ordering::SeqCst), 0);
        assert!(matches!(
            got[&sym("SPY")].source,
            DataSource::StaleArchive { .. }
        ));
        assert!(matches!(
            got[&sym("AAPL")].source,
            DataSource::Fallback { .. }
        ));
    }

    #[tokio::test]
    async fn no_api_source_falls_back_without_a_key() {
        let env = Env::new();
        env.write_fallback("SPY", 9.0);
        let got = env
            .loader(None, now())
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        assert!(matches!(
            got[&sym("SPY")].source,
            DataSource::Fallback { .. }
        ));
    }

    #[tokio::test]
    async fn a_rate_limit_stops_further_api_calls_in_the_same_load() {
        let env = Env::new();
        env.write_fallback("SPY", 1.0);
        env.write_fallback("AAPL", 2.0);
        let mock = Mock1::new(vec![
            Err(MarketDataError::RateLimited("cap".into())),
            Ok(download("AAPL", 3.0)),
        ]);
        let got = env
            .loader(Some(&mock), now())
            .load_history(&[sym("SPY"), sym("AAPL")], DataMode::Auto)
            .await
            .unwrap();
        assert_eq!(mock.calls.load(Ordering::SeqCst), 1);
        assert!(matches!(
            got[&sym("AAPL")].source,
            DataSource::Fallback { .. }
        ));
    }

    #[tokio::test]
    async fn the_api_key_never_reaches_the_archive() {
        const KEY: &str = "SECRET-ARCHIVE-KEY-777";
        let env = Env::new();
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .respond_with(
                ResponseTemplate::new(200)
                    .set_body_string(sample_body("SPY", &[("2026-09-25", 3.5)])),
            )
            .mount(&server)
            .await;
        let source = AlphaVantageSource::new(KEY, server.uri(), StdDuration::ZERO).unwrap();
        let loader = HistoryLoader::new(
            Archive::new(env.storage()),
            env.fallback_dir(),
            24,
            Some(source),
            Box::new(FixedClock::new(now())),
        );
        let got = loader
            .load_history(&[sym("SPY")], DataMode::Auto)
            .await
            .unwrap();
        assert!(matches!(got[&sym("SPY")].source, DataSource::Api { .. }));
        let files = storage::list(&env.storage().path().join("market-data").join("SPY")).unwrap();
        assert_eq!(files.len(), 1);
        for file in files {
            let text = std::fs::read_to_string(file).unwrap();
            assert!(!text.contains(KEY));
            assert!(text.contains("TIME_SERIES_DAILY"));
        }
    }
}
