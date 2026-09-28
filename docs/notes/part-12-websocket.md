# Part 12 - WebSocket streaming and history

## Why `watch` gives natural back-pressure

Each WebSocket client task clones its own `watch::Receiver<Arc<RiskSnapshot>>` from the engine's one `watch::Sender` (`EngineHandle::subscribe_snapshots`). A `watch` channel holds exactly one value — the newest one sent — and a new send simply overwrites whatever was there; it never queues. `Receiver::changed()` marks the current value "seen" the instant it returns, so a client whose task is slow (a stalled TCP write, a busy event loop, a burst of ticks arriving faster than it can serialise and flush them) never builds up a backlog: the next time it polls, it just sees "yes, something changed" once, reads whatever the latest value now is, and silently skips every intermediate tick it missed. The engine's publish side (`snapshot_tx.send(..)`, inside `Engine::spawn`'s loop) never awaits anything and never has to know how many clients exist or how fast any of them are. There is no per-client queue for the engine to feed, so a slow or dead client cannot slow the engine down, and memory cannot grow with the number of missed ticks — it is bounded by the number of *clients*, not the number of *ticks* they missed. This is the whole design of the streaming layer; a bounded channel with a drop policy would have to reinvent, by hand, exactly what `watch` already gives away for free.

Client-side throttling (`{"type":"subscribe","max_hz":2}`) sits on top of this, not instead of it: `Throttle::on_change` decides, on each `changed()` wakeup, whether to send now or defer to a trailing deadline. A deferred change is simply not sent — the `watch` channel has already moved on to whatever the latest value is by the time the trailing timer fires — so throttling is just a client choosing to see fewer of the snapshots that "skip ahead" mechanism would otherwise deliver at full rate, not a second queue bolted on top of the first.

## The message protocol

One JSON object per WebSocket text frame, tagged with `"type"` (`#[serde(tag = "type", rename_all = "snake_case")]`):

| Type | When | Payload |
|---|---|---|
| `hello` | First, once, right after upgrade | `protocol_version`, `server_version`, `tick_interval_ms`, `speed` |
| `snapshot` | Right after `hello`, then on every change (subject to throttle) | `snapshot`: the full `RiskSnapshot` |
| `alert` | As each limit's state changes | `alert`: one `AlertEvent` |
| `resync` | The alert broadcast lagged for this client | `reason`; a fresh `snapshot` follows immediately |
| `error` | Something the client sent was invalid | `message` |

The client may send exactly one thing back: `{"type":"subscribe","max_hz":2}`, which caps how often `snapshot` arrives (default: unthrottled, i.e. the natural tick rate; hard cap 10 Hz regardless of what is requested). A ping is sent every 15 s; the socket closes if no pong has arrived within the last 30 s of real time.

## The snapshot's size, for the real sample portfolio

`config/market.toml`'s five symbols, `data/sample_portfolio.csv`'s 12 positions, offline/fallback data, real `config/limits.toml`: **24,557 bytes** as JSON, measured with `serde_json::to_vec` on the exact `Arc<RiskSnapshot>` a fresh `/ws` connection sees as its first `snapshot` message. At the default 250 ms tick interval that is under 100 KB/s per unthrottled client — the `max_hz` throttle exists for a browser tab that does not need every one of those.

## Design choices worth defending

- **The trailing flush.** A client throttled to, say, 2 Hz would otherwise be able to silently go stale: if the last "skipped" change happens to be the final one before the underlying value stops changing for a while, nothing would ever tell the client about it, because there is no *later* change to trigger a send. `Throttle::on_change` returns `Defer(deadline)` instead of just dropping the update, and a pinned, resettable `tokio::time::Sleep` (guarded by `, if throttle.pending()` in the `select!`, so it is never even polled when nothing is owed) fires exactly once at that deadline to flush the latest known value. The deadline is computed from `last_sent`, not from `now`, so several skipped changes inside one throttle window all resolve to the same, earliest-correct deadline instead of each pushing it back.
- **`Throttle` has no `tokio` or socket dependency.** It is a plain struct over `std::time::Instant`, tested with `let t1 = t0 + Duration::from_millis(500)` — synthetic instants, no real sleeping, no engine, no runtime. The alternative (testing the trailing flush by actually pausing a running engine and waiting) does not work anyway: `Engine::spawn`'s loop calls `snapshot_tx.send(..)` unconditionally on every interval tick regardless of whether the simulated clock is paused, so a "paused" integration test would still see a steady stream of `changed()` wakeups and could never isolate the trailing timer as the cause of a message.
- **`RiskSnapshot.limits` versus `active_alerts`.** `active_alerts` (Part 11) only ever holds limits that have left `Ok` at least once — enough for an alerts pane, useless for a utilisation bar on a limit that is nowhere near its line. Getting the full list into the snapshot without hand-copying `LimitMonitor::evaluate`'s six-limit formula into `engine` needed one small, additive method on `risk::LimitMonitor` itself (`statuses`), sharing the same internal list-building code `evaluate` already had; hysteresis, `active()` and `history()` are completely unaffected.

## Interview questions

1. **Why not just use a `broadcast` channel for snapshots too, the way alerts already do?** `broadcast` delivers every message to every subscriber and makes a slow one pay for it with an explicit lagged error — correct for events, where missing one really does lose information. A snapshot supersedes every earlier one; a client that was two ticks behind gains nothing from seeing the tick it missed, only the current state. `watch` encodes that "only the latest matters" property directly, so a slow client is handled by definition instead of by an error path the client has to react to.
2. **What actually stops a WebSocket client's task from blocking the server on shutdown?** Nothing extra needs to: axum's own graceful shutdown (already wired to the same `CancellationToken` the engine's tick loop watches) drains in-flight connections before `axum::serve(..).with_graceful_shutdown(..)` returns, so a connection's own `socket.recv()` naturally observes the close during that drain. No `CancellationToken` had to be threaded into `ws::run` itself.
3. **Why does the throttle cap at 10 Hz even if a client explicitly asks for more?** The tick rate is already the ceiling in practice (250 ms ticks are 4 Hz), so a client asking for, say, 30 Hz cannot actually receive snapshots any faster than the engine produces them regardless. The cap exists to bound a future faster tick rate (or a misbehaving client) rather than to change today's behaviour, which is why `on_change`'s "unthrottled" default (`min_interval: None`) is simply the natural rate, not `Some(1/10s)`.
