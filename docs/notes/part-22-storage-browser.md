# Part 22: Storage browser

Pages to browse everything the app saves: History, Portfolios, Data, and a saved-run comparison on the Backtest page. All read REST only, with no WebSocket.

## Key maths and rules

- **Session alignment.** Every session starts on the same simulated date, so wall time cannot line two up. Each series is shifted to hours of simulated time since its own first history point. The union of both sets of sample times is the x axis, and a session with no sample at an x has `null` there (drawn with `connectNulls`).
- **Cumulative P&L from history.** `pnl_today.actual` restarts every simulated day. The total at a point is the last value of each earlier New York date, plus today's.
- **Diff.** `diffBooks` matches on position id over `{id, description, quantity}`. In both with a different quantity is "changed"; in one only is added or removed.
- **Usage level.** `ok` below `warn_fraction × limit`, `warning` from there to the limit, `over` above it.

## Design choices

- The range control uses New York dates taken from the session's own history, so a date becomes `from`/`to` instants with no time-zone arithmetic in the browser. The server downsamples; the CSV export asks for `max_points=0` to get every point.
- `GreeksCharts` gained a `stored` mode (no live window, no point cap, dated ticks) and `limits` is optional, so a saved session draws no limit lines.
- The two routes added are read-only. Closes live behind their own route so the archive list stays light.
- Old sessions must not break the list: unreadable meta skips the session; an unreadable last close falls back to `daily-pnl.csv`.

## Interview questions

1. *Why align by elapsed time, not timestamp?* Sessions replay the same simulated dates at different wall times, and each starts at its own first tick. Elapsed simulated time is the only common axis.
2. *Why a pure `portfolioDiff` over a common shape?* The book and a version arrive as different types. Reducing both to id, description and quantity makes the diff trivially testable and independent of either API shape.
3. *Why show a diff before loading a version?* A load replaces the live book. Showing what would be added, removed and changed turns a destructive action into an informed one.
