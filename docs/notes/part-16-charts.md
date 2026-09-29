# Part 16 - Charts and P&L headline

## Key design choices

**Chart components stay prop-driven; `Dashboard.tsx` remains the only file that reads
the store.** `GreeksCharts`, `AllocationDonut` and `PnlAttributionPanel` all take
plain props, exactly like the four Part 15 panels. The one place this needed care:
`GreeksCharts` originally read `history` straight out of the store via a
`useThrottledHistory` hook called inside the component itself. That broke the rule,
so the hook call moved up into `Dashboard.tsx` (the one file allowed to touch
`useRiskStore`), and `GreeksCharts` now takes `history: HistoryPoint[]` as a prop
like everything else. `useThrottledHistory` itself lives in `src/hooks/` as a small,
reusable subscription helper, not store-specific logic baked into a panel.

**Update throttling is a store subscription, not a `setInterval` poll.**
`useThrottledHistory` (`src/hooks/useThrottledHistory.ts`) subscribes to
`useRiskStore` directly and keeps the latest `history` array in a `ref`, but only
calls `setState` — and therefore only triggers a re-render — on a timer that fires
at most twice a second (`intervalMs = 500`, matching the prompt's "at most twice a
second"). A tick that lands between two timer fires is coalesced into the next one
rather than dropped: the ref always holds the very latest array, so the chart never
falls behind, it just doesn't redraw on every single tick. Recharts' own animation
is also switched off on every live `<Line>` (`isAnimationActive={false}`), since an
animated transition on a series that repaints twice a second never finishes before
the next repaint starts. Each metric's plotted array is then built with `useMemo`
inside `GreeksCharts`, keyed on the (already-throttled) `history` reference and the
window selection, so an unrelated re-render of `Dashboard` does not recompute four
arrays for nothing.

**The window selector filters and caps in `sim_time`, not wall-clock time or index.**
`lib/chartWindow.ts`'s `filterWindow` measures "5 minutes" and "15 minutes" from the
*last point's own `sim_time`*, because the simulator runs at up to 600× real speed —
a wall-clock 5-minute window would show hours of simulated history. `'all'` returns
everything the store has buffered (already capped at 2,400 points, `HISTORY_CAP` in
`riskStore.ts`), and a second, tighter cap (`MAX_CHART_POINTS = 600`) keeps a long
`'all'` session from asking Recharts to lay out thousands of points at once —
keeping the *most recent* points, since a chart reads left-to-right toward "now".

**The dashed limit line and shaded band are derived from `LimitStatus`, not a new
API field.** `portfolio_dollar_delta`, `portfolio_dollar_gamma1_pct` and
`portfolio_vega` are all limits on an *absolute* value, so a breach can come from
either side — `lib/chartLimits.ts`'s `metricBounds` draws a mirrored line and band on
both the positive and negative side. `portfolio_theta_floor` compares
`max(0, -theta)` against its limit (a book earning theta can never breach it), so
theta gets only a lower bound. The one piece of data the charts need but the wire
format doesn't carry is `config/limits.toml`'s `warning_threshold` (a single 0.8
applied to every limit, per `risk::LimitsConfig`) — since this part's prompt scopes
the work to `frontend/`, that fraction is reproduced as a documented constant
(`LIMIT_WARNING_THRESHOLD`) rather than plumbed through a new field on
`LimitStatus`. If `config/limits.toml`'s value ever changes, this constant has to
change with it — a real, if small, coupling that a later part could close by adding
the fraction to the wire type instead.

**One fixed colour per symbol, shared by the donut and the positions table.**
`lib/symbolColors.ts` gives each of the five symbols `config/market.toml` actually
calibrates (AAPL, MSFT, NVDA, SPY, TSLA) its own slot in an 8-hue categorical
palette, validated against this app's dark surface (`#111827`) for CVD separation,
contrast and lightness band. Any other symbol (the add-position form accepts a free
`underlying` string) falls back to a deterministic hash into the three remaining
slots, so it never collides with one of the five known symbols, though two unknown
symbols could in principle still hash together — an acceptable gap for a symbol the
simulator was never calibrated to price in the first place. `PositionsTable`'s group
header now shows the same colour as a small dot next to the symbol.

**The P&L attribution's stacked segments are coloured by sign, not by bucket
category.** The main "today" bar chart is explicitly green-for-gain,
red-for-loss, neutral-for-residual per the prompt. Rather than introduce a second
categorical palette for the seven stacked buckets in the per-underlying breakdown
(risking a colour that means "AAPL" in the donut also meaning "delta bucket" in the
attribution chart), every stacked segment reuses the same sign-based colouring —
each `<Cell>` is coloured by *that bucket's own value* for *that underlying*, so a
segment that hurt is red and one that helped is green regardless of which Greek it
came from. Each segment's outline (`stroke`) is the underlying's own `symbolColor`,
which ties the stack back to the symbol legend without a second palette.

**The headline is built only from the six named buckets, both for the winners and
for "Net".** `lib/headline.ts`'s `buildHeadline` takes `delta`, `gamma`, `vega`,
`theta`, `rho` and `higher_order` — never `residual` or `actual`, per the prompt.
"Net" is therefore the sum of those six (`PnlBuckets::explained()`'s own definition
on the Rust side), not the true `actual` P&L; the two can differ by the residual,
which the sentence deliberately never mentions. A tie between two buckets is broken
by a fixed priority order (delta, gamma, vega, theta, rho, higher-order), so the
sentence is always deterministic. An all-zero day gets its own sentence rather than
naming a "biggest mover" that isn't one.

## Chart update throttling (summary)

`Dashboard.tsx` calls `useThrottledHistory()` once; every chart panel underneath
receives the same throttled `history` array as a prop. The hook subscribes to the
store outside of React's normal render cycle and only flushes into `useState` on a
500ms timer, so a 250ms tick interval (the default `config/simulation.toml`) cannot
drive more than 2 re-renders a second no matter how fast snapshots arrive. Combined
with `isAnimationActive={false}` on every live series and `useMemo`'d per-metric
arrays, the four line charts stay smooth even at high `clock_speed`.

## How the headline chooses its buckets

`buildHeadline(buckets: PnlBuckets)` looks at exactly six fields — the Taylor
decomposition minus `residual` and `higher_order`'s siblings `rho`/`actual` are
kept, only `residual` and `actual` are excluded. It finds the largest positive value
(the "made $X from ...” clause) and the most negative value (the "lost $Y to ..."
clause) independently; either clause is omitted if there is no bucket of that sign.
Net is the sum of the same six buckets. All six exactly zero gets a dedicated flat
sentence instead of forcing a "biggest mover" out of nothing. Ties are broken by a
fixed bucket order so the same input always produces the same sentence.

## Screenshot review fixes

No browser was available in the sandbox this part was built in, so
`frontend/scripts/screenshot.ts` (both widths) was run outside it, by the person
reviewing the part, over two rounds. The first round found five real layout
problems, listed below and fixed directly in the code. A second round confirmed
four of the five (timezone, the pulse, 1280px wrapping, the small fixes) but found
the first attempt at item 4 (the chart y-axis) still flattened the chart, just for a
different reason — see item 4 for the corrected design. Neither round of fixes has
itself been re-screenshotted from inside this sandbox to confirm, since the
no-browser limitation still applies here.

1. **Timezone bug.** Every sim time (header, alert history, chart axes) was
   rendered with a bare `toLocaleString`/`toLocaleTimeString` call, i.e. the
   *viewer's* browser timezone, not the market's — a browser in Singapore showed a
   10:16 AM New York tick as `10:16:00 PM`. `lib/format.ts` now has one
   `formatSimTime(iso, opts)`, explicitly passing `timeZone: 'America/New_York'` to
   `toLocaleString` and appending an explicit "ET" suffix; `Header.tsx` and
   `AlertsPane.tsx`'s own local, timezone-naive copies of `formatSimTime` were
   deleted in favour of the shared one. A test forces `TZ=Asia/Singapore` (via
   `vi.stubEnv`, a fixed UTC+8 offset with no DST, chosen to contrast cleanly with
   US Eastern) and asserts the output is unchanged, since the whole point of an
   explicit `timeZone` option is that the runtime's own local zone cannot affect it.
2. **Positions table background flash.** At a fast tick rate the 400ms background
   flash (`flash-up`/`flash-down`, Part 15) never fully cleared before the next tick
   retriggered it, leaving nearly every numeric cell permanently tinted, and a green
   background under red (loss) text was illegible. Two changes: (a)
   `useCellFlashes` now diffs each cell's *displayed, formatted text*
   (`cellText`), not the raw float, so a change too small to show up on screen no
   longer counts as a change at all; (b) the pulse itself moved from a background
   colour to a `text-shadow` glow (`text-pulse-up`/`text-pulse-down` in
   `index.css`), which never fights with `signColor`'s green/red/neutral text
   colour underneath, and leaves the background permanently neutral.
3. **1280px layout.** Descriptions wrapping to two lines, a minus sign splitting
   from its number, and long tile values (`$1.4k / vol pt`) wrapping were all the
   same root cause: the table was forced to exactly `w-full` and cells had no
   `whitespace-nowrap`. Every `<td>`/`<th>` in `PositionsTable.tsx` now has
   `whitespace-nowrap`; the table itself is `w-full min-w-max` (at least the
   container's width, but free to grow with its content) inside an
   `overflow-x-auto` wrapper, so a narrow viewport scrolls horizontally instead of
   wrapping text. The old fixed `max-h-[32rem]` vertical cap on the table (which
   also caused item 5's row-cutting) was dropped for the same reason — nothing
   clips vertically now either.
4. **Flat gamma/vega charts and overlapping vega axis labels.** Recharts' own
   default y-axis domain starts at 0, which flattens a series whose whole range
   sits far from zero into what looks like a flat line — exactly this book's dollar
   gamma and vega. The first fix (`paddedYDomain`) padded 10% around the data *plus*
   both the upper and lower configured limits, which turned out to be its own
   version of the same bug: `portfolio_dollar_gamma1_pct`/`_vega` are limits on an
   *absolute* value, so both bounds exist and both sit far from zero (e.g. ±$9.5k
   for gamma), stretching the axis to ±the limit regardless of where the data
   actually was — gamma at $194 still ended up on a −$11.4k to +$11.4k axis. The
   real fix, `lib/chartLimits.ts`'s `computeChartScale`, scales the domain from the
   data *alone* (~10% padding, with a small minimum span so a constant series is
   never zero-height), then separately decides whether to draw a limit at all: only
   the side the data's own extremes lean towards (by magnitude) is ever considered,
   and even then only if that limit sits within 1.5× the data-only domain's reach on
   that side — close enough to be worth a modest extension, not so far that drawing
   it would flatten the chart all over again. A limit that fails that check is
   surfaced as a small "limit $9.5k" label next to the chart's title instead of
   being drawn on the axis at all. Separately, `formatPerVolPoint`/`formatPerDay`
   (which appended `" / vol pt"`/`" / day"`) were deleted entirely — axis ticks and
   tooltips now use plain `formatCompactCurrency` (`$1.2k`) on every chart, and the
   unit moved into the chart's own title (`Vega ($ / vol pt)`, `Theta ($ / day)`)
   instead, which is what was making the vega axis's labels tall enough to overlap
   the shared time axis below it.
5. **Small fixes.** The "By underlying" P&L stacked chart's category `YAxis` now
   has `interval={0}` (Recharts was auto-skipping some category ticks — MSFT and
   SPY — to avoid a collision it judged unnecessary given the chart's default
   height; the per-row height was also bumped from 28px to 32px for the same
   reason) so every underlying's label always shows. `RiskTiles`' `compute_micros`
   tile is now labelled "Compute time". The positions panel's row-cutting was the
   same fix as item 3 (no vertical cap left to cut a row in half).

## Questions an interviewer might ask

1. **Why filter the chart window by `sim_time` instead of just taking the last N
   points?** Because the simulator's tick rate and `clock_speed` are independent of
   how much *simulated* time has passed — a slow real machine falling behind on
   ticks, or a `clock_speed` change mid-session, would otherwise make "5 minutes"
   mean a different amount of simulated history depending on how fast the last few
   ticks happened to arrive. Measuring from `sim_time` keeps the window's meaning
   ("the last five minutes of market time") stable regardless of tick pacing.
2. **Why does `GreeksCharts` take `history` as a prop instead of just calling
   `useThrottledHistory()` itself, given the hook already reads the store?** Because
   Part 15 established that only the dashboard container reads `useRiskStore`, so
   that Part 21's backtest replay can feed the same panels from a different source
   by changing one file. A hook that reads the store directly, even a throttled one,
   would put that same store-reading concern inside a panel component again.
3. **Why is `LIMIT_WARNING_THRESHOLD` a duplicated constant instead of a new field on
   `LimitStatus`?** Because this part's prompt scopes the work to `frontend/`, and
   `warning_threshold` is one number in `config/limits.toml`, not per-limit — adding
   it to the wire type would need a small `risk`/`engine`/`server` change that
   belongs to a part that touches the backend. The duplication is documented at its
   one call site specifically so it is easy to find and fix together if the config
   value ever changes.
