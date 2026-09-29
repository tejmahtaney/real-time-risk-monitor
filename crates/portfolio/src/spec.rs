//! The CSV portfolio format: parsing rows into unresolved [`PositionSpec`]s.
//!
//! Columns are `id,underlying,kind,strike,expiry,quantity,entry_price,iv`.
//! Lines starting with `#` are comments. A spec is *unresolved* because its
//! strike and expiry may be relative (`ATM+5%`, `+30d`) and its entry price may
//! be blank. [`crate::resolve`] turns specs into positions against a market.

use std::fmt;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::str::FromStr;

use chrono::NaiveDate;
use csv::{ReaderBuilder, StringRecord, Trim, WriterBuilder};
use domain::{Instrument, InstrumentKind, Position, PositionId, Symbol};
use serde::{Deserialize, Serialize};

use crate::error::PortfolioError;

/// Longest relative expiry accepted, in days (about ten years).
const MAX_RELATIVE_DAYS: u32 = 3650;

/// The columns, in canonical order.
const COLUMNS: [&str; 8] = [
    "id",
    "underlying",
    "kind",
    "strike",
    "expiry",
    "quantity",
    "entry_price",
    "iv",
];

/// A strike as written in the file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub enum StrikeSpec {
    /// A fixed strike in dollars, such as `230`.
    Absolute(f64),
    /// The spot at load time plus `offset_pct` per cent, such as `ATM`, `ATM+5%` or `ATM-10%`.
    Atm {
        /// Offset in per cent. `5.0` means 5 % above the spot.
        offset_pct: f64,
    },
}

impl FromStr for StrikeSpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = s.trim();
        if let Ok(value) = text.parse::<f64>() {
            return if value.is_finite() && value > 0.0 {
                Ok(Self::Absolute(value))
            } else {
                Err(format!("strike must be a positive number, got {text:?}"))
            };
        }
        let upper = text.to_ascii_uppercase();
        let Some(rest) = upper.strip_prefix("ATM") else {
            return Err(format!(
                "expected a number, ATM, ATM+N% or ATM-N%, got {text:?}"
            ));
        };
        if rest.is_empty() {
            return Ok(Self::Atm { offset_pct: 0.0 });
        }
        let bad = || format!("expected ATM+N% or ATM-N%, got {text:?}");
        let number = rest.strip_suffix('%').ok_or_else(bad)?;
        if !(number.starts_with('+') || number.starts_with('-')) {
            return Err(bad());
        }
        let offset_pct: f64 = number.parse().map_err(|_| bad())?;
        if !offset_pct.is_finite() || offset_pct <= -100.0 {
            return Err(format!("strike offset must be above -100 %, got {text:?}"));
        }
        Ok(Self::Atm { offset_pct })
    }
}

impl fmt::Display for StrikeSpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Absolute(k) => write!(f, "{k}"),
            Self::Atm { offset_pct } if offset_pct.abs() < 1e-12 => f.write_str("ATM"),
            Self::Atm { offset_pct } => write!(f, "ATM{offset_pct:+}%"),
        }
    }
}

impl TryFrom<String> for StrikeSpec {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<StrikeSpec> for String {
    fn from(spec: StrikeSpec) -> Self {
        spec.to_string()
    }
}

/// An expiry as written in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[serde(try_from = "String", into = "String")]
#[ts(type = "string")]
pub enum ExpirySpec {
    /// A fixed date, `YYYY-MM-DD`.
    Date(NaiveDate),
    /// `+0d`: expiry on the start date itself.
    Today,
    /// `+Nd`: the first Friday on or after the start date plus N days (N at least 1).
    FirstFridayAfter(u32),
}

impl FromStr for ExpirySpec {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let text = s.trim();
        if let Some(digits) = text.strip_prefix('+') {
            let bad = || format!("expected +Nd with N a whole number of days, got {text:?}");
            let digits = digits.strip_suffix(['d', 'D']).ok_or_else(bad)?;
            let days: u32 = digits.parse().map_err(|_| bad())?;
            if days > MAX_RELATIVE_DAYS {
                return Err(format!(
                    "at most +{MAX_RELATIVE_DAYS}d is allowed, got {text:?}"
                ));
            }
            return Ok(if days == 0 {
                Self::Today
            } else {
                Self::FirstFridayAfter(days)
            });
        }
        NaiveDate::parse_from_str(text, "%Y-%m-%d")
            .map(Self::Date)
            .map_err(|_| format!("expected YYYY-MM-DD, +0d or +Nd, got {text:?}"))
    }
}

impl fmt::Display for ExpirySpec {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Date(d) => write!(f, "{d}"),
            Self::Today => f.write_str("+0d"),
            Self::FirstFridayAfter(n) => write!(f, "+{n}d"),
        }
    }
}

impl TryFrom<String> for ExpirySpec {
    type Error = String;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        value.parse()
    }
}

impl From<ExpirySpec> for String {
    fn from(spec: ExpirySpec) -> Self {
        spec.to_string()
    }
}

/// One portfolio row before it is resolved against a market.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export)]
pub struct PositionSpec {
    /// Unique id.
    pub id: PositionId,
    /// The underlying equity. For an equity row, the shares' ticker.
    pub underlying: Symbol,
    /// Equity, call or put.
    pub kind: InstrumentKind,
    /// The strike. `None` for equity, required for options.
    pub strike: Option<StrikeSpec>,
    /// The expiry. `None` for equity, required for options.
    pub expiry: Option<ExpirySpec>,
    /// Signed quantity: shares or contracts.
    pub quantity: f64,
    /// Entry price per unit. `None` means the theoretical value at load time.
    pub entry_price: Option<f64>,
    /// Volatility override as a decimal. `None` means use the underlying's.
    pub iv: Option<f64>,
}

/// Loads and parses a portfolio CSV file.
///
/// # Errors
///
/// [`PortfolioError::Io`] when the file cannot be opened, otherwise as [`parse_csv`].
pub fn load_csv(path: &Path) -> Result<Vec<PositionSpec>, PortfolioError> {
    let file = File::open(path).map_err(|e| PortfolioError::Io {
        path: path.display().to_string(),
        message: e.to_string(),
    })?;
    parse_csv(file)
}

/// Builds a spec that round-trips a resolved position back to CSV exactly: strike as
/// [`StrikeSpec::Absolute`], expiry as [`ExpirySpec::Date`], entry price and any IV
/// override carried through explicitly rather than left blank, so a saved and reloaded
/// version reproduces the same entry price and P&L basis.
pub fn to_spec(position: &Position) -> PositionSpec {
    let (strike, expiry) = match &position.instrument {
        Instrument::Equity { .. } => (None, None),
        Instrument::Option(contract) => (
            Some(StrikeSpec::Absolute(contract.strike)),
            Some(ExpirySpec::Date(contract.expiry)),
        ),
    };
    PositionSpec {
        id: position.id.clone(),
        underlying: position.instrument.underlying().clone(),
        kind: position.instrument.kind(),
        strike,
        expiry,
        quantity: position.quantity,
        entry_price: Some(position.entry_price),
        iv: position.iv_override,
    }
}

/// Writes `specs` as portfolio CSV, in [`COLUMNS`] order, using each field's [`Display`]
/// impl. These are exactly the types [`parse_csv`] parses back, so this round-trips.
///
/// [`Display`]: std::fmt::Display
///
/// # Errors
///
/// [`PortfolioError::Csv`] when the underlying writer fails.
pub fn write_csv<W: Write>(writer: W, specs: &[PositionSpec]) -> Result<(), PortfolioError> {
    let mut wtr = WriterBuilder::new().from_writer(writer);
    wtr.write_record(COLUMNS).map_err(|e| csv_write_error(&e))?;
    for spec in specs {
        wtr.write_record([
            spec.id.as_str(),
            spec.underlying.as_str(),
            kind_str(spec.kind),
            &spec.strike.map(|s| s.to_string()).unwrap_or_default(),
            &spec.expiry.map(|e| e.to_string()).unwrap_or_default(),
            &spec.quantity.to_string(),
            &spec.entry_price.map(|p| p.to_string()).unwrap_or_default(),
            &spec.iv.map(|v| v.to_string()).unwrap_or_default(),
        ])
        .map_err(|e| csv_write_error(&e))?;
    }
    wtr.flush().map_err(|e| PortfolioError::Io {
        path: "<csv writer>".to_owned(),
        message: e.to_string(),
    })
}

/// [`write_csv`] into an in-memory buffer, for callers that need bytes to write to
/// storage rather than a stream to write to directly.
///
/// # Errors
///
/// As [`write_csv`].
pub fn to_csv_bytes(specs: &[PositionSpec]) -> Result<Vec<u8>, PortfolioError> {
    let mut buf = Vec::new();
    write_csv(&mut buf, specs)?;
    Ok(buf)
}

fn kind_str(kind: InstrumentKind) -> &'static str {
    match kind {
        InstrumentKind::Equity => "equity",
        InstrumentKind::Call => "call",
        InstrumentKind::Put => "put",
    }
}

fn csv_write_error(e: &csv::Error) -> PortfolioError {
    PortfolioError::Csv {
        line: 0,
        column: "(row)".to_owned(),
        message: e.to_string(),
    }
}

/// Parses portfolio CSV from any reader.
///
/// The first non-comment line is the header. Columns may come in any order,
/// but all eight must be present. Whitespace around fields is trimmed and
/// lines starting with `#` are skipped. `strike` and `expiry` are required for
/// options and must be blank for equity. Blank `entry_price` and `iv` mean
/// `None`.
///
/// # Errors
///
/// [`PortfolioError::Csv`] for the first problem found, naming the line and the column.
pub fn parse_csv<R: Read>(reader: R) -> Result<Vec<PositionSpec>, PortfolioError> {
    let mut rdr = ReaderBuilder::new()
        .has_headers(true)
        .trim(Trim::All)
        .comment(Some(b'#'))
        .from_reader(reader);

    let headers = rdr.headers().map_err(|e| row_error(&e))?.clone();
    let header_line = headers.position().map_or(1, csv::Position::line);
    let mut index = [0_usize; COLUMNS.len()];
    for (slot, name) in index.iter_mut().zip(COLUMNS) {
        *slot = headers
            .iter()
            .position(|h| h.eq_ignore_ascii_case(name))
            .ok_or_else(|| PortfolioError::Csv {
                line: header_line,
                column: name.to_owned(),
                message: "column is missing from the header".to_owned(),
            })?;
    }

    let mut specs = Vec::new();
    for record in rdr.records() {
        let record = record.map_err(|e| row_error(&e))?;
        specs.push(parse_row(&record, &index)?);
    }
    Ok(specs)
}

/// Converts a `csv` crate error into a [`PortfolioError::Csv`] for the whole row.
fn row_error(e: &csv::Error) -> PortfolioError {
    let line = e.position().map_or(0, csv::Position::line);
    let message = match e.kind() {
        csv::ErrorKind::UnequalLengths {
            expected_len, len, ..
        } => format!("expected {expected_len} fields, found {len}"),
        _ => e.to_string(),
    };
    PortfolioError::Csv {
        line,
        column: "(row)".to_owned(),
        message,
    }
}

fn parse_row(record: &StringRecord, index: &[usize; 8]) -> Result<PositionSpec, PortfolioError> {
    let line = record.position().map_or(0, csv::Position::line);
    let err = |column: &str, message: String| PortfolioError::Csv {
        line,
        column: column.to_owned(),
        message,
    };
    let cell = |i: usize| record.get(index[i]).unwrap_or("");

    let id = PositionId::new(cell(0)).map_err(|e| err("id", e.to_string()))?;
    let underlying = Symbol::new(cell(1)).map_err(|e| err("underlying", e.to_string()))?;
    let kind = match cell(2).to_ascii_lowercase().as_str() {
        "equity" => InstrumentKind::Equity,
        "call" => InstrumentKind::Call,
        "put" => InstrumentKind::Put,
        other => {
            return Err(err(
                "kind",
                format!("expected equity, call or put, got {other:?}"),
            ));
        }
    };

    let strike_text = cell(3);
    let strike = optional(strike_text, str::parse::<StrikeSpec>).map_err(|m| err("strike", m))?;
    let expiry_text = cell(4);
    let expiry = optional(expiry_text, str::parse::<ExpirySpec>).map_err(|m| err("expiry", m))?;
    if kind == InstrumentKind::Equity {
        if strike.is_some() {
            return Err(err("strike", "must be blank for equity".to_owned()));
        }
        if expiry.is_some() {
            return Err(err("expiry", "must be blank for equity".to_owned()));
        }
    } else {
        if strike.is_none() {
            return Err(err("strike", "required for an option".to_owned()));
        }
        if expiry.is_none() {
            return Err(err("expiry", "required for an option".to_owned()));
        }
    }

    let quantity = parse_number(cell(5)).map_err(|m| err("quantity", m))?;
    let quantity = quantity.ok_or_else(|| err("quantity", "is required".to_owned()))?;
    let entry_price = parse_number(cell(6)).map_err(|m| err("entry_price", m))?;
    if entry_price.is_some_and(|p| p < 0.0) {
        return Err(err("entry_price", "must not be negative".to_owned()));
    }
    let iv = parse_number(cell(7)).map_err(|m| err("iv", m))?;
    if iv.is_some_and(|v| v <= 0.0) {
        return Err(err("iv", "must be positive".to_owned()));
    }

    Ok(PositionSpec {
        id,
        underlying,
        kind,
        strike,
        expiry,
        quantity,
        entry_price,
        iv,
    })
}

/// Blank gives `None`, otherwise the parser's result.
fn optional<T>(text: &str, parse: impl Fn(&str) -> Result<T, String>) -> Result<Option<T>, String> {
    if text.is_empty() {
        Ok(None)
    } else {
        parse(text).map(Some)
    }
}

/// Blank gives `None`. Otherwise a finite number.
fn parse_number(text: &str) -> Result<Option<f64>, String> {
    optional(text, |t| match t.parse::<f64>() {
        Ok(v) if v.is_finite() => Ok(v),
        _ => Err(format!("expected a finite number, got {t:?}")),
    })
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;

    use super::*;

    const HEADER: &str = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n";

    fn parse(body: &str) -> Result<Vec<PositionSpec>, PortfolioError> {
        parse_csv(format!("{HEADER}{body}").as_bytes())
    }

    fn csv_error(body: &str) -> (u64, String, String) {
        match parse(body) {
            Err(PortfolioError::Csv {
                line,
                column,
                message,
            }) => (line, column, message),
            other => (
                0,
                String::new(),
                format!("expected a CSV error, got {other:?}"),
            ),
        }
    }

    #[test]
    fn strike_forms() {
        assert_eq!("230".parse(), Ok(StrikeSpec::Absolute(230.0)));
        assert_eq!("232.5".parse(), Ok(StrikeSpec::Absolute(232.5)));
        assert_eq!("ATM".parse(), Ok(StrikeSpec::Atm { offset_pct: 0.0 }));
        assert_eq!("atm".parse(), Ok(StrikeSpec::Atm { offset_pct: 0.0 }));
        assert_eq!("ATM+5%".parse(), Ok(StrikeSpec::Atm { offset_pct: 5.0 }));
        assert_eq!("ATM-10%".parse(), Ok(StrikeSpec::Atm { offset_pct: -10.0 }));
        assert_eq!("ATM+2.5%".parse(), Ok(StrikeSpec::Atm { offset_pct: 2.5 }));
    }

    #[test]
    fn bad_strikes() {
        for bad in [
            "", "abc", "0", "-5", "ATM5%", "ATM+5", "ATM+x%", "ATM-100%", "ATM+%", "NaN", "inf",
        ] {
            assert!(bad.parse::<StrikeSpec>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn expiry_forms() {
        let d = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        assert_eq!("2026-10-16".parse(), Ok(ExpirySpec::Date(d)));
        assert_eq!("+0d".parse(), Ok(ExpirySpec::Today));
        assert_eq!("+14d".parse(), Ok(ExpirySpec::FirstFridayAfter(14)));
        assert_eq!("+30D".parse(), Ok(ExpirySpec::FirstFridayAfter(30)));
    }

    #[test]
    fn bad_expiries() {
        for bad in [
            "",
            "tomorrow",
            "2026-13-01",
            "+d",
            "+5",
            "+-3d",
            "+3.5d",
            "+3651d",
            "16/10/2026",
        ] {
            assert!(bad.parse::<ExpirySpec>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn specs_display_and_round_trip() {
        for text in ["230", "232.5", "ATM", "ATM+5%", "ATM-10%", "ATM+2.5%"] {
            let spec: StrikeSpec = text.parse().unwrap();
            assert_eq!(spec.to_string(), text);
        }
        for text in ["2026-10-16", "+0d", "+14d"] {
            let spec: ExpirySpec = text.parse().unwrap();
            assert_eq!(spec.to_string(), text);
        }
    }

    #[test]
    fn parses_every_column_form() {
        let specs = parse(
            "spy-stock,spy,equity,,,500,,\n\
             put-hedge,SPY,put,ATM-5%,+30d,5,,\n\
             fixed,AAPL,call,230,2026-10-16,-3,4.25,0.3\n\
             today,SPY,CALL,ATM,+0d,1,,\n",
        )
        .unwrap();
        assert_eq!(specs.len(), 4);

        let stock = &specs[0];
        assert_eq!(stock.id.as_str(), "spy-stock");
        assert_eq!(stock.underlying.as_str(), "SPY");
        assert_eq!(stock.kind, InstrumentKind::Equity);
        assert_eq!((stock.strike, stock.expiry), (None, None));
        assert_relative_eq!(stock.quantity, 500.0);
        assert_eq!((stock.entry_price, stock.iv), (None, None));

        let hedge = &specs[1];
        assert_eq!(hedge.kind, InstrumentKind::Put);
        assert_eq!(hedge.strike, Some(StrikeSpec::Atm { offset_pct: -5.0 }));
        assert_eq!(hedge.expiry, Some(ExpirySpec::FirstFridayAfter(30)));

        let fixed = &specs[2];
        assert_eq!(fixed.strike, Some(StrikeSpec::Absolute(230.0)));
        assert_eq!(
            fixed.expiry,
            Some(ExpirySpec::Date(
                NaiveDate::from_ymd_opt(2026, 10, 16).unwrap()
            ))
        );
        assert_relative_eq!(fixed.quantity, -3.0);
        assert_eq!(fixed.entry_price, Some(4.25));
        assert_eq!(fixed.iv, Some(0.3));

        assert_eq!(specs[3].kind, InstrumentKind::Call);
        assert_eq!(specs[3].expiry, Some(ExpirySpec::Today));
    }

    #[test]
    fn comments_whitespace_and_column_order() {
        let text = "# a comment\n\
                    quantity , id, underlying, kind, strike, expiry, iv, entry_price\n\
                    # another\n\
                    10 , x , aapl , call , ATM , +7d , 0.4 , 2.5\n";
        let specs = parse_csv(text.as_bytes()).unwrap();
        assert_eq!(specs.len(), 1);
        assert_eq!(specs[0].id.as_str(), "x");
        assert_relative_eq!(specs[0].quantity, 10.0);
        assert_eq!(specs[0].iv, Some(0.4));
        assert_eq!(specs[0].entry_price, Some(2.5));
    }

    #[test]
    fn error_names_line_and_column() {
        // Line 1 is the header, so the third data row is line 4.
        let body = "a,SPY,equity,,,1,,\nb,SPY,equity,,,1,,\nc,SPY,bond,,,1,,\n";
        let (line, column, message) = csv_error(body);
        assert_eq!((line, column.as_str()), (4, "kind"));
        assert!(message.contains("bond"), "{message}");
    }

    #[test]
    fn line_numbers_count_comment_lines() {
        let text = format!("# c1\n{HEADER}# c2\nok,SPY,equity,,,1,,\nbad,SPY,call,ATM,soon,1,,\n");
        let got = match parse_csv(text.as_bytes()) {
            Err(PortfolioError::Csv { line, column, .. }) => (line, column),
            other => (0, format!("unexpected {other:?}")),
        };
        // Lines: comment 1, header 2, comment 3, good row 4, bad row 5.
        assert_eq!((got.0, got.1.as_str()), (5, "expiry"));
    }

    #[test]
    fn bad_rows_report_the_right_column() {
        let cases = [
            ("a,SPY,equity,,,abc,,", "quantity"),
            ("a,SPY,equity,,,,,", "quantity"),
            ("a,SPY,equity,,,NaN,,", "quantity"),
            ("a,SPY,put,ATM,+1d,1,-2,", "entry_price"),
            ("a,SPY,put,ATM,+1d,1,x,", "entry_price"),
            ("a,SPY,put,ATM,+1d,1,,0", "iv"),
            ("a,SPY,put,ATM,+1d,1,,fast", "iv"),
            ("a,SPY,put,junk,+1d,1,,", "strike"),
            ("a,SPY,put,,+1d,1,,", "strike"),
            ("a,SPY,put,ATM,,1,,", "expiry"),
            ("a,SPY,equity,100,,1,,", "strike"),
            ("a,SPY,equity,,+1d,1,,", "expiry"),
            (",SPY,equity,,,1,,", "id"),
            ("a,TOO LONG SYMBOL,equity,,,1,,", "underlying"),
        ];
        for (row, expected) in cases {
            let (line, column, _) = csv_error(row);
            assert_eq!((line, column.as_str()), (2, expected), "{row}");
        }
    }

    #[test]
    fn wrong_field_count_is_a_row_error() {
        let (line, column, message) = csv_error("a,SPY,equity\n");
        assert_eq!((line, column.as_str()), (2, "(row)"));
        assert!(message.contains("expected 8 fields, found 3"), "{message}");
    }

    #[test]
    fn missing_header_column_is_named() {
        let text = "id,underlying,kind,strike,expiry,quantity,entry_price\n";
        let got = match parse_csv(text.as_bytes()) {
            Err(PortfolioError::Csv { line, column, .. }) => (line, column),
            other => (0, format!("unexpected {other:?}")),
        };
        assert_eq!((got.0, got.1.as_str()), (1, "iv"));
    }

    #[test]
    fn display_includes_line_and_column() {
        let (line, column, message) = csv_error("a,SPY,equity,,,x,,\n");
        let e = PortfolioError::Csv {
            line,
            column,
            message,
        };
        assert!(e.to_string().starts_with("line 2, column quantity:"), "{e}");
    }

    #[test]
    fn empty_file_gives_no_specs() {
        assert_eq!(parse("").unwrap(), Vec::new());
    }

    #[test]
    fn to_spec_uses_absolute_strike_and_date_expiry() {
        let expiry = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let call = domain::OptionContract::new(
            Symbol::new("AAPL").unwrap(),
            domain::OptionType::Call,
            230.0,
            expiry,
        )
        .unwrap();
        let position = Position::new(
            PositionId::new("call-1").unwrap(),
            Instrument::Option(call),
            -3.0,
            4.25,
            Some(0.3),
        )
        .unwrap();
        let spec = to_spec(&position);
        assert_eq!(spec.id.as_str(), "call-1");
        assert_eq!(spec.strike, Some(StrikeSpec::Absolute(230.0)));
        assert_eq!(spec.expiry, Some(ExpirySpec::Date(expiry)));
        assert_relative_eq!(spec.quantity, -3.0);
        assert_eq!(spec.entry_price, Some(4.25));
        assert_eq!(spec.iv, Some(0.3));
    }

    #[test]
    fn write_csv_round_trips_through_parse_csv() {
        let expiry = NaiveDate::from_ymd_opt(2026, 10, 16).unwrap();
        let call = domain::OptionContract::new(
            Symbol::new("AAPL").unwrap(),
            domain::OptionType::Call,
            230.0,
            expiry,
        )
        .unwrap();
        let put = domain::OptionContract::new(
            Symbol::new("SPY").unwrap(),
            domain::OptionType::Put,
            600.0,
            expiry,
        )
        .unwrap();
        let positions = [
            Position::new(
                PositionId::new("stock").unwrap(),
                Instrument::Equity {
                    symbol: Symbol::new("SPY").unwrap(),
                },
                500.0,
                771.35,
                None,
            )
            .unwrap(),
            Position::new(
                PositionId::new("call").unwrap(),
                Instrument::Option(call),
                -3.0,
                4.25,
                Some(0.3),
            )
            .unwrap(),
            Position::new(
                PositionId::new("put").unwrap(),
                Instrument::Option(put),
                5.0,
                12.0,
                None,
            )
            .unwrap(),
        ];
        let specs: Vec<PositionSpec> = positions.iter().map(to_spec).collect();
        let bytes = to_csv_bytes(&specs).unwrap();
        let round_tripped = parse_csv(bytes.as_slice()).unwrap();
        assert_eq!(round_tripped, specs);
    }

    #[test]
    fn load_csv_reads_a_file_and_reports_a_missing_one() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("p.csv");
        std::fs::write(&path, format!("{HEADER}a,SPY,equity,,,1,,\n")).unwrap();
        assert_eq!(load_csv(&path).unwrap().len(), 1);
        assert!(matches!(
            load_csv(&dir.path().join("nope.csv")),
            Err(PortfolioError::Io { .. })
        ));
    }
}
