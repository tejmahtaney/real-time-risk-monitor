# Part 23 - README and maths documentation

No new maths. This part documents the maths already built and checks each formula against the code.

## What a reviewer should read first

1. `README.md`, "The maths" section. Check the formulas against `crates/pricing/src/greeks.rs`, `crates/portfolio/src/risk.rs` and `crates/risk/src/var/mod.rs`.
2. `README.md`, "Limitations and future work". It is deliberately blunt.
3. `docs/ARCHITECTURE.md`, "The tick pipeline" and "Concurrency and back-pressure".
4. The two UI fixes in the first commit: `frontend/src/lib/chartLimits.ts` and `CompareRuns.tsx`.

## Design choices

- Numbers in the docs come only from tests, benchmarks or `docs/PROGRESS.md`, each with its date or machine. Where notes disagreed, the later figure won (VaR window 60, not 99; `var_99` 32,500, not 2,000,000).
- The `compute_micros` figure was taken from a debug build, and the table says so. Only the criterion and VaR figures are release.
- The chart band now appears only when the data is within 10 % of the warning line and still fills half the plot height. Otherwise the limit is a corner label.

## Questions an interviewer might ask

1. *Why show a residual?* An honest small remainder is more useful than a decomposition that pretends to be exact.
2. *Why does the Kupiec pass at n = 80 mean little?* 0.8 exceptions are expected, and the test rejects only from 4.
3. *Why does a slow WebSocket client not slow the engine?* A `watch` channel holds one value and overwrites it, so there is no queue.
