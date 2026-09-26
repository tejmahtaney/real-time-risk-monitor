//! Runs the simulator headless for N sessions and prints one table per day.
//!
//! Run from the repo root: `cargo run -p marketdata --example simulate -- 3`.
//! The argument is the number of sessions (default 3). It never calls the API: history
//! comes from the local archive, else the fallback files. Per symbol and day it prints the
//! open, close, high, low, the change and the vol range.

use std::collections::BTreeMap;
use std::path::Path;

use chrono::NaiveDate;
use domain::{Clock, SimClock, StepKind, Symbol, market_date};
use marketdata::{
    AlphaVantageSource, Archive, DataMode, GbmSimulator, HistoryLoader, MarketConfig, MarketTick,
    PriceFeed, SimulationConfig, SystemClock, calibrate,
};
use storage::StorageRoot;

/// Open, high, low, latest spot and the vol range for one symbol on one day.
struct Range {
    open: f64,
    high: f64,
    low: f64,
    last: f64,
    vol_min: f64,
    vol_max: f64,
}

/// One session's ranges for every symbol.
struct Day {
    date: NaiveDate,
    ranges: BTreeMap<Symbol, Range>,
}

impl Day {
    fn start(date: NaiveDate, tick: &MarketTick) -> Self {
        let ranges = tick
            .quotes
            .iter()
            .map(|(symbol, q)| {
                let range = Range {
                    open: q.spot,
                    high: q.spot,
                    low: q.spot,
                    last: q.spot,
                    vol_min: q.vol,
                    vol_max: q.vol,
                };
                (symbol.clone(), range)
            })
            .collect();
        Self { date, ranges }
    }

    fn update(&mut self, tick: &MarketTick) {
        for (symbol, q) in &tick.quotes {
            if let Some(r) = self.ranges.get_mut(symbol) {
                r.high = r.high.max(q.spot);
                r.low = r.low.min(q.spot);
                r.last = q.spot;
                r.vol_min = r.vol_min.min(q.vol);
                r.vol_max = r.vol_max.max(q.vol);
            }
        }
    }

    fn print(&self) {
        println!("\n{}", self.date);
        println!("symbol      open     close      high       low   change   vol range");
        for (symbol, r) in &self.ranges {
            println!(
                "{:<6} {:>9.2} {:>9.2} {:>9.2} {:>9.2} {:>+7.2}%   {:.1}% - {:.1}%",
                symbol.as_str(),
                r.open,
                r.last,
                r.high,
                r.low,
                (r.last / r.open - 1.0) * 100.0,
                r.vol_min * 100.0,
                r.vol_max * 100.0
            );
        }
    }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let sessions: usize = match std::env::args().nth(1) {
        Some(arg) => arg.parse()?,
        None => 3,
    };
    let market = MarketConfig::load(Path::new("config/market.toml"))?;
    let sim_config = SimulationConfig::load(Path::new("config/simulation.toml"))?;
    let loader = HistoryLoader::<AlphaVantageSource>::new(
        Archive::new(StorageRoot::resolve()?),
        "data/fallback",
        market.cache_max_age_hours,
        None,
        Box::new(SystemClock),
    );
    let histories = loader
        .load_history(&market.symbols, DataMode::Offline)
        .await?;
    let seed = calibrate(&histories, &market)?;

    let start = sim_config.start_time(&seed);
    let mut clock = SimClock::new(start, sim_config.speed)?;
    let mut feed = GbmSimulator::new(&seed, &sim_config)?;
    println!(
        "seed {}, speed {}x, tick {} ms, seed data as of {}, first session opens {}",
        sim_config.seed, sim_config.speed, sim_config.tick_interval_ms, seed.as_of, start
    );

    let mut day = Day::start(
        market_date(clock.now()),
        &feed.current_tick(clock.now(), StepKind::Overnight),
    );
    let mut finished = 0;
    while finished < sessions {
        let step = clock.advance(sim_config.tick_interval());
        let tick = feed.step(&step)?;
        match step.kind {
            StepKind::Overnight => day = Day::start(step.session_date, &tick),
            StepKind::Intraday => day.update(&tick),
            StepKind::Close => {
                day.update(&tick);
                day.print();
                finished += 1;
            }
        }
    }
    Ok(())
}
