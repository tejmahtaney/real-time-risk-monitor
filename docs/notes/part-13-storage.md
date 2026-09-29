# Part 13 - Local storage and session recording

## Folder layout

```
storage/
  market-data/{SYMBOL}/<ts>-daily.json         from Part 7
  portfolios/current.json                      live book, rewritten atomically on every change
  portfolios/versions/<ts>-<slug>.csv          saved versions, never overwritten
  sessions/<session_id>/meta.json              start and end times, seed, speed, git commit, dropped records
  sessions/<session_id>/config/                copies of every config file the session used
  sessions/<session_id>/history.jsonl          one HistoryPoint per tick
  sessions/<session_id>/events.jsonl           alerts, settlements, position changes, control actions
  sessions/<session_id>/daily-pnl.csv          one row per simulated day with every P&L bucket
  sessions/<session_id>/closes/<sim_date>.json full RiskSnapshot at each session close
```

A session id is its UTC start time plus a short random suffix (`20260926T041500Z-k3f9`), so ids sort by time. `backtests/` (Part 20) and `exports/` are reserved but unused so far.

## Why the recorder never blocks the engine

The tick loop hands every record — a `HistoryPoint`, a `SessionEvent`, a session-close snapshot, a daily P&L row, or the current portfolio — to the recorder over a bounded `mpsc` channel (capacity 10,000) with `try_send`, never `send`. A full channel is not waited on; the record is dropped and a counter increments. This is exactly the back-pressure rule `server::ws` already uses for its `watch`/`broadcast` channels to live UI clients: never let a slow consumer set the producer's pace. The two differ only in what gets dropped (a counted record here, a superseded snapshot there) and in using a bounded queue rather than "one slot, always overwritten" — a session recording should keep as much history as it reasonably can, unlike a live view that only ever needs the newest value. All file I/O in the engine crate lives inside the recorder task; `Engine::tick` and `Engine::apply` never touch the disk themselves.

## Crash-safety rules

- **JSON files** (`meta.json`, `current.json`, `closes/<date>.json`) are written with the Part 7 atomic pattern: serialise to a temp file in the same directory, `fsync`, then `rename`. A crash mid-write leaves either the old file or nothing, never a half-written one.
- **JSONL files** (`history.jsonl`, `events.jsonl`) are append-only, flushed at least once a second and once more at shutdown. A crash can leave a truncated last line; readers in `crates/storage` detect and skip it with a warning rather than failing the whole file, while a broken line anywhere *before* the last one is a hard error, since that can only mean real corruption.
- **`meta.json`** is written once at session start with `end: None`, and rewritten in place at shutdown with `end` set and the final dropped-record count. A session whose `end` is still `None` was cut short (crash or `kill -9`); every reader treats that as "still running," not as bad data.
- **Shutdown order** is: stop accepting new ticks, drain whatever is still in the recorder's channel with `try_recv`, flush the batched JSONL writers, write the final `meta.json`, then let the process exit. `Engine::spawn`'s returned task handle resolves only once the recorder has finished this sequence, so awaiting it in `main` is what makes the ordering real rather than a comment.

## Why portfolio version I/O lives in the server handlers, not the engine

`GET/POST /api/portfolio/versions` and `POST /api/portfolio/versions/{file}/load` read and write the version CSV files directly in `crates/server/src/storage_routes.rs`, not through a command the engine task executes. The engine only ever receives or returns already-parsed `PositionSpec`s (`EngineCommand::GetSpecs`, `EngineCommand::LoadPortfolio`). This keeps the one rule from the recorder section intact in the other direction too: nothing that runs on the engine's own tick-loop task ever opens a file. A save or load happens on an HTTP request, off the tick path entirely, so there is no reason to route it through the single-owner command channel just to touch disk — doing so would only add a hop for no benefit, since the specs it produces or needs are exactly what the channel already carries.

## Why settled positions are left out of `current.json`

A settled option's cash has already been paid out and the settlement itself is recorded once in `events.jsonl`. If a settled position stayed in `current.json`, a restart would resolve the very same book again and pay it out a second time. `Engine::active_positions` filters the book down to positions with no recorded settlement before it is ever serialised, so both the automatic `current.json` rewrite and a manually saved portfolio version only ever contain positions that are still live.

## Measured bytes per hour

A live session recorded for 398 real seconds (default `config/simulation.toml`: `tick_interval_ms = 250`, `speed = 120`, so about 6.6 real minutes covers roughly two simulated trading days) left a session folder of 946,747 bytes (`history.jsonl` 855,471; `events.jsonl` 1,301; `daily-pnl.csv` 362; `meta.json` 248; `config/` and `closes/` the rest). That scales to **≈ 8.56 MB (≈ 8.17 MiB) per real hour**, almost all of it `history.jsonl` at four points a second.

## Interview questions

**Q: Why `try_send` and a counter instead of a larger channel or an unbounded one?**
A: An unbounded channel just moves the problem from "records get dropped" to "memory grows without limit if the disk falls behind," which is worse for a process meant to run for hours. A bounded channel with `try_send` gives the engine a hard, fixed cost per tick (one channel op) regardless of what the recorder is doing, and the dropped count in `meta.json`/`/api/health` makes any actual back-pressure visible instead of silent.

**Q: Why filter settled positions out of the persisted book instead of marking them settled in the file?**
A: The book's own settlement record (kept in-memory and mirrored to `events.jsonl`) is already the source of truth for "this happened once." Persisting settled positions at all would mean every reader of `current.json` — including a restart six months later — would need to re-derive "is this actually still owed" from data that was only ever a point-in-time snapshot, rather than just reading what's there.

**Q: What happens if the process is killed mid-tick, between the recorder receiving a record and it being flushed?**
A: At most one second of buffered `history.jsonl`/`events.jsonl` lines is lost (the flush interval), and `meta.json`'s `end` stays `None`, which callers must treat as "session was cut short." `current.json` and `closes/<date>.json` are written atomically as whole files, so they are never observed half-written — they simply reflect whatever the last completed write was before the crash.
