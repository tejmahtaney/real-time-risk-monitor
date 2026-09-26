//! The position book: positions by id in insertion order, valuation and expiry settlement.

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use domain::{Instrument, MarketState, Position, PositionId, Symbol};

use crate::error::PortfolioError;
use crate::risk::{PositionRisk, Settlement, SettlementEvent, intrinsic, value_position};

#[derive(Debug, Clone)]
struct BookEntry {
    position: Position,
    settlement: Option<Settlement>,
}

/// Positions keyed by id, in insertion order.
///
/// Lookups scan linearly. A book holds dozens of positions, so that is
/// simpler than a second index and fast enough. Expired positions stay in the
/// book for display, with their settlement recorded.
#[derive(Debug, Clone, Default)]
pub struct PositionBook {
    entries: Vec<BookEntry>,
}

impl PositionBook {
    /// An empty book.
    pub fn new() -> Self {
        Self::default()
    }

    /// Builds a book from positions, in order.
    ///
    /// # Errors
    ///
    /// [`PortfolioError::DuplicateId`] at the first repeated id.
    pub fn from_positions(
        positions: impl IntoIterator<Item = Position>,
    ) -> Result<Self, PortfolioError> {
        let mut book = Self::new();
        for position in positions {
            book.add(position)?;
        }
        Ok(book)
    }

    /// Adds a position at the end.
    ///
    /// # Errors
    ///
    /// [`PortfolioError::DuplicateId`] when the id is already in the book.
    pub fn add(&mut self, position: Position) -> Result<(), PortfolioError> {
        if self.get(&position.id).is_some() {
            return Err(PortfolioError::DuplicateId(position.id));
        }
        self.entries.push(BookEntry {
            position,
            settlement: None,
        });
        Ok(())
    }

    /// Removes a position and returns it. Order of the others is kept.
    ///
    /// # Errors
    ///
    /// [`PortfolioError::UnknownPosition`] when the id is not in the book.
    pub fn remove(&mut self, id: &PositionId) -> Result<Position, PortfolioError> {
        let index = self
            .entries
            .iter()
            .position(|e| &e.position.id == id)
            .ok_or_else(|| PortfolioError::UnknownPosition(id.clone()))?;
        Ok(self.entries.remove(index).position)
    }

    /// The position with this id, if any.
    pub fn get(&self, id: &PositionId) -> Option<&Position> {
        self.entries
            .iter()
            .find(|e| &e.position.id == id)
            .map(|e| &e.position)
    }

    /// The settlement of a position, if it has expired and been settled.
    pub fn settlement(&self, id: &PositionId) -> Option<&Settlement> {
        self.entries
            .iter()
            .find(|e| &e.position.id == id)
            .and_then(|e| e.settlement.as_ref())
    }

    /// All positions in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &Position> {
        self.entries.iter().map(|e| &e.position)
    }

    /// The distinct underlyings held, in symbol order.
    pub fn underlyings(&self) -> BTreeSet<Symbol> {
        self.iter()
            .map(|p| p.instrument.underlying().clone())
            .collect()
    }

    /// The number of positions, including expired ones.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the book has no positions.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Values every position, in insertion order.
    ///
    /// One bad position, such as one whose underlying is missing from the
    /// market, gets a [`crate::PositionStatus::Error`] row and does not stop
    /// the rest. An option that is past its expiry but not yet settled is also
    /// an error, so stale Greeks cannot reach the risk numbers: call
    /// [`PositionBook::settle_expired`] first. `now` sets the time to expiry.
    pub fn value_all(&self, market: &MarketState, now: DateTime<Utc>) -> Vec<PositionRisk> {
        self.entries
            .iter()
            .map(|e| value_position(&e.position, e.settlement.as_ref(), market, now))
            .collect()
    }

    /// Settles every option at or past its expiry time and returns what happened.
    ///
    /// An option settles in cash at its intrinsic value, max(ω × (S − K), 0),
    /// against the spot in `market` at the moment of the call. Realised P&L is
    /// (intrinsic − entry) × quantity × multiplier. The position stays in the
    /// book with its settlement recorded, reports status `Expired` and zero
    /// Greeks, and is skipped by risk. Calling again does nothing for
    /// positions that are already settled. An option whose underlying has no
    /// spot in the market is left unsettled and logged.
    ///
    /// Cash settlement is a simplification. Real equity options settle into
    /// shares: exercising a call buys 100 shares per contract at the strike,
    /// and the seller is assigned and delivers them. Here that is collapsed
    /// into its cash value at the spot in `market`, which is the price when
    /// the engine ticks, not exactly the 16:00 close.
    pub fn settle_expired(
        &mut self,
        now: DateTime<Utc>,
        market: &MarketState,
    ) -> Vec<SettlementEvent> {
        let mut events = Vec::new();
        for entry in &mut self.entries {
            if entry.settlement.is_some() {
                continue;
            }
            let position = &entry.position;
            let Instrument::Option(contract) = &position.instrument else {
                continue;
            };
            let expiry = contract.expiry_datetime();
            if now < expiry {
                continue;
            }
            let Some(state) = market.underlying(&contract.underlying) else {
                tracing::warn!(
                    id = %position.id,
                    underlying = %contract.underlying,
                    "cannot settle an expired option: no market data for the underlying"
                );
                continue;
            };
            let value = intrinsic(contract.option_type, contract.strike, state.spot());
            let realised_pnl =
                (value - position.entry_price) * position.quantity * contract.multiplier;
            if !realised_pnl.is_finite() {
                tracing::warn!(id = %position.id, "cannot settle an expired option: P&L is not finite");
                continue;
            }
            let settlement = Settlement {
                settled_at: now,
                spot: state.spot(),
                intrinsic: value,
                realised_pnl,
            };
            tracing::info!(
                id = %position.id,
                spot = state.spot(),
                intrinsic = value,
                realised_pnl,
                "option expired and settled in cash"
            );
            events.push(SettlementEvent {
                position_id: position.id.clone(),
                description: position.instrument.description(),
                underlying: contract.underlying.clone(),
                kind: position.instrument.kind(),
                strike: contract.strike,
                quantity: position.quantity,
                multiplier: contract.multiplier,
                entry_price: position.entry_price,
                expiry,
                settled_at: now,
                spot: state.spot(),
                intrinsic: value,
                realised_pnl,
            });
            entry.settlement = Some(settlement);
        }
        events
    }
}

#[cfg(test)]
mod tests {
    use approx::assert_relative_eq;
    use chrono::{NaiveDate, TimeZone};
    use domain::{InstrumentKind, OptionContract, OptionType, UnderlyingState, year_fraction};
    use pricing::BsInputs;

    use super::*;
    use crate::risk::{PositionStatus, ReportingGreeks};

    fn utc(y: i32, m: u32, d: u32, h: u32, min: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, m, d, h, min, 0).unwrap()
    }

    fn now() -> DateTime<Utc> {
        utc(2026, 10, 1, 14, 0)
    }

    /// 16:00 New York on Friday 16 October 2026 is 20:00 UTC.
    fn expiry_time() -> DateTime<Utc> {
        utc(2026, 10, 16, 20, 0)
    }

    fn sym(s: &str) -> Symbol {
        Symbol::new(s).unwrap()
    }

    fn market_at(spot: f64) -> MarketState {
        MarketState::new(
            now(),
            0.04,
            [
                (sym("SPY"), UnderlyingState::new(spot, 0.20, 0.01).unwrap()),
                (
                    sym("AAPL"),
                    UnderlyingState::new(230.0, 0.25, 0.005).unwrap(),
                ),
            ],
        )
        .unwrap()
    }

    fn market() -> MarketState {
        market_at(600.0)
    }

    fn stock(id: &str, symbol: &str, quantity: f64, entry: f64) -> Position {
        Position::new(
            PositionId::new(id).unwrap(),
            Instrument::Equity {
                symbol: sym(symbol),
            },
            quantity,
            entry,
            None,
        )
        .unwrap()
    }

    fn option(
        id: &str,
        symbol: &str,
        kind: OptionType,
        strike: f64,
        quantity: f64,
        entry: f64,
    ) -> Position {
        let contract = OptionContract::new(
            sym(symbol),
            kind,
            strike,
            NaiveDate::from_ymd_opt(2026, 10, 16).unwrap(),
        )
        .unwrap();
        Position::new(
            PositionId::new(id).unwrap(),
            Instrument::Option(contract),
            quantity,
            entry,
            None,
        )
        .unwrap()
    }

    fn one(position: Position, market: &MarketState, at: DateTime<Utc>) -> PositionRisk {
        let book = PositionBook::from_positions([position]).unwrap();
        book.value_all(market, at).remove(0)
    }

    /// Raw Greeks computed straight from `pricing`, as the reference.
    fn reference(kind: OptionType, strike: f64, vol: f64) -> pricing::Greeks {
        let t = year_fraction(now(), expiry_time());
        let inputs = BsInputs::new(600.0, strike, t, 0.04, 0.01, vol).unwrap();
        pricing::greeks(kind, &inputs)
    }

    // ---- the book ----

    #[test]
    fn rejects_duplicate_ids() {
        let mut book = PositionBook::new();
        book.add(stock("a", "SPY", 1.0, 1.0)).unwrap();
        let e = book.add(stock("a", "AAPL", 2.0, 1.0)).unwrap_err();
        assert!(matches!(e, PortfolioError::DuplicateId(id) if id.as_str() == "a"));
        assert_eq!(book.len(), 1);
        assert!(
            PositionBook::from_positions([
                stock("x", "SPY", 1.0, 1.0),
                stock("x", "SPY", 1.0, 1.0)
            ])
            .is_err()
        );
    }

    #[test]
    fn keeps_insertion_order_and_supports_get_and_remove() {
        let mut book = PositionBook::new();
        assert!(book.is_empty());
        for id in ["c", "a", "b"] {
            book.add(stock(id, "SPY", 1.0, 1.0)).unwrap();
        }
        let ids = |b: &PositionBook| b.iter().map(|p| p.id.to_string()).collect::<Vec<_>>();
        assert_eq!(ids(&book), ["c", "a", "b"]);

        let a = PositionId::new("a").unwrap();
        assert!(book.get(&a).is_some());
        let removed = book.remove(&a).unwrap();
        assert_eq!(removed.id, a);
        assert_eq!(ids(&book), ["c", "b"]);
        assert!(book.get(&a).is_none());
        assert!(matches!(
            book.remove(&a),
            Err(PortfolioError::UnknownPosition(_))
        ));
        // The id can be reused after removal.
        book.add(stock("a", "SPY", 1.0, 1.0)).unwrap();
        assert_eq!(ids(&book), ["c", "b", "a"]);
    }

    #[test]
    fn underlyings_are_distinct_and_sorted() {
        let book = PositionBook::from_positions([
            stock("1", "SPY", 1.0, 1.0),
            option("2", "AAPL", OptionType::Call, 230.0, 1.0, 1.0),
            stock("3", "SPY", 1.0, 1.0),
        ])
        .unwrap();
        let names: Vec<_> = book.underlyings().iter().map(Symbol::to_string).collect();
        assert_eq!(names, ["AAPL", "SPY"]);
    }

    // ---- valuation ----

    #[test]
    fn equity_risk() {
        let row = one(stock("s", "SPY", 500.0, 590.0), &market(), now());
        assert_eq!(row.status, PositionStatus::Active);
        assert_eq!(row.description, "SPY");
        assert_eq!(row.kind, InstrumentKind::Equity);
        assert_relative_eq!(row.multiplier, 1.0);
        assert_relative_eq!(row.spot, 600.0);
        assert_relative_eq!(row.mark, 600.0);
        assert_relative_eq!(row.market_value, 300_000.0);
        assert_relative_eq!(row.unrealised_pnl, 5_000.0);
        assert_relative_eq!(row.greeks.delta, 1.0);
        assert_relative_eq!(row.reporting.share_delta, 500.0);
        assert_relative_eq!(row.reporting.dollar_delta, 300_000.0);
        assert_relative_eq!(row.reporting.share_gamma, 0.0);
        assert_relative_eq!(row.reporting.dollar_gamma_1pct, 0.0);
        assert_relative_eq!(row.reporting.vega, 0.0);
        assert_relative_eq!(row.reporting.theta, 0.0);
        assert_relative_eq!(row.reporting.rho, 0.0);
    }

    #[test]
    fn short_equity_flips_the_sign() {
        let row = one(stock("s", "SPY", -100.0, 590.0), &market(), now());
        assert_relative_eq!(row.reporting.share_delta, -100.0);
        assert_relative_eq!(row.market_value, -60_000.0);
        assert_relative_eq!(row.unrealised_pnl, -1_000.0);
    }

    #[test]
    fn option_reporting_greeks_follow_the_formulas() {
        let position = option("c", "SPY", OptionType::Call, 600.0, 3.0, 20.0);
        let row = one(position, &market(), now());
        let raw = reference(OptionType::Call, 600.0, 0.20);
        let size = 3.0 * 100.0;
        assert_eq!(row.status, PositionStatus::Active);
        assert_eq!(row.description, "SPY 2026-10-16 C 600");
        assert_relative_eq!(row.vol, 0.20);
        assert_relative_eq!(row.time_to_expiry, year_fraction(now(), expiry_time()));
        assert_relative_eq!(row.mark, raw.price, max_relative = 1e-12);
        assert_relative_eq!(row.market_value, raw.price * size, max_relative = 1e-12);
        assert_relative_eq!(
            row.unrealised_pnl,
            (raw.price - 20.0) * size,
            max_relative = 1e-12
        );
        let g = row.reporting;
        assert_relative_eq!(g.share_delta, raw.delta * size, max_relative = 1e-12);
        assert_relative_eq!(
            g.dollar_delta,
            raw.delta * size * 600.0,
            max_relative = 1e-12
        );
        assert_relative_eq!(g.share_gamma, raw.gamma * size, max_relative = 1e-12);
        assert_relative_eq!(
            g.dollar_gamma_1pct,
            0.5 * raw.gamma * 6.0 * 6.0 * size,
            max_relative = 1e-12
        );
        assert_relative_eq!(g.vega, raw.vega / 100.0 * size, max_relative = 1e-12);
        assert_relative_eq!(g.theta, raw.theta / 365.0 * size, max_relative = 1e-12);
        assert_relative_eq!(g.rho, raw.rho / 100.0 * size, max_relative = 1e-12);
    }

    #[test]
    fn scaling_is_linear_in_quantity() {
        let m = market();
        let one_lot = one(
            option("a", "SPY", OptionType::Put, 570.0, 1.0, 5.0),
            &m,
            now(),
        );
        let four_lots = one(
            option("a", "SPY", OptionType::Put, 570.0, 4.0, 5.0),
            &m,
            now(),
        );
        let (a, b) = (one_lot.reporting, four_lots.reporting);
        assert_relative_eq!(b.share_delta, 4.0 * a.share_delta, max_relative = 1e-12);
        assert_relative_eq!(b.dollar_delta, 4.0 * a.dollar_delta, max_relative = 1e-12);
        assert_relative_eq!(b.share_gamma, 4.0 * a.share_gamma, max_relative = 1e-12);
        assert_relative_eq!(
            b.dollar_gamma_1pct,
            4.0 * a.dollar_gamma_1pct,
            max_relative = 1e-12
        );
        assert_relative_eq!(b.vega, 4.0 * a.vega, max_relative = 1e-12);
        assert_relative_eq!(b.theta, 4.0 * a.theta, max_relative = 1e-12);
        assert_relative_eq!(b.rho, 4.0 * a.rho, max_relative = 1e-12);
        assert_relative_eq!(
            four_lots.market_value,
            4.0 * one_lot.market_value,
            max_relative = 1e-12
        );
    }

    #[test]
    fn short_options_have_the_opposite_sign() {
        let m = market();
        let long = one(
            option("a", "SPY", OptionType::Call, 600.0, 2.0, 5.0),
            &m,
            now(),
        );
        let short = one(
            option("a", "SPY", OptionType::Call, 600.0, -2.0, 5.0),
            &m,
            now(),
        );
        assert!(long.reporting.share_delta > 0.0 && short.reporting.share_delta < 0.0);
        assert!(long.reporting.share_gamma > 0.0 && short.reporting.share_gamma < 0.0);
        assert!(long.reporting.vega > 0.0 && short.reporting.vega < 0.0);
        assert!(long.reporting.theta < 0.0 && short.reporting.theta > 0.0);
        assert_relative_eq!(
            short.reporting.share_delta,
            -long.reporting.share_delta,
            max_relative = 1e-12
        );
        assert_relative_eq!(short.market_value, -long.market_value, max_relative = 1e-12);
        // Same entry price, same mark: the P&L of the short is the opposite of the long.
        assert_relative_eq!(
            short.unrealised_pnl,
            -long.unrealised_pnl,
            max_relative = 1e-12
        );
    }

    #[test]
    fn multiplier_scales_the_position() {
        let plain = option("a", "SPY", OptionType::Call, 600.0, 1.0, 5.0);
        let mut adjusted = plain.clone();
        if let Instrument::Option(c) = &mut adjusted.instrument {
            c.multiplier = 150.0;
        }
        let m = market();
        let (a, b) = (one(plain, &m, now()), one(adjusted, &m, now()));
        assert_relative_eq!(b.multiplier, 150.0);
        assert_relative_eq!(
            b.reporting.share_delta,
            1.5 * a.reporting.share_delta,
            max_relative = 1e-12
        );
        assert_relative_eq!(b.market_value, 1.5 * a.market_value, max_relative = 1e-12);
        // The per-unit mark does not depend on the multiplier.
        assert_relative_eq!(b.mark, a.mark, max_relative = 1e-12);
    }

    #[test]
    fn iv_override_replaces_the_market_vol() {
        let mut position = option("a", "SPY", OptionType::Call, 600.0, 1.0, 5.0);
        position.iv_override = Some(0.50);
        let row = one(position, &market(), now());
        let raw = reference(OptionType::Call, 600.0, 0.50);
        assert_relative_eq!(row.vol, 0.50);
        assert_relative_eq!(row.mark, raw.price, max_relative = 1e-12);
        assert!(row.mark > reference(OptionType::Call, 600.0, 0.20).price);
    }

    #[test]
    fn covered_call_net_share_delta_is_70() {
        // 100 shares plus one short call of delta 0.3 is 100 − 30 = 70 shares.
        let raw_call = pricing::Greeks {
            price: 4.0,
            delta: 0.3,
            gamma: 0.02,
            vega: 30.0,
            theta: -12.0,
            rho: 8.0,
            vanna: 0.0,
            volga: 0.0,
        };
        let call = ReportingGreeks::scale(&raw_call, -1.0, 100.0, 230.0);
        let shares = ReportingGreeks::scale(
            &pricing::Greeks {
                price: 230.0,
                delta: 1.0,
                gamma: 0.0,
                vega: 0.0,
                theta: 0.0,
                rho: 0.0,
                vanna: 0.0,
                volga: 0.0,
            },
            100.0,
            1.0,
            230.0,
        );
        assert_relative_eq!(
            shares.share_delta + call.share_delta,
            70.0,
            max_relative = 1e-12
        );
    }

    #[test]
    fn covered_call_through_the_book() {
        let m = market();
        let book = PositionBook::from_positions([
            stock("s", "AAPL", 100.0, 230.0),
            option("c", "AAPL", OptionType::Call, 240.0, -1.0, 3.0),
        ])
        .unwrap();
        let rows = book.value_all(&m, now());
        let net: f64 = rows.iter().map(|r| r.reporting.share_delta).sum();
        let call_delta = rows[1].greeks.delta;
        assert!(call_delta > 0.0 && call_delta < 1.0);
        assert_relative_eq!(net, 100.0 - 100.0 * call_delta, max_relative = 1e-12);
        assert!(net > 0.0 && net < 100.0);
    }

    #[test]
    fn theoretical_entry_price_gives_zero_unrealised_pnl() {
        use crate::{parse_csv, resolve};

        let csv = "id,underlying,kind,strike,expiry,quantity,entry_price,iv\n\
                   s,SPY,equity,,,10,,\n\
                   c,SPY,call,ATM+5%,+30d,-4,,\n\
                   p,SPY,put,ATM-5%,+30d,4,,0.35\n";
        let specs = parse_csv(csv.as_bytes()).unwrap();
        let m = market();
        let book = PositionBook::from_positions(resolve(&specs, &m, now()).unwrap()).unwrap();
        for row in book.value_all(&m, now()) {
            assert_relative_eq!(row.unrealised_pnl, 0.0, epsilon = 1e-9);
        }
        // An hour later theta and the clock have moved the marks, so P&L is no longer exactly zero.
        let later = book.value_all(&m, now() + chrono::Duration::hours(6));
        assert!(later[1].unrealised_pnl.abs() > 1e-6);
    }

    #[test]
    fn a_missing_underlying_errors_only_that_position() {
        let book = PositionBook::from_positions([
            stock("spy", "SPY", 10.0, 600.0),
            stock("tsla", "TSLA", 10.0, 250.0),
            option("aapl-c", "AAPL", OptionType::Call, 230.0, 1.0, 5.0),
            option("tsla-c", "TSLA", OptionType::Call, 250.0, 1.0, 5.0),
        ])
        .unwrap();
        let rows = book.value_all(&market(), now());
        assert_eq!(rows.len(), 4);
        assert_eq!(rows[0].status, PositionStatus::Active);
        assert_eq!(rows[2].status, PositionStatus::Active);
        for i in [1, 3] {
            let PositionStatus::Error { reason } = &rows[i].status else {
                assert!(matches!(rows[i].status, PositionStatus::Error { .. }));
                continue;
            };
            assert!(reason.contains("TSLA"), "{reason}");
            assert_relative_eq!(rows[i].market_value, 0.0);
            assert_relative_eq!(rows[i].reporting.share_delta, 0.0);
            assert_relative_eq!(rows[i].greeks.delta, 0.0);
        }
        assert_relative_eq!(rows[0].reporting.share_delta, 10.0);
    }

    #[test]
    fn a_bad_contract_is_an_error_row_not_a_nan() {
        let mut bad = option("bad", "SPY", OptionType::Call, 600.0, 1.0, 5.0);
        if let Instrument::Option(c) = &mut bad.instrument {
            c.strike = -1.0;
        }
        let mut bad_multiplier = option("bm", "SPY", OptionType::Call, 600.0, 1.0, 5.0);
        if let Instrument::Option(c) = &mut bad_multiplier.instrument {
            c.multiplier = f64::NAN;
        }
        let mut bad_iv = option("iv", "SPY", OptionType::Call, 600.0, 1.0, 5.0);
        bad_iv.iv_override = Some(-0.2);
        let book = PositionBook::from_positions([
            bad,
            bad_multiplier,
            bad_iv,
            stock("ok", "SPY", 1.0, 1.0),
        ])
        .unwrap();
        let rows = book.value_all(&market(), now());
        for row in &rows[..3] {
            assert!(
                matches!(row.status, PositionStatus::Error { .. }),
                "{}",
                row.id
            );
            assert!(row.market_value.is_finite() && row.mark.is_finite());
            assert_relative_eq!(row.market_value, 0.0);
        }
        assert_eq!(rows[3].status, PositionStatus::Active);
    }

    #[test]
    fn an_unsettled_expired_option_is_an_error() {
        let book =
            PositionBook::from_positions([option("c", "SPY", OptionType::Call, 600.0, 1.0, 5.0)])
                .unwrap();
        let rows = book.value_all(&market(), expiry_time());
        let PositionStatus::Error { reason } = &rows[0].status else {
            assert!(matches!(rows[0].status, PositionStatus::Error { .. }));
            return;
        };
        assert!(reason.contains("not settled"), "{reason}");
        assert_relative_eq!(rows[0].reporting.share_delta, 0.0);
    }

    // ---- settlement ----

    /// Spot 600 at expiry. Strikes 550 (in the money for a call) and 650 (in the money for a put).
    fn settlement_book() -> PositionBook {
        PositionBook::from_positions([
            option("call-itm", "SPY", OptionType::Call, 550.0, 2.0, 40.0),
            option("call-otm", "SPY", OptionType::Call, 650.0, 2.0, 3.0),
            option("put-itm", "SPY", OptionType::Put, 650.0, 1.0, 60.0),
            option("put-otm", "SPY", OptionType::Put, 550.0, 1.0, 2.0),
            option("short-call-itm", "SPY", OptionType::Call, 550.0, -1.0, 40.0),
            stock("stock", "SPY", 100.0, 590.0),
        ])
        .unwrap()
    }

    #[test]
    fn settles_itm_and_otm_calls_and_puts_in_cash() {
        let mut book = settlement_book();
        let events = book.settle_expired(expiry_time(), &market());
        assert_eq!(events.len(), 5, "the stock does not expire");

        let by_id = |id: &str| {
            events
                .iter()
                .find(|e| e.position_id.as_str() == id)
                .unwrap()
        };
        // Call 550: intrinsic 50, (50 − 40) × 2 × 100 = 2,000.
        assert_relative_eq!(by_id("call-itm").intrinsic, 50.0);
        assert_relative_eq!(by_id("call-itm").realised_pnl, 2_000.0);
        // Call 650: worthless, (0 − 3) × 2 × 100 = −600.
        assert_relative_eq!(by_id("call-otm").intrinsic, 0.0);
        assert_relative_eq!(by_id("call-otm").realised_pnl, -600.0);
        // Put 650: intrinsic 50, (50 − 60) × 1 × 100 = −1,000.
        assert_relative_eq!(by_id("put-itm").intrinsic, 50.0);
        assert_relative_eq!(by_id("put-itm").realised_pnl, -1_000.0);
        // Put 550: worthless, (0 − 2) × 100 = −200.
        assert_relative_eq!(by_id("put-otm").intrinsic, 0.0);
        assert_relative_eq!(by_id("put-otm").realised_pnl, -200.0);
        // A short ITM call loses the intrinsic: (50 − 40) × −1 × 100 = −1,000.
        assert_relative_eq!(by_id("short-call-itm").realised_pnl, -1_000.0);

        let e = by_id("call-itm");
        assert_eq!(e.description, "SPY 2026-10-16 C 550");
        assert_eq!(e.expiry, expiry_time());
        assert_eq!(e.settled_at, expiry_time());
        assert_relative_eq!(e.spot, 600.0);
    }

    #[test]
    fn settled_positions_are_expired_with_zero_greeks() {
        let mut book = settlement_book();
        book.settle_expired(expiry_time(), &market());
        let rows = book.value_all(&market(), expiry_time());
        for row in &rows[..5] {
            assert_eq!(row.status, PositionStatus::Expired, "{}", row.id);
            assert_eq!(row.reporting, ReportingGreeks::default());
            assert_relative_eq!(row.greeks.delta, 0.0);
            assert_relative_eq!(row.greeks.gamma, 0.0);
            assert_relative_eq!(row.greeks.vega, 0.0);
            assert_relative_eq!(row.greeks.theta, 0.0);
            assert_relative_eq!(row.greeks.rho, 0.0);
            assert_relative_eq!(row.market_value, 0.0);
            assert_relative_eq!(row.unrealised_pnl, 0.0);
            assert_relative_eq!(row.time_to_expiry, 0.0);
        }
        assert_relative_eq!(rows[0].realised_pnl, 2_000.0);
        assert_relative_eq!(rows[0].mark, 50.0);
        // They stay in the book, in order, and the stock is untouched.
        assert_eq!(book.len(), 6);
        assert_eq!(rows[5].status, PositionStatus::Active);
        assert_relative_eq!(rows[5].reporting.share_delta, 100.0);
        assert!(
            book.settlement(&PositionId::new("call-itm").unwrap())
                .is_some()
        );
        assert!(
            book.settlement(&PositionId::new("stock").unwrap())
                .is_none()
        );
    }

    #[test]
    fn settlement_is_idempotent() {
        let mut book = settlement_book();
        assert_eq!(book.settle_expired(expiry_time(), &market()).len(), 5);
        // A later call at a very different spot changes nothing.
        let later = expiry_time() + chrono::Duration::hours(1);
        assert!(book.settle_expired(later, &market_at(300.0)).is_empty());
        let rows = book.value_all(&market_at(300.0), later);
        assert_relative_eq!(rows[0].realised_pnl, 2_000.0);
    }

    #[test]
    fn nothing_settles_before_expiry_and_it_settles_exactly_at_expiry() {
        let mut book = settlement_book();
        let before = expiry_time() - chrono::Duration::seconds(1);
        assert!(book.settle_expired(before, &market()).is_empty());
        assert!(
            book.value_all(&market(), before)
                .iter()
                .all(|r| r.status == PositionStatus::Active)
        );
        assert_eq!(book.settle_expired(expiry_time(), &market()).len(), 5);
    }

    #[test]
    fn settlement_uses_the_spot_in_the_market_it_is_given() {
        let mut book =
            PositionBook::from_positions([option("c", "SPY", OptionType::Call, 550.0, 1.0, 40.0)])
                .unwrap();
        let events = book.settle_expired(expiry_time(), &market_at(560.0));
        assert_relative_eq!(events[0].intrinsic, 10.0);
        assert_relative_eq!(events[0].realised_pnl, -3_000.0);
    }

    #[test]
    fn an_option_with_no_spot_stays_unsettled() {
        let mut book = PositionBook::from_positions([
            option("tsla-c", "TSLA", OptionType::Call, 250.0, 1.0, 5.0),
            option("spy-c", "SPY", OptionType::Call, 550.0, 1.0, 40.0),
        ])
        .unwrap();
        let events = book.settle_expired(expiry_time(), &market());
        assert_eq!(events.len(), 1);
        assert_eq!(events[0].position_id.as_str(), "spy-c");
        assert!(
            book.settlement(&PositionId::new("tsla-c").unwrap())
                .is_none()
        );
        let rows = book.value_all(&market(), expiry_time());
        assert!(matches!(rows[0].status, PositionStatus::Error { .. }));
        assert_eq!(rows[1].status, PositionStatus::Expired);
        // Once the data is there, the next call settles it.
        let mut with_tsla = market();
        with_tsla.insert(sym("TSLA"), UnderlyingState::new(260.0, 0.6, 0.0).unwrap());
        assert_eq!(book.settle_expired(expiry_time(), &with_tsla).len(), 1);
    }

    #[test]
    fn removing_a_settled_position_drops_its_settlement() {
        let mut book = settlement_book();
        book.settle_expired(expiry_time(), &market());
        let id = PositionId::new("call-itm").unwrap();
        book.remove(&id).unwrap();
        assert!(book.settlement(&id).is_none());
        assert_eq!(book.len(), 5);
    }
}
