//! Cost of one `implied_volatility()` solve, near the money and deep out of the money.
//!
//! Run with `cargo bench -p pricing`.

// A benchmark cannot report a failure any other way, and every input here is a
// constant known to be valid.
#![allow(clippy::unwrap_used)]

use criterion::{Criterion, criterion_group, criterion_main};
use pricing::{BsInputs, IvRequest, OptionType, implied_volatility, price};
use std::hint::black_box;

/// A request whose price comes from the given volatility.
fn request(strike: f64, time_to_expiry: f64, vol: f64) -> IvRequest {
    let inputs = BsInputs::new(100.0, strike, time_to_expiry, 0.05, 0.02, vol).unwrap();
    IvRequest {
        option_type: OptionType::Call,
        price: price(OptionType::Call, &inputs),
        spot: 100.0,
        strike,
        time_to_expiry,
        rate: 0.05,
        dividend_yield: 0.02,
    }
}

fn bench_implied_vol(c: &mut Criterion) {
    let near_the_money = request(100.0, 0.5, 0.25);
    c.bench_function("implied_vol/near_the_money", |b| {
        b.iter(|| implied_volatility(black_box(&near_the_money)));
    });

    let deep_otm = request(140.0, 0.1, 0.30);
    c.bench_function("implied_vol/deep_out_of_the_money", |b| {
        b.iter(|| implied_volatility(black_box(&deep_otm)));
    });
}

criterion_group!(benches, bench_implied_vol);
criterion_main!(benches);
