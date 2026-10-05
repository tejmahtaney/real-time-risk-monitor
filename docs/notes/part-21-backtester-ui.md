# Part 21 - Backtester UI

**Key idea.** A replay is just another snapshot stream, so the panels that already take props are reused unchanged. What changes is where the data comes from and when a run's memory is freed.

**Design choices.**
- **One client per source.** `createStreamClient` replaces the module singleton. Live keeps its history bootstrap, polling fallback and backfill. A replay has none: its history is its own snapshots, REST has nothing to poll, and reconnecting is pointless once a run is finished or freed, so it asks `GET /api/backtests/{id}` first. Separate zustand stores keep a replay from overwriting live data, so the live dashboard is still live afterwards.
- **StrictMode-safe freeing.** Development mounts, unmounts and mounts every effect. Freeing on unmount would delete the run on the throw-away unmount. The release is deferred one task and a re-mount of the same run cancels it. A real route change does not re-take the lease, so it fires. On page unload a `keepalive` `fetch` sends the DELETE, because `sendBeacon` is POST only.
- **Status is not the snapshot.** In replay `snapshot.paused` is always false and `change_since_open` is 0, so play state and progress come from `BacktestStatus`. The header keeps showing live mode.
- **Results from `BacktestResults` only.** Never the stream, so a run looks the same live, saved or after a restart. `partial` and `unfinished` are labelled.
- **Kupiec in words.** Reject with more exceptions than expected: understates risk. Fewer: overstates. Otherwise consistent. Under 250 days the sentence adds that the test has little power, because a pass then says little (80 days expect 0.8 exceptions).
- **Archive coverage** is computed by the same merge and alignment as a run, so the number on the button is the number of dates a run would replay.

**Interview questions.**
1. *Why not delete the run in a plain `useEffect` cleanup?* StrictMode runs the cleanup once before the real mount, which would free the run the user just started. A deferred, cancellable release only fires when no new mount takes the run.
2. *Why does a drawdown chart start its peak at zero?* P&L starts at zero, so a run that only loses is already in drawdown from day one; it matches the backend's `max_drawdown`.
3. *Is a Kupiec pass good evidence?* Not on a short replay. With n = 80 and 99 % VaR it expects 0.8 exceptions and rejects only from 4, so only a rejection means anything.
