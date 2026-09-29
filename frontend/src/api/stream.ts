/**
 * The live data client: one WebSocket connection to `/ws`, with a polling fallback
 * while it is down and a history backfill once it comes back.
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
import { api } from './rest'
import { useRiskStore } from '../store/riskStore'

const MIN_BACKOFF_MS = 1_000
const MAX_BACKOFF_MS = 30_000
const JITTER_FRACTION = 0.2
const STABLE_CONNECTION_MS = 10_000
const POLL_INTERVAL_MS = 2_000

/** Overridable so tests can point the client at a fake WebSocket without touching
 * `window.location`. Production code leaves this as the default. */
export function defaultWsUrl(): string {
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${window.location.host}/ws`
}

interface StreamState {
  started: boolean
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
    socket: null,
    backoffMs: MIN_BACKOFF_MS,
    lastSeq: undefined,
    reconnectTimer: null,
    stableTimer: null,
    pollTimer: null,
    seenUnknownTypes: new Set(),
  }
}

let state = freshState()

/** Starts the singleton stream, if it is not already running. Safe to call from every
 * component that wants live data; only the first call has any effect. */
export function startStream(wsUrl: () => string = defaultWsUrl): void {
  if (state.started) return
  state.started = true
  void bootstrapHistory()
  connect(wsUrl)
}

/** The one-time "on load" backfill described in this module's doc comment: whatever
 * the engine's history ring buffer already holds, fetched once up front regardless
 * of whether the socket is even open yet. */
async function bootstrapHistory(): Promise<void> {
  try {
    const points = await api.history()
    useRiskStore.getState().mergeHistory(points)
  } catch (err) {
    useRiskStore.getState().setError(err instanceof Error ? err.message : String(err))
  }
}

/** Tears down the singleton and resets all module state. Production code never calls
 * this (the page just closes); it exists so tests get a clean slate between cases. */
export function stopStreamForTests(): void {
  if (state.reconnectTimer) clearTimeout(state.reconnectTimer)
  if (state.stableTimer) clearTimeout(state.stableTimer)
  if (state.pollTimer) clearInterval(state.pollTimer)
  state.socket?.close()
  state = freshState()
}

function connect(wsUrl: () => string): void {
  const socket = new WebSocket(wsUrl())
  state.socket = socket

  socket.onopen = () => {
    stopPolling()
    useRiskStore.getState().setStatus('open')
    useRiskStore.getState().setError(null)
    state.stableTimer = setTimeout(() => {
      state.backoffMs = MIN_BACKOFF_MS
    }, STABLE_CONNECTION_MS)
    if (state.lastSeq !== undefined) {
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
    useRiskStore.getState().setStatus('polling')
    startPolling()
    scheduleReconnect(wsUrl)
  }

  // A browser WebSocket always follows `onerror` with `onclose`, so there is nothing
  // extra to do here beyond letting `onclose` run the shared recovery path.
  socket.onerror = () => {}
}

function scheduleReconnect(wsUrl: () => string): void {
  if (state.reconnectTimer) return
  const capped = Math.min(state.backoffMs, MAX_BACKOFF_MS)
  const jitter = Math.random() * capped * JITTER_FRACTION
  state.reconnectTimer = setTimeout(() => {
    state.reconnectTimer = null
    state.backoffMs = Math.min(state.backoffMs * 2, MAX_BACKOFF_MS)
    connect(wsUrl)
  }, capped + jitter)
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

async function pollOnce(): Promise<void> {
  try {
    const [summary, positions] = await Promise.all([api.riskSummary(), api.positions()])
    useRiskStore.getState().setPolled(summary, positions)
  } catch (err) {
    useRiskStore.getState().setError(err instanceof Error ? err.message : String(err))
  }
}

async function backfillHistory(since: number): Promise<void> {
  try {
    const points = await api.history(since)
    useRiskStore.getState().mergeHistory(points)
  } catch (err) {
    useRiskStore.getState().setError(err instanceof Error ? err.message : String(err))
  }
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
      useRiskStore.getState().applySnapshot(snapshot)
      break
    }
    case 'alert': {
      const alert = (parsed as { alert: AlertEvent }).alert
      useRiskStore.getState().addAlert(alert)
      break
    }
    case 'resync': {
      const reason = (parsed as { reason: string }).reason
      useRiskStore.getState().setError(`resync: ${reason}`)
      if (state.lastSeq !== undefined) void backfillHistory(state.lastSeq)
      break
    }
    case 'error': {
      const message = (parsed as { message: string }).message
      useRiskStore.getState().setError(message)
      break
    }
    default:
      if (!state.seenUnknownTypes.has(parsed.type)) {
        state.seenUnknownTypes.add(parsed.type)
        console.warn(`stream: ignoring unknown message type "${parsed.type}"`)
      }
  }
}
