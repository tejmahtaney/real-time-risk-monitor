/**
 * The stream client: one WebSocket connection per source. {@link createStreamClient}
 * builds a client for either the live feed (`/ws`) or one backtest replay
 * (`/ws/backtests/{id}`), each writing to its own store.
 *
 * The live client has a polling fallback while its socket is down, a history backfill
 * once it comes back and a one-off `/api/history` bootstrap. The backtest client has
 * none of these: a replay's history is in its own snapshots, there is nothing to poll,
 * and it reconnects only while the run is still in the server's memory (a run that has
 * finished or been freed has nothing more to stream).
 *
 * The rest of this comment describes the live client, and the live singleton
 * ({@link startStream}) built on the factory.
 *
 * # Why one module-level socket
 *
 * React's StrictMode (development only) intentionally mounts an effect, cleans it up
 * and mounts it again, to flush out effects that are not idempotent. A `useEffect`
 * that opened its own `WebSocket` would therefore open two connections to the same
 * backend on every component mount. Keeping the connection as module-level state,
 * guarded by {@link startStream}'s `started` flag, makes the second StrictMode mount a
 * safe no-op: there is exactly one socket for the whole page, however many components
 * call `startStream()`.
 *
 * # Reconnect strategy
 *
 * Exponential backoff from {@link MIN_BACKOFF_MS} (1 s) to {@link MAX_BACKOFF_MS}
 * (30 s), doubling on every failed attempt, with up to 20 % jitter so many tabs do not
 * all retry in lockstep. The backoff resets to 1 s once a connection has stayed open
 * for {@link STABLE_CONNECTION_MS}, so a brief blip does not leave later reconnects
 * waiting the full 30 s.
 *
 * While the socket is not open, `GET /api/risk-summary` and `GET /api/positions` are
 * polled every {@link POLL_INTERVAL_MS} (2 s), so the shell still shows *something*
 * during an outage; polling stops the moment the socket reopens. On reopening after a
 * previous connection, `GET /api/history?since=<last seq>` backfills whatever ticks
 * were missed, using the store's own optimistic history when the gap is small.
 *
 * On the very first call, {@link startStream} also does one unconditional
 * `GET /api/history` (the full buffered history, no `since`) before ever opening the
 * socket, so the Part 16 charts have something to draw as soon as the page loads,
 * rather than waiting for a whole window's worth of live ticks. `mergeHistory`
 * dedupes by `seq`, so this races harmlessly against the socket's own first snapshot,
 * whichever arrives first.
 */
import type { AlertEvent } from './generated/AlertEvent'
import type { RiskSnapshot } from './generated/RiskSnapshot'
import { ApiError, api } from './rest'
import { useBacktestStore, useRiskStore, type RiskStore } from '../store/riskStore'

const MIN_BACKOFF_MS = 1_000
const MAX_BACKOFF_MS = 30_000
const JITTER_FRACTION = 0.2
const STABLE_CONNECTION_MS = 10_000
const POLL_INTERVAL_MS = 2_000

function wsBase(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${window.location.host}`
}

/** Overridable so tests can point the client at a fake WebSocket without touching
 * `window.location`. Production code leaves this as the default. */
export function defaultWsUrl(): string {
  return `${wsBase()}/ws`
}

/** Where a client's data comes from. */
export type StreamSource = { kind: 'live' } | { kind: 'backtest'; id: string }

/** The WebSocket URL for a source: `/ws` for live, `/ws/backtests/{id}` for a replay. */
export function wsUrlForSource(source: StreamSource): string {
  return source.kind === 'live'
    ? defaultWsUrl()
    : `${wsBase()}/ws/backtests/${encodeURIComponent(source.id)}`
}

/** Options for {@link createStreamClient}. */
export interface StreamOptions {
  source: StreamSource
  /** The store the client writes to. Defaults to the live store, or the backtest store. */
  store?: RiskStore
  /** Overrides the socket URL; tests use this. */
  wsUrl?: () => string
  /** Backtest only: whether a closed socket should be reopened. Defaults to asking the
   * server whether the run is still in memory and unfinished. */
  shouldReconnect?: () => Promise<boolean>
}

/** A running (or stopped) stream connection. */
export interface StreamClient {
  /** Starts the client. A second call is a no-op, so StrictMode's double mount is safe. */
  start(): void
  /** Closes the socket and cancels every timer. The client cannot be restarted. */
  stop(): void
}

interface StreamState {
  started: boolean
  stopped: boolean
  socket: WebSocket | null
  backoffMs: number
  lastSeq: number | undefined
  reconnectTimer: ReturnType<typeof setTimeout> | null
  stableTimer: ReturnType<typeof setTimeout> | null
  pollTimer: ReturnType<typeof setInterval> | null
  seenUnknownTypes: Set<string>
}

function freshState(): StreamState {
  return {
    started: false,
    stopped: false,
    socket: null,
    backoffMs: MIN_BACKOFF_MS,
    lastSeq: undefined,
    reconnectTimer: null,
    stableTimer: null,
    pollTimer: null,
    seenUnknownTypes: new Set(),
  }
}

async function runIsStillStreaming(id: string): Promise<boolean> {
  try {
    const status = await api.backtestStatus(id)
    return status.in_memory && status.state !== 'finished' && status.state !== 'unfinished'
  } catch (err) {
    // A 404 means the run is gone; any other failure is a blip worth retrying.
    return !(err instanceof ApiError && err.status === 404)
  }
}

/** Builds a stream client for one source. Nothing connects until `start()`. */
export function createStreamClient(options: StreamOptions): StreamClient {
  const live = options.source.kind === 'live'
  const store: RiskStore = options.store ?? (live ? useRiskStore : useBacktestStore)
  const wsUrl = options.wsUrl ?? (() => wsUrlForSource(options.source))
  const shouldReconnect =
    options.shouldReconnect ??
    (options.source.kind === 'backtest'
      ? () => runIsStillStreaming((options.source as { id: string }).id)
      : () => Promise.resolve(true))
  let state = freshState()

  function setError(err: unknown): void {
    store.getState().setError(err instanceof Error ? err.message : String(err))
  }

  /** The one-time "on load" backfill described in this module's doc comment: whatever
   * the engine's history ring buffer already holds, fetched once up front regardless
   * of whether the socket is even open yet. Live only. */
  async function bootstrapHistory(): Promise<void> {
    try {
      store.getState().mergeHistory(await api.history())
    } catch (err) {
      setError(err)
    }
  }

  async function backfillHistory(since: number): Promise<void> {
    try {
      store.getState().mergeHistory(await api.history(since))
    } catch (err) {
      setError(err)
    }
  }

  async function pollOnce(): Promise<void> {
    try {
      const [summary, positions] = await Promise.all([api.riskSummary(), api.positions()])
      store.getState().setPolled(summary, positions)
    } catch (err) {
      setError(err)
    }
  }

  function startPolling(): void {
    if (state.pollTimer) return
    void pollOnce()
    state.pollTimer = setInterval(() => {
      void pollOnce()
    }, POLL_INTERVAL_MS)
  }

  function stopPolling(): void {
    if (state.pollTimer) {
      clearInterval(state.pollTimer)
      state.pollTimer = null
    }
  }

  function scheduleReconnect(): void {
    if (state.reconnectTimer || state.stopped) return
    const capped = Math.min(state.backoffMs, MAX_BACKOFF_MS)
    const jitter = Math.random() * capped * JITTER_FRACTION
    state.reconnectTimer = setTimeout(() => {
      state.reconnectTimer = null
      state.backoffMs = Math.min(state.backoffMs * 2, MAX_BACKOFF_MS)
      connect()
    }, capped + jitter)
  }

  function connect(): void {
    if (state.stopped) return
    const socket = new WebSocket(wsUrl())
    state.socket = socket

    socket.onopen = () => {
      stopPolling()
      store.getState().setStatus('open')
      store.getState().setError(null)
      state.stableTimer = setTimeout(() => {
        state.backoffMs = MIN_BACKOFF_MS
      }, STABLE_CONNECTION_MS)
      if (live && state.lastSeq !== undefined) {
        void backfillHistory(state.lastSeq)
      }
    }

    socket.onmessage = (event: MessageEvent<string>) => {
      handleMessage(event.data)
    }

    socket.onclose = () => {
      if (state.stableTimer) {
        clearTimeout(state.stableTimer)
        state.stableTimer = null
      }
      state.socket = null
      if (state.stopped) return
      if (live) {
        store.getState().setStatus('polling')
        startPolling()
        scheduleReconnect()
        return
      }
      store.getState().setStatus('closed')
      void shouldReconnect().then((again) => {
        if (again && !state.stopped) scheduleReconnect()
      })
    }

    // A browser WebSocket always follows `onerror` with `onclose`, so there is nothing
    // extra to do here beyond letting `onclose` run the shared recovery path.
    socket.onerror = () => {}
  }

  function isRecord(value: unknown): value is Record<string, unknown> {
    return typeof value === 'object' && value !== null
  }

  /** Parses one incoming frame and dispatches it by its `type` tag. An unrecognised
   * `type` (a newer server talking to an older build of this frontend) is logged once
   * and otherwise ignored, rather than throwing and killing the socket's message loop. */
  function handleMessage(raw: string): void {
    let parsed: unknown
    try {
      parsed = JSON.parse(raw)
    } catch {
      return
    }
    if (!isRecord(parsed) || typeof parsed.type !== 'string') return

    switch (parsed.type) {
      case 'hello':
        break
      case 'snapshot': {
        const snapshot = (parsed as { snapshot: RiskSnapshot }).snapshot
        state.lastSeq = snapshot.seq
        store.getState().applySnapshot(snapshot)
        break
      }
      case 'alert': {
        const alert = (parsed as { alert: AlertEvent }).alert
        store.getState().addAlert(alert)
        break
      }
      case 'resync': {
        const reason = (parsed as { reason: string }).reason
        store.getState().setError(`resync: ${reason}`)
        if (live && state.lastSeq !== undefined) void backfillHistory(state.lastSeq)
        break
      }
      case 'error': {
        const message = (parsed as { message: string }).message
        store.getState().setError(message)
        break
      }
      default:
        if (!state.seenUnknownTypes.has(parsed.type)) {
          state.seenUnknownTypes.add(parsed.type)
          console.warn(`stream: ignoring unknown message type "${parsed.type}"`)
        }
    }
  }

  return {
    start() {
      if (state.started) return
      state.started = true
      if (live) void bootstrapHistory()
      connect()
    },
    stop() {
      state.stopped = true
      if (state.reconnectTimer) clearTimeout(state.reconnectTimer)
      if (state.stableTimer) clearTimeout(state.stableTimer)
      if (state.pollTimer) clearInterval(state.pollTimer)
      const socket = state.socket
      state.socket = null
      state.reconnectTimer = null
      state.stableTimer = null
      state.pollTimer = null
      socket?.close()
      state = { ...freshState(), started: true, stopped: true }
    },
  }
}

let liveClient: StreamClient | null = null

/** Starts the live stream, if it is not already running. Safe to call from every
 * component that wants live data; only the first call has any effect. */
export function startStream(wsUrl: () => string = defaultWsUrl): void {
  if (liveClient) return
  liveClient = createStreamClient({ source: { kind: 'live' }, wsUrl })
  liveClient.start()
}

/** Tears down the live singleton and resets its state. Production code never calls
 * this (the page just closes); it exists so tests get a clean slate between cases. */
export function stopStreamForTests(): void {
  liveClient?.stop()
  liveClient = null
}
