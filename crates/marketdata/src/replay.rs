//! Historical prices for the backtester: parsing and validating uploaded files, the aligned
//! price table, rolling volatilities and rolling `VaR` windows, and the replay feed.
//!
//! A [`RawPrices`] holds whatever was uploaded or merged from the archive, per symbol. A
//! [`PriceTable`] keeps only the dates on which every portfolio symbol has a price, adjusts the
//! closes for real stock splits, and can answer, for any date index `i`, questions that use only
//! closes up to `i`: the rolling 20-day realised volatility, the `VaR` return window and the
//! correlation. That is what makes the replay free of look-ahead.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;
use std::str::FromStr;
use std::sync::Arc;

use chrono::NaiveDate;
use domain::{ClockStep, DomainError, SpotAdjustment, Symbol};
use thiserror::Error;

use crate::calibration::{
    TRADING_DAYS_PER_YEAR, adjust_splits, correlation_matrix, ewma_vol, log_returns, realised_vol,
};
use crate::config::{DividendModel, MarketConfig};
use crate::feed::{FeedError, MarketTick, PriceFeed, Quote};
use crate::loader::DataSource;
use crate::seed::{SeedData, SymbolSeed};
use crate::source::DailyBar;

/// Dates at the start of a replay that only feed the first volatility estimate. They produce no
/// P&L. The replay starts at index `WARM_UP_DATES`, the first date with a full window of returns.
pub const WARM_UP_DATES: usize = 20;

/// Returns in the rolling realised volatility.
pub const ROLLING_VOL_WINDOW: usize = 20;

/// The fewest dates a replay needs: the warm-up, the start date and one replayed date.
pub const MIN_REPLAY_DATES: usize = WARM_UP_DATES + 2;

/// Below this a window of returns is treated as constant.
const MIN_WINDOW_VARIANCE: f64 = 1e-24;

/// A price file or price table could not be used.
#[derive(Debug, Error, PartialEq)]
pub enum ReplayDataError {
    /// The file holds no rows.
    #[error("the price file is empty")]
    Empty,
    /// The first line is neither `date,symbol,close` nor `date,SYM1,SYM2,...`.
    #[error("line {line}: bad header: {message}")]
    BadHeader {
        /// The 1-based line number.
        line: u64,
        /// What is wrong.
        message: String,
    },
    /// A row has the wrong number of columns.
    #[error("line {line}: expected {expected} columns, found {found}")]
    WrongColumnCount {
        /// The 1-based line number.
        line: u64,
        /// Columns the header promised.
        expected: usize,
        /// Columns the row has.
        found: usize,
    },
    /// The row could not be read as text.
    #[error("line {line}: {message}")]
    Unreadable {
        /// The 1-based line number.
        line: u64,
        /// What went wrong.
        message: String,
    },
    /// A date is not `YYYY-MM-DD`.
    #[error("line {line}: bad date {value:?}, expected YYYY-MM-DD")]
    BadDate {
        /// The 1-based line number.
        line: u64,
        /// The text found.
        value: String,
    },
    /// A symbol is not a valid ticker.
    #[error("line {line}: bad symbol {value:?}")]
    BadSymbol {
        /// The 1-based line number.
        line: u64,
        /// The text found.
        value: String,
    },
    /// A price is not a positive finite number.
    #[error("line {line}: bad price {value:?} for {symbol}: must be a positive, finite number")]
    BadPrice {
        /// The 1-based line number.
        line: u64,
        /// The symbol the price belongs to.
        symbol: String,
        /// The text found.
        value: String,
    },
    /// A symbol has two prices for one date.
    #[error("line {line}: {symbol} already has a price for {date}")]
    DuplicateDate {
        /// The 1-based line number of the second price.
        line: u64,
        /// The symbol.
        symbol: Symbol,
        /// The repeated date.
        date: NaiveDate,
    },
    /// Portfolio symbols with no prices in the file.
    #[error("the price data has no prices for portfolio symbol(s) {}", join(.0))]
    MissingSymbols(Vec<Symbol>),
    /// Too few common dates to warm up and replay.
    #[error("only {have} dates have a price for every symbol, {need} are needed")]
    TooFewDates {
        /// Dates with a price for every symbol.
        have: usize,
        /// Dates needed.
        need: usize,
    },
    /// The rolling volatility window is flat.
    #[error("{symbol} has a constant price over the 20 days to {date}, so its volatility is zero")]
    ZeroVolWindow {
        /// The symbol.
        symbol: Symbol,
        /// The last date of the flat window.
        date: NaiveDate,
    },
    /// No archived history for a symbol.
    #[error("the local archive has no price history for {0}")]
    NoArchiveHistory(Symbol),
    /// A domain rule failed while building a seed.
    #[error(transparent)]
    Domain(#[from] DomainError),
}

fn join(symbols: &[Symbol]) -> String {
    symbols
        .iter()
        .map(ToString::to_string)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Closes per symbol per date, exactly as read: nothing dropped, sorted or adjusted.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RawPrices {
    by_symbol: BTreeMap<Symbol, BTreeMap<NaiveDate, f64>>,
}

/// The shape of a price file, from its header.
enum Layout {
    /// `date,symbol,close`.
    Long,
    /// `date,SPY,AAPL,...`, one column per symbol.
    Wide(Vec<Symbol>),
}

impl RawPrices {
    /// Reads a CSV in long form (`date,symbol,close`) or wide form (`date,SPY,AAPL,...`), told
    /// apart by the header. Rows may come in any order. In wide form a blank cell means "no price
    /// that day". Every error names its 1-based line, counting the header as line 1.
    ///
    /// # Errors
    ///
    /// [`ReplayDataError`] for an empty file, a bad header, a wrong column count, a bad date,
    /// symbol or price (not positive and finite), or two prices for one symbol and date.
    pub fn parse_csv(bytes: &[u8]) -> Result<Self, ReplayDataError> {
        let bytes = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
        let text = std::str::from_utf8(bytes).map_err(|error| {
            let valid = bytes.get(..error.valid_up_to()).unwrap_or_default();
            ReplayDataError::Unreadable {
                line: valid.split(|&b| b == b'\n').count() as u64,
                message: "the file is not valid UTF-8 text".to_owned(),
            }
        })?;
        // Blank lines are skipped but still counted, so every line number is the one an editor shows.
        let mut lines = text
            .lines()
            .enumerate()
            .map(|(i, line)| (i as u64 + 1, line))
            .filter(|(_, line)| !line.trim().is_empty());
        let Some((header_line, header)) = lines.next() else {
            return Err(ReplayDataError::Empty);
        };
        let header = split_fields(header);
        let layout = layout_of(&header, header_line)?;
        let columns = header.len();

        let mut raw = Self::default();
        for (line, row) in lines {
            let fields = split_fields(row);
            if fields.len() != columns {
                return Err(ReplayDataError::WrongColumnCount {
                    line,
                    expected: columns,
                    found: fields.len(),
                });
            }
            let date_text = fields.first().map_or("", String::as_str);
            let date = NaiveDate::parse_from_str(date_text, "%Y-%m-%d").map_err(|_| {
                ReplayDataError::BadDate {
                    line,
                    value: date_text.to_owned(),
                }
            })?;
            match &layout {
                Layout::Long => {
                    let symbol_text = fields.get(1).map_or("", String::as_str);
                    let symbol = parse_symbol(symbol_text, line)?;
                    let price =
                        parse_price(fields.get(2).map_or("", String::as_str), symbol_text, line)?;
                    raw.insert(symbol, date, price, line)?;
                }
                Layout::Wide(symbols) => {
                    for (symbol, cell) in symbols.iter().zip(fields.iter().skip(1)) {
                        if cell.is_empty() {
                            continue;
                        }
                        let price = parse_price(cell, symbol.as_str(), line)?;
                        raw.insert(symbol.clone(), date, price, line)?;
                    }
                }
            }
        }
        if raw.by_symbol.is_empty() {
            return Err(ReplayDataError::Empty);
        }
        Ok(raw)
    }

    /// Builds prices from one symbol's daily bars per entry, such as archive history. A repeated
    /// date keeps the later bar.
    pub fn from_bars(bars: impl IntoIterator<Item = (Symbol, Vec<DailyBar>)>) -> Self {
        let mut raw = Self::default();
        for (symbol, bars) in bars {
            let series = raw.by_symbol.entry(symbol).or_default();
            for bar in bars {
                series.insert(bar.date, bar.close);
            }
        }
        raw
    }

    fn insert(
        &mut self,
        symbol: Symbol,
        date: NaiveDate,
        price: f64,
        line: u64,
    ) -> Result<(), ReplayDataError> {
        let series = self.by_symbol.entry(symbol.clone()).or_default();
        if series.insert(date, price).is_some() {
            return Err(ReplayDataError::DuplicateDate { line, symbol, date });
        }
        Ok(())
    }

    /// The symbols that have at least one price, in order.
    pub fn symbols(&self) -> impl Iterator<Item = &Symbol> {
        self.by_symbol.keys()
    }

    /// The prices as a long-form CSV (`date,symbol,close`), sorted by date then symbol. This is
    /// what a run saves as `input.csv` when the source was the archive.
    pub fn to_long_csv(&self) -> String {
        let mut rows: Vec<(NaiveDate, &Symbol, f64)> = self
            .by_symbol
            .iter()
            .flat_map(|(symbol, series)| series.iter().map(move |(d, c)| (*d, symbol, *c)))
            .collect();
        rows.sort_by(|a, b| (a.0, a.1).cmp(&(b.0, b.1)));
        let mut out = String::from("date,symbol,close\n");
        for (date, symbol, close) in rows {
            // Writing to a String cannot fail.
            let _ = writeln!(out, "{date},{symbol},{close}");
        }
        out
    }
}

/// Splits one line on commas, trimming each field and any quotes around it. Quoted fields with
/// commas or line breaks inside are not supported: a price file has none.
fn split_fields(line: &str) -> Vec<String> {
    line.split(',')
        .map(|field| field.trim().trim_matches('"').trim().to_owned())
        .collect()
}

fn layout_of(header: &[String], line: u64) -> Result<Layout, ReplayDataError> {
    let names: Vec<String> = header.iter().map(|s| s.to_ascii_lowercase()).collect();
    let bad = |message: &str| ReplayDataError::BadHeader {
        line,
        message: message.to_owned(),
    };
    if names.first().map(String::as_str) != Some("date") {
        return Err(bad("the first column must be `date`"));
    }
    if names.len() == 3 && names.get(1).map(String::as_str) == Some("symbol") {
        if names.get(2).map(String::as_str) == Some("close") {
            return Ok(Layout::Long);
        }
        return Err(bad("long form is `date,symbol,close`"));
    }
    if names.len() < 2 {
        return Err(bad("expected `date,symbol,close` or `date,SYM1,SYM2,...`"));
    }
    let mut symbols = Vec::with_capacity(names.len() - 1);
    for name in header.iter().skip(1) {
        let symbol = parse_symbol(name, line)?;
        if symbols.contains(&symbol) {
            return Err(bad(&format!("column {symbol} appears twice")));
        }
        symbols.push(symbol);
    }
    Ok(Layout::Wide(symbols))
}

fn parse_symbol(text: &str, line: u64) -> Result<Symbol, ReplayDataError> {
    Symbol::new(&text.to_ascii_uppercase()).map_err(|_| ReplayDataError::BadSymbol {
        line,
        value: text.to_owned(),
    })
}

fn parse_price(text: &str, symbol: &str, line: u64) -> Result<f64, ReplayDataError> {
    match f64::from_str(text) {
        Ok(value) if value.is_finite() && value > 0.0 => Ok(value),
        _ => Err(ReplayDataError::BadPrice {
            line,
            symbol: symbol.to_owned(),
            value: text.to_owned(),
        }),
    }
}

/// A price break that looks like a split: the first date at the new price and the ratio, where
/// 4.0 is a 4-for-1 split.
#[derive(Debug, Clone, PartialEq)]
pub struct PriceBreak {
    /// The symbol.
    pub symbol: Symbol,
    /// The first date at the new price.
    pub date: NaiveDate,
    /// New shares per old share.
    pub ratio: f64,
}

/// The `VaR` inputs at one date: the trailing returns and their correlation.
#[derive(Debug, Clone, PartialEq)]
pub struct VarWindow {
    /// The dates of the returns, oldest first. A return is dated by its later close.
    pub dates: Vec<NaiveDate>,
    /// One row per date, one column per symbol in table order.
    pub returns: Vec<Vec<f64>>,
    /// The correlation matrix of the returns.
    pub correlation: Vec<Vec<f64>>,
}

/// Aligned, split-adjusted closes for the portfolio's symbols, with rolling statistics.
///
/// Every method that takes a date index `i` reads closes at indices up to `i` and no later, so
/// nothing here can leak a future price into a replay date.
#[derive(Debug, Clone)]
pub struct PriceTable {
    symbols: Vec<Symbol>,
    dates: Vec<NaiveDate>,
    /// Closes as read, per symbol per date index.
    raw: Vec<Vec<f64>>,
    /// Closes after split handling.
    closes: Vec<Vec<f64>>,
    /// Rolling realised vol per symbol per date index, `NaN` before [`WARM_UP_DATES`].
    vols: Vec<Vec<f64>>,
    dropped_dates: usize,
    ignored_symbols: Vec<Symbol>,
    detected: Vec<PriceBreak>,
}

impl PriceTable {
    /// Aligns `raw` on the dates where every one of `symbols` has a price, and back-adjusts every
    /// detected split so no split shows as a one-day collapse. Each detection is logged.
    ///
    /// Symbols in `raw` that are not in `symbols` are ignored and listed by
    /// [`ignored_symbols`](Self::ignored_symbols). Dates missing a price for any of `symbols` are
    /// dropped and counted by [`dropped_dates`](Self::dropped_dates).
    ///
    /// # Errors
    ///
    /// [`ReplayDataError::MissingSymbols`] when a portfolio symbol has no prices,
    /// [`ReplayDataError::TooFewDates`] under [`MIN_REPLAY_DATES`] common dates, and
    /// [`ReplayDataError::ZeroVolWindow`] for a flat 20-day window.
    pub fn new(raw: &RawPrices, symbols: &[Symbol]) -> Result<Self, ReplayDataError> {
        let symbols: Vec<Symbol> = symbols
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let missing: Vec<Symbol> = symbols
            .iter()
            .filter(|s| !raw.by_symbol.contains_key(*s))
            .cloned()
            .collect();
        if !missing.is_empty() {
            return Err(ReplayDataError::MissingSymbols(missing));
        }
        let series: Vec<&BTreeMap<NaiveDate, f64>> = symbols
            .iter()
            .filter_map(|s| raw.by_symbol.get(s))
            .collect();
        let union: BTreeSet<NaiveDate> = series.iter().flat_map(|m| m.keys().copied()).collect();
        let dates: Vec<NaiveDate> = union
            .iter()
            .copied()
            .filter(|d| series.iter().all(|m| m.contains_key(d)))
            .collect();
        let dropped_dates = union.len() - dates.len();
        if dates.len() < MIN_REPLAY_DATES {
            return Err(ReplayDataError::TooFewDates {
                have: dates.len(),
                need: MIN_REPLAY_DATES,
            });
        }
        let raw_closes: Vec<Vec<f64>> = series
            .iter()
            .map(|m| dates.iter().filter_map(|d| m.get(d).copied()).collect())
            .collect();

        let mut detected = Vec::new();
        for (symbol, map) in symbols.iter().zip(&series) {
            let bars: Vec<DailyBar> = map
                .iter()
                .map(|(&date, &close)| DailyBar { date, close })
                .collect();
            let (_, splits) = adjust_splits(symbol, &bars);
            detected.extend(splits.into_iter().map(|split| PriceBreak {
                symbol: symbol.clone(),
                date: split.date,
                ratio: split.ratio,
            }));
        }
        let ignored_symbols = raw
            .by_symbol
            .keys()
            .filter(|s| !symbols.contains(s))
            .cloned()
            .collect();
        let mut table = Self {
            symbols,
            dates,
            closes: raw_closes.clone(),
            raw: raw_closes,
            vols: Vec::new(),
            dropped_dates,
            ignored_symbols,
            detected,
        };
        table.rebuild(&[])?;
        Ok(table)
    }

    /// Chooses which detected splits the engine will apply itself, and rebuilds the closes.
    ///
    /// A detected split that matches an entry of `kept` (same symbol and date) is **left in the
    /// prices**: the symbol's series is expressed in pre-split terms from that date on
    /// (closes × the entry's ratio), and the engine, which divides the spot by the ratio when it
    /// applies the split, brings it back to the real close while adjusting the book. Every other
    /// detected split is back-adjusted: earlier closes are divided by its ratio.
    ///
    /// # Errors
    ///
    /// [`ReplayDataError::ZeroVolWindow`] if the rebuilt series has a flat window.
    pub fn keep_splits(&mut self, kept: &[PriceBreak]) -> Result<(), ReplayDataError> {
        self.rebuild(kept)
    }

    fn rebuild(&mut self, kept: &[PriceBreak]) -> Result<(), ReplayDataError> {
        let mut closes = Vec::with_capacity(self.symbols.len());
        for (symbol, raw) in self.symbols.iter().zip(&self.raw) {
            let breaks: Vec<(NaiveDate, f64, bool)> = self
                .detected
                .iter()
                .filter(|b| &b.symbol == symbol)
                .map(|b| {
                    let keep = kept
                        .iter()
                        .find(|k| &k.symbol == symbol && k.date == b.date);
                    (b.date, keep.map_or(b.ratio, |k| k.ratio), keep.is_some())
                })
                .collect();
            let adjusted: Vec<f64> = self
                .dates
                .iter()
                .zip(raw)
                .map(|(&date, &close)| {
                    let factor: f64 = breaks
                        .iter()
                        .map(
                            |&(split_date, ratio, keep)| match (keep, date >= split_date) {
                                (true, true) => ratio,
                                (false, false) => 1.0 / ratio,
                                _ => 1.0,
                            },
                        )
                        .product();
                    close * factor
                })
                .collect();
            closes.push(adjusted);
        }
        let mut vols = Vec::with_capacity(closes.len());
        for (symbol, series) in self.symbols.iter().zip(&closes) {
            let returns = log_returns(series);
            let mut column = vec![f64::NAN; series.len()];
            for index in WARM_UP_DATES..series.len() {
                let window = returns
                    .get(index - ROLLING_VOL_WINDOW..index)
                    .unwrap_or_default();
                let vol = realised_vol(window);
                if !vol.is_finite() || vol * vol / TRADING_DAYS_PER_YEAR <= MIN_WINDOW_VARIANCE {
                    return Err(ReplayDataError::ZeroVolWindow {
                        symbol: symbol.clone(),
                        date: self.dates.get(index).copied().unwrap_or(NaiveDate::MIN),
                    });
                }
                if let Some(slot) = column.get_mut(index) {
                    *slot = vol;
                }
            }
            vols.push(column);
        }
        self.closes = closes;
        self.vols = vols;
        Ok(())
    }

    /// The symbols, in the column order of every per-symbol method.
    pub fn symbols(&self) -> &[Symbol] {
        &self.symbols
    }

    /// The common dates, oldest first.
    pub fn dates(&self) -> &[NaiveDate] {
        &self.dates
    }

    /// How many dates the table holds.
    pub fn len(&self) -> usize {
        self.dates.len()
    }

    /// Whether the table holds no dates. Never true for a table that was built.
    pub fn is_empty(&self) -> bool {
        self.dates.is_empty()
    }

    /// The index of the first replay date: the first with a full window of returns.
    pub fn start_index(&self) -> usize {
        WARM_UP_DATES
    }

    /// The date at [`start_index`](Self::start_index).
    pub fn start_date(&self) -> Option<NaiveDate> {
        self.dates.get(WARM_UP_DATES).copied()
    }

    /// The last date.
    pub fn end_date(&self) -> Option<NaiveDate> {
        self.dates.last().copied()
    }

    /// The index of `date`, if the table has it.
    pub fn index_of(&self, date: NaiveDate) -> Option<usize> {
        self.dates.binary_search(&date).ok()
    }

    /// How many dates were dropped for lacking a price for some symbol.
    pub fn dropped_dates(&self) -> usize {
        self.dropped_dates
    }

    /// Symbols in the input that are not in the portfolio and were ignored.
    pub fn ignored_symbols(&self) -> &[Symbol] {
        &self.ignored_symbols
    }

    /// The price breaks that look like splits, before any choice about keeping them.
    pub fn detected_splits(&self) -> &[PriceBreak] {
        &self.detected
    }

    /// One symbol's close at date index `index` after split handling.
    pub fn close(&self, symbol_index: usize, index: usize) -> Option<f64> {
        self.closes.get(symbol_index)?.get(index).copied()
    }

    /// One symbol's rolling realised vol at `index`: the sample standard deviation of the 20 log
    /// returns ending at `index`, times √252. `None` before [`WARM_UP_DATES`].
    pub fn vol(&self, symbol_index: usize, index: usize) -> Option<f64> {
        self.vols
            .get(symbol_index)?
            .get(index)
            .copied()
            .filter(|v| v.is_finite())
    }

    /// The trailing `VaR` window at `index`: up to `max_returns` log returns ending at `index`
    /// (each dated by its later close) and their correlation. Uses no close after `index`.
    ///
    /// A window too short for a correlation, or one with a constant column, gives the identity
    /// matrix and a warning rather than an error, since a `VaR` figure is better than none.
    pub fn var_window(&self, index: usize, max_returns: usize) -> VarWindow {
        let count = max_returns.min(index);
        let first = index + 1 - count;
        let dates: Vec<NaiveDate> = (first..=index)
            .filter_map(|j| self.dates.get(j).copied())
            .collect();
        let columns: Vec<Vec<f64>> = self
            .closes
            .iter()
            .map(|series| {
                let closes = series.get(first - 1..=index).unwrap_or_default();
                log_returns(closes)
            })
            .collect();
        let width = self.symbols.len();
        let returns: Vec<Vec<f64>> = (0..dates.len())
            .map(|row| {
                columns
                    .iter()
                    .filter_map(|column| column.get(row).copied())
                    .collect()
            })
            .collect();
        let correlation = if dates.len() >= 2 {
            correlation_matrix(&self.symbols, &columns).unwrap_or_else(|error| {
                tracing::warn!(%error, index, "replay correlation window unusable, using independence");
                identity(width)
            })
        } else {
            identity(width)
        };
        VarWindow {
            dates,
            returns,
            correlation,
        }
    }

    /// The market seed at date `index`: spots, rolling vols, yields and rate from `market`, and
    /// the `VaR` window of up to `market.vol_window_days` returns. The engine builds its market
    /// state and `VaR` calculator from this, exactly as it does from a calibrated seed.
    ///
    /// A symbol on the discrete dividend model has a yield of zero, as in [`calibrate`](crate::calibrate).
    /// A symbol the config has no yield for gets zero, with a warning.
    ///
    /// # Errors
    ///
    /// [`ReplayDataError::ZeroVolWindow`] when `index` is before the warm-up ends.
    pub fn seed_at(
        &self,
        index: usize,
        market: &MarketConfig,
    ) -> Result<SeedData, ReplayDataError> {
        let date = self.dates.get(index).copied().unwrap_or(NaiveDate::MIN);
        let window = self.var_window(index, market.vol_window_days.max(ROLLING_VOL_WINDOW));
        let mut underlyings = BTreeMap::new();
        for (i, symbol) in self.symbols.iter().enumerate() {
            let (Some(spot), Some(vol)) = (self.close(i, index), self.vol(i, index)) else {
                return Err(ReplayDataError::ZeroVolWindow {
                    symbol: symbol.clone(),
                    date,
                });
            };
            let recent: Vec<f64> = window
                .returns
                .iter()
                .rev()
                .take(ROLLING_VOL_WINDOW)
                .rev()
                .filter_map(|row| row.get(i).copied())
                .collect();
            let dividend_yield =
                match market.model_of(symbol) {
                    DividendModel::Discrete => 0.0,
                    DividendModel::Yield => market
                        .dividend_yields
                        .get(symbol)
                        .copied()
                        .unwrap_or_else(|| {
                            tracing::warn!(%symbol, "no dividend yield configured, using zero");
                            0.0
                        }),
                };
            underlyings.insert(
                symbol.clone(),
                SymbolSeed {
                    spot,
                    vol_realised: vol,
                    vol_ewma: ewma_vol(&recent),
                    vol,
                    dividend_yield,
                    source: DataSource::Replay {
                        origin: "price table".to_owned(),
                    },
                },
            );
        }
        Ok(SeedData {
            as_of: date,
            rate: market.risk_free_rate,
            symbols: self.symbols.clone(),
            underlyings,
            return_dates: window.dates,
            returns: window.returns,
            correlation: window.correlation,
            dividend_schedule: BTreeMap::new(),
        })
    }
}

fn identity(n: usize) -> Vec<Vec<f64>> {
    (0..n)
        .map(|i| (0..n).map(|j| f64::from(u8::from(i == j))).collect())
        .collect()
}

/// A [`PriceFeed`] that reads closes from a [`PriceTable`] instead of simulating them.
///
/// [`step`](PriceFeed::step) finds the step's `session_date` in the table and returns that date's
/// close and rolling 20-day realised vol for every symbol. It is a pure lookup, so a paused (zero
/// length) step returns the same quotes again and no step can see a later date.
///
/// **Splits.** A split the engine applies (see [`PriceTable::keep_splits`]) calls
/// [`adjust_spot`](PriceFeed::adjust_spot) with `DivideBy(ratio)`. The feed keeps a divisor per
/// symbol and emits close ÷ divisor from then on, which is how the engine's live-mode split
/// path works unchanged on replayed prices.
///
/// **Dividends.** The closes are real, so they already fell on the ex-date.
/// [`prices_include_dividends`](PriceFeed::prices_include_dividends) is true, which tells the
/// engine not to lower the spot again, and a `Subtract` adjustment is accepted and ignored.
#[derive(Debug, Clone)]
pub struct ReplayFeed {
    table: Arc<PriceTable>,
    divisors: Vec<f64>,
}

impl ReplayFeed {
    /// A feed over `table`.
    pub fn new(table: Arc<PriceTable>) -> Self {
        let divisors = vec![1.0; table.symbols().len()];
        Self { table, divisors }
    }
}

impl PriceFeed for ReplayFeed {
    fn step(&mut self, step: &ClockStep) -> Result<MarketTick, FeedError> {
        let index = self.table.index_of(step.session_date).ok_or_else(|| {
            FeedError::InvalidStep(format!("{} is not a replay date", step.session_date))
        })?;
        let mut quotes = BTreeMap::new();
        for (i, symbol) in self.table.symbols().iter().enumerate() {
            let divisor = self.divisors.get(i).copied().unwrap_or(1.0);
            let (Some(close), Some(vol)) = (self.table.close(i, index), self.table.vol(i, index))
            else {
                return Err(FeedError::InvalidStep(format!(
                    "no volatility for {symbol} on {}: inside the warm-up",
                    step.session_date
                )));
            };
            quotes.insert(
                symbol.clone(),
                Quote {
                    spot: close / divisor,
                    vol,
                },
            );
        }
        Ok(MarketTick {
            time: step.to,
            kind: step.kind,
            quotes,
        })
    }

    fn adjust_spot(
        &mut self,
        symbol: &Symbol,
        adjustment: SpotAdjustment,
    ) -> Result<(), FeedError> {
        let position = self
            .table
            .symbols()
            .iter()
            .position(|s| s == symbol)
            .ok_or_else(|| FeedError::UnknownSymbol(symbol.clone()))?;
        match adjustment {
            SpotAdjustment::DivideBy(ratio) => {
                if !ratio.is_finite() || ratio <= 0.0 {
                    return Err(FeedError::Domain(DomainError::InvalidSplitRatio(ratio)));
                }
                if let Some(divisor) = self.divisors.get_mut(position) {
                    *divisor *= ratio;
                }
            }
            SpotAdjustment::Subtract(_) => {}
        }
        Ok(())
    }

    fn prices_include_dividends(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::{Duration, TimeZone, Utc};
    use domain::StepKind;

    use super::*;

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn day(i: usize) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 1, 5).unwrap() + Duration::days(i64::try_from(i).unwrap())
    }

    /// Alternating log returns of +a and -a: the price takes two values and every 20-return
    /// window holds ten of each, so the rolling vol is the constant sqrt(20/19 a^2 252).
    fn alternating(start: f64, a: f64, n: usize) -> Vec<f64> {
        let mut close = start;
        (0..n)
            .map(|i| {
                if i > 0 {
                    close *= if i % 2 == 1 { a.exp() } else { (-a).exp() };
                }
                close
            })
            .collect()
    }

    fn long_csv(series: &[(&str, Vec<f64>)]) -> String {
        let mut out = String::from("date,symbol,close\n");
        for (symbol, closes) in series {
            for (i, close) in closes.iter().enumerate() {
                let _ = writeln!(out, "{},{symbol},{close}", day(i));
            }
        }
        out
    }

    fn wide_csv(series: &[(&str, Vec<f64>)]) -> String {
        let names: Vec<&str> = series.iter().map(|(s, _)| *s).collect();
        let mut out = format!("date,{}\n", names.join(","));
        let n = series.first().map_or(0, |(_, c)| c.len());
        for i in 0..n {
            let cells: Vec<String> = series.iter().map(|(_, c)| c[i].to_string()).collect();
            let _ = writeln!(out, "{},{}", day(i), cells.join(","));
        }
        out
    }

    fn table(series: &[(&str, Vec<f64>)]) -> PriceTable {
        let raw = RawPrices::parse_csv(long_csv(series).as_bytes()).unwrap();
        let symbols: Vec<Symbol> = series.iter().map(|(s, _)| sym(s)).collect();
        PriceTable::new(&raw, &symbols).unwrap()
    }

    fn error_of(text: &str) -> ReplayDataError {
        RawPrices::parse_csv(text.as_bytes()).unwrap_err()
    }

    // --- parsing and validation --------------------------------------------------------

    #[test]
    fn long_and_wide_forms_read_the_same_prices() {
        let series = [
            ("SPY", alternating(100.0, 0.01, 30)),
            ("AAPL", alternating(50.0, 0.02, 30)),
        ];
        let long = RawPrices::parse_csv(long_csv(&series).as_bytes()).unwrap();
        let wide = RawPrices::parse_csv(wide_csv(&series).as_bytes()).unwrap();
        assert_eq!(long, wide);
    }

    #[test]
    fn unsorted_rows_are_sorted() {
        let raw = RawPrices::parse_csv(
            b"date,symbol,close\n2026-01-06,SPY,101\n2026-01-05,SPY,100\n2026-01-07,SPY,102\n",
        )
        .unwrap();
        assert!(raw.to_long_csv().starts_with(
            "date,symbol,close\n2026-01-05,SPY,100\n2026-01-06,SPY,101\n2026-01-07,SPY,102\n"
        ));
    }

    #[test]
    fn a_bom_and_windows_line_endings_are_fine() {
        let raw = RawPrices::parse_csv(b"\xEF\xBB\xBFdate,symbol,close\r\n2026-01-05,spy,100\r\n")
            .unwrap();
        assert_eq!(raw.symbols().next(), Some(&sym("SPY")));
    }

    #[test]
    fn errors_carry_the_line_number() {
        assert_eq!(error_of(""), ReplayDataError::Empty);
        assert_eq!(error_of("date,symbol,close\n"), ReplayDataError::Empty);
        assert!(matches!(
            error_of("day,symbol,close\n"),
            ReplayDataError::BadHeader { line: 1, .. }
        ));
        assert!(matches!(
            error_of("date\n2026-01-05\n"),
            ReplayDataError::BadHeader { line: 1, .. }
        ));
        assert!(matches!(
            error_of("date,SPY,SPY\n"),
            ReplayDataError::BadHeader { line: 1, .. }
        ));
        assert_eq!(
            error_of("date,symbol,close\n2026-01-05,SPY,100\n2026-01-06,SPY\n"),
            ReplayDataError::WrongColumnCount {
                line: 3,
                expected: 3,
                found: 2
            }
        );
        assert_eq!(
            error_of("date,symbol,close\n2026-01-05,SPY,100\n\n2026-13-06,SPY,101\n"),
            ReplayDataError::BadDate {
                line: 4,
                value: "2026-13-06".into()
            }
        );
        assert!(matches!(
            error_of("date,symbol,close\n2026-01-05,SP Y!,100\n"),
            ReplayDataError::BadSymbol { line: 2, .. }
        ));
        for bad in ["-1", "0", "abc", "NaN", "inf", ""] {
            let text = format!("date,symbol,close\n2026-01-05,SPY,100\n2026-01-06,SPY,{bad}\n");
            assert!(
                matches!(error_of(&text), ReplayDataError::BadPrice { line: 3, .. }),
                "{bad:?}"
            );
        }
        assert_eq!(
            error_of(
                "date,symbol,close\n2026-01-05,SPY,100\n2026-01-06,SPY,101\n2026-01-05,SPY,102\n"
            ),
            ReplayDataError::DuplicateDate {
                line: 4,
                symbol: sym("SPY"),
                date: NaiveDate::from_ymd_opt(2026, 1, 5).unwrap()
            }
        );
        // The same date for two different symbols is fine.
        assert!(
            RawPrices::parse_csv(b"date,symbol,close\n2026-01-05,SPY,1\n2026-01-05,AAPL,2\n")
                .is_ok()
        );
        // Wide form: a repeated date row.
        assert!(matches!(
            error_of("date,SPY,AAPL\n2026-01-05,1,2\n2026-01-06,1,2\n2026-01-05,3,\n"),
            ReplayDataError::DuplicateDate { line: 4, .. }
        ));
        assert!(matches!(
            error_of("date,SPY,AAPL\n2026-01-05,1,-2\n"),
            ReplayDataError::BadPrice { line: 2, .. }
        ));
    }

    #[test]
    fn a_missing_portfolio_symbol_is_an_error() {
        let raw =
            RawPrices::parse_csv(long_csv(&[("SPY", alternating(100.0, 0.01, 30))]).as_bytes())
                .unwrap();
        let error = PriceTable::new(&raw, &[sym("SPY"), sym("TSLA")]).unwrap_err();
        assert_eq!(error, ReplayDataError::MissingSymbols(vec![sym("TSLA")]));
        assert!(error.to_string().contains("TSLA"));
    }

    #[test]
    fn only_dates_with_every_symbol_are_kept_and_the_drops_are_counted() {
        let spy = alternating(100.0, 0.01, 30);
        let aapl = alternating(50.0, 0.02, 30);
        let mut text = long_csv(&[("SPY", spy), ("AAPL", aapl)]);
        // Two dates only SPY has, and an extra symbol nobody asked for.
        text += "2027-01-01,SPY,100\n2027-01-02,SPY,101\n2026-01-05,QQQ,300\n";
        let raw = RawPrices::parse_csv(text.as_bytes()).unwrap();
        let table = PriceTable::new(&raw, &[sym("SPY"), sym("AAPL")]).unwrap();
        assert_eq!(table.len(), 30);
        assert_eq!(table.dropped_dates(), 2);
        assert_eq!(table.ignored_symbols(), [sym("QQQ")]);
    }

    #[test]
    fn too_few_dates_and_flat_windows_are_errors() {
        let raw =
            RawPrices::parse_csv(long_csv(&[("SPY", alternating(100.0, 0.01, 21))]).as_bytes())
                .unwrap();
        assert_eq!(
            PriceTable::new(&raw, &[sym("SPY")]).unwrap_err(),
            ReplayDataError::TooFewDates { have: 21, need: 22 }
        );
        let flat = RawPrices::parse_csv(long_csv(&[("SPY", vec![100.0; 30])]).as_bytes()).unwrap();
        assert!(matches!(
            PriceTable::new(&flat, &[sym("SPY")]),
            Err(ReplayDataError::ZeroVolWindow { .. })
        ));
    }

    // --- rolling statistics and look-ahead ---------------------------------------------

    #[test]
    fn the_rolling_vol_of_an_alternating_series_is_a_constant() {
        let a: f64 = 0.01;
        let table = table(&[("SPY", alternating(100.0, a, 40))]);
        let expected = (20.0 / 19.0 * a * a * 252.0).sqrt();
        assert!(table.vol(0, WARM_UP_DATES - 1).is_none());
        for i in WARM_UP_DATES..40 {
            assert_relative_eq!(table.vol(0, i).unwrap(), expected, max_relative = 1e-9);
        }
        assert_eq!(table.start_date(), Some(day(20)));
    }

    #[test]
    fn nothing_after_a_date_can_change_the_statistics_at_that_date() {
        let base = alternating(100.0, 0.01, 40);
        let mut changed = base.clone();
        for close in changed.iter_mut().skip(31) {
            *close *= 1.7;
        }
        let a = table(&[("SPY", base), ("AAPL", alternating(50.0, 0.02, 40))]);
        let b = table(&[("SPY", changed), ("AAPL", alternating(50.0, 0.02, 40))]);
        for index in [20, 25, 30] {
            assert_eq!(a.var_window(index, 60), b.var_window(index, 60));
            assert_eq!(a.vol(0, index), b.vol(0, index));
            assert_eq!(
                a.seed_at(index, &market()).unwrap(),
                b.seed_at(index, &market()).unwrap()
            );
        }
        assert_ne!(a.var_window(35, 60), b.var_window(35, 60));
    }

    #[test]
    fn the_var_window_is_the_trailing_returns_ending_at_the_date() {
        let table = table(&[
            ("SPY", alternating(100.0, 0.01, 40)),
            ("AAPL", alternating(50.0, 0.02, 40)),
        ]);
        let window = table.var_window(30, 25);
        assert_eq!(window.dates.len(), 25);
        assert_eq!(window.dates.last(), Some(&day(30)));
        assert_eq!(window.dates.first(), Some(&day(6)));
        assert_eq!(window.returns.len(), 25);
        assert!(window.returns.iter().all(|row| row.len() == 2));
        // The last return is dated day 30, a down move for SPY (column 1, after AAPL) at i = 30.
        assert_relative_eq!(
            window.returns.last().unwrap()[1],
            -0.01,
            max_relative = 1e-9
        );
        // Perfectly (anti)correlated alternating series: correlation is exactly 1 on the diagonal.
        assert_relative_eq!(window.correlation[0][0], 1.0);
        assert_relative_eq!(window.correlation[0][1], 1.0, max_relative = 1e-9);
        // Early in the series the window is shorter than asked for.
        assert_eq!(table.var_window(3, 60).dates.len(), 3);
    }

    fn market() -> MarketConfig {
        MarketConfig::from_toml_str(
            r#"
            symbols = ["SPY", "AAPL"]
            risk_free_rate = 0.04
            vol_window_days = 60
            vol_method = "realised"
            cache_max_age_hours = 24
            data_mode = "offline"
            [dividend_yields]
            SPY = 0.01
            AAPL = 0.005
            [dividend_model]
            SPY = "discrete"
            "#,
        )
        .unwrap()
    }

    #[test]
    fn the_seed_carries_the_spot_vol_yield_and_window() {
        let table = table(&[
            ("SPY", alternating(100.0, 0.01, 40)),
            ("AAPL", alternating(50.0, 0.02, 40)),
        ]);
        let seed = table.seed_at(20, &market()).unwrap();
        assert_eq!(seed.as_of, day(20));
        assert_eq!(seed.symbols, [sym("AAPL"), sym("SPY")]);
        assert_relative_eq!(seed.rate, 0.04);
        let spy = &seed.underlyings[&sym("SPY")];
        assert_relative_eq!(spy.spot, table.close(1, 20).unwrap());
        assert_relative_eq!(spy.vol, table.vol(1, 20).unwrap());
        // SPY is on the discrete dividend model, so its continuous yield is zero.
        assert_relative_eq!(spy.dividend_yield, 0.0);
        assert_relative_eq!(seed.underlyings[&sym("AAPL")].dividend_yield, 0.005);
        assert_eq!(seed.return_dates.len(), 20);
        assert_eq!(seed.returns.len(), 20);
        assert!(seed.dividend_schedule.is_empty());
        // Before the warm-up ends there is no vol.
        assert!(table.seed_at(10, &market()).is_err());
    }

    // --- splits ------------------------------------------------------------------------

    /// 40 closes on an alternating pattern, then a 4-for-1 split at index 30: prices from there
    /// on are a quarter of what they would have been.
    fn split_series() -> Vec<f64> {
        alternating(100.0, 0.01, 40)
            .into_iter()
            .enumerate()
            .map(|(i, c)| if i >= 30 { c / 4.0 } else { c })
            .collect()
    }

    #[test]
    fn a_real_split_is_back_adjusted_and_logged_not_shown_as_a_collapse() {
        let raw = split_series();
        let table = table(&[("AAPL", raw.clone())]);
        assert_eq!(
            table.detected_splits(),
            [PriceBreak {
                symbol: sym("AAPL"),
                date: day(30),
                ratio: 4.0
            }]
        );
        // Earlier closes are divided by 4, so the series is on the post-split basis throughout.
        assert_relative_eq!(
            table.close(0, 0).unwrap(),
            raw[0] / 4.0,
            max_relative = 1e-12
        );
        assert_relative_eq!(table.close(0, 35).unwrap(), raw[35], max_relative = 1e-12);
        // No return anywhere is anywhere near -75 %.
        let window = table.var_window(39, 60);
        assert!(window.returns.iter().all(|row| row[0].abs() < 0.05));
    }

    #[test]
    fn a_kept_split_is_left_in_pre_split_terms_for_the_engine_to_apply() {
        let raw = split_series();
        let mut table = table(&[("AAPL", raw.clone())]);
        let kept = PriceBreak {
            symbol: sym("AAPL"),
            date: day(30),
            ratio: 4.0,
        };
        table.keep_splits(std::slice::from_ref(&kept)).unwrap();
        // Before the split: the real closes. From the split: real close x 4, pre-split terms.
        assert_relative_eq!(table.close(0, 10).unwrap(), raw[10], max_relative = 1e-12);
        assert_relative_eq!(
            table.close(0, 30).unwrap(),
            raw[30] * 4.0,
            max_relative = 1e-12
        );

        let mut feed = ReplayFeed::new(Arc::new(table));
        let step = |i: usize| ClockStep {
            from: Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap(),
            to: Utc.with_ymd_and_hms(2026, 1, 1, 21, 0, 0).unwrap(),
            trading_years: 1.0 / 252.0,
            calendar_years: 1.0 / 365.0,
            session_date: day(i),
            kind: StepKind::Overnight,
        };
        // The step onto the split date still emits pre-split terms; the engine then divides.
        let tick = feed.step(&step(30)).unwrap();
        assert_relative_eq!(
            tick.quotes[&sym("AAPL")].spot,
            raw[30] * 4.0,
            max_relative = 1e-12
        );
        feed.adjust_spot(&sym("AAPL"), SpotAdjustment::DivideBy(4.0))
            .unwrap();
        // From then on the feed emits the real post-split closes.
        let tick = feed.step(&step(31)).unwrap();
        assert_relative_eq!(
            tick.quotes[&sym("AAPL")].spot,
            raw[31],
            max_relative = 1e-12
        );
        // Volatility is unaffected by the split either way.
        assert_relative_eq!(
            tick.quotes[&sym("AAPL")].vol,
            feed.table.vol(0, 31).unwrap()
        );
    }

    // --- the feed ----------------------------------------------------------------------

    #[test]
    fn the_feed_is_a_pure_lookup_of_close_and_vol() {
        let table = Arc::new(table(&[("SPY", alternating(100.0, 0.01, 40))]));
        let mut feed = ReplayFeed::new(Arc::clone(&table));
        assert!(feed.prices_include_dividends());
        let step = ClockStep {
            from: Utc.with_ymd_and_hms(2026, 1, 25, 21, 0, 0).unwrap(),
            to: Utc.with_ymd_and_hms(2026, 1, 26, 21, 0, 0).unwrap(),
            trading_years: 1.0 / 252.0,
            calendar_years: 1.0 / 365.0,
            session_date: day(21),
            kind: StepKind::Overnight,
        };
        let first = feed.step(&step).unwrap();
        let again = feed.step(&step).unwrap();
        assert_eq!(first, again);
        assert_eq!(first.time, step.to);
        assert_eq!(first.kind, StepKind::Overnight);
        let quote = first.quotes[&sym("SPY")];
        assert_relative_eq!(quote.spot, table.close(0, 21).unwrap());
        assert_relative_eq!(quote.vol, table.vol(0, 21).unwrap());
        // A dividend adjustment is accepted and changes nothing: the close already fell.
        feed.adjust_spot(&sym("SPY"), SpotAdjustment::Subtract(5.0))
            .unwrap();
        assert_eq!(feed.step(&step).unwrap(), first);
        assert!(matches!(
            feed.adjust_spot(&sym("NOPE"), SpotAdjustment::DivideBy(2.0)),
            Err(FeedError::UnknownSymbol(_))
        ));
        let stray = ClockStep {
            session_date: NaiveDate::from_ymd_opt(2030, 1, 1).unwrap(),
            ..step
        };
        assert!(matches!(feed.step(&stray), Err(FeedError::InvalidStep(_))));
        let early = ClockStep {
            session_date: day(5),
            ..step
        };
        assert!(matches!(feed.step(&early), Err(FeedError::InvalidStep(_))));
    }
}
