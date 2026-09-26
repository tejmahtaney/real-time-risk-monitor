//! Cost of `greeks()`, for one option and for a loop over 10,000 varied options.
//!
//! Run with `cargo bench -p pricing`.

// A benchmark cannot report a failure any other way, and every input here is a
// constant known to be valid.
#![allow(clippy::unwrap_used)]

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use pricing::{BsInputs, OptionType, greeks};
use std::hint::black_box;

const OPTIONS: usize = 10_000;

/// Deterministic spread of spot, strike, expiry, rate, yield and vol, no RNG needed.
fn options() -> Vec<(OptionType, BsInputs)> {
    (0..OPTIONS)
        .map(|n| {
            let x = n as f64 / OPTIONS as f64;
            let spot = 50.0 + 150.0 * x;
            let strike = spot * (0.7 + 0.6 * ((n * 7919) % 1000) as f64 / 1000.0);
            let t = 0.02 + 2.0 * ((n * 104_729) % 1000) as f64 / 1000.0;
            let rate = 0.10 * ((n * 31) % 100) as f64 / 100.0;
            let q = 0.05 * ((n * 17) % 100) as f64 / 100.0;
            let vol = 0.10 + 0.8 * ((n * 13) % 100) as f64 / 100.0;
            let ty = if n % 2 == 0 {
                OptionType::Call
            } else {
                OptionType::Put
            };
            (ty, BsInputs::new(spot, strike, t, rate, q, vol).unwrap())
        })
        .collect()
}

fn bench_greeks(c: &mut Criterion) {
    let single = BsInputs::new(100.0, 95.0, 0.75, 0.05, 0.02, 0.25).unwrap();
    c.bench_function("greeks/one_option", |b| {
        b.iter(|| greeks(black_box(OptionType::Call), black_box(&single)));
    });

    let book = options();
    let mut group = c.benchmark_group("greeks/book");
    group.throughput(Throughput::Elements(OPTIONS as u64));
    group.bench_function("10000_options", |b| {
        b.iter(|| {
            let mut vega_sum = 0.0;
            for (ty, inputs) in black_box(&book) {
                vega_sum += greeks(*ty, inputs).vega;
            }
            vega_sum
        });
    });
    group.finish();
}

criterion_group!(benches, bench_greeks);
criterion_main!(benches);
