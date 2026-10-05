/**
 * Typed wrappers for every REST route the backend exposes (`crates/server/src/routes.rs`
 * and `crates/server/src/storage_routes.rs`). Every request and response type here is
 * generated from the Rust types by `ts-rs` (see `src/api/generated/`), so a route's
 * shape can never silently drift between the backend and this file.
 */
import type { ArchiveCoverage } from './generated/ArchiveCoverage'
import type { AlertsResponse } from './generated/AlertsResponse'
import type { ArchiveFileInfo as _ArchiveFileInfo } from './generated/ArchiveFileInfo'
import type { BacktestControlRequest } from './generated/BacktestControlRequest'
import type { BacktestCreateRequest } from './generated/BacktestCreateRequest'
import type { BacktestListItem } from './generated/BacktestListItem'
import type { BacktestResults } from './generated/BacktestResults'
import type { BacktestStatus } from './generated/BacktestStatus'
import type { ControlRequest } from './generated/ControlRequest'
import type { GreeksResponse } from './generated/GreeksResponse'
import type { HealthResponse } from './generated/HealthResponse'
import type { HistoryPoint } from './generated/HistoryPoint'
import type { LoadedVersionResponse } from './generated/LoadedVersionResponse'
import type { PnlReport } from './generated/PnlReport'
import type { PositionRisk } from './generated/PositionRisk'
import type { PositionSpec } from './generated/PositionSpec'
import type { RiskSummaryResponse } from './generated/RiskSummaryResponse'
import type { SavedVersionResponse } from './generated/SavedVersionResponse'
import type { SaveVersionRequest } from './generated/SaveVersionRequest'
import type { SessionDetail } from './generated/SessionDetail'
import type { SessionEvent } from './generated/SessionEvent'
import type { SessionSummary } from './generated/SessionSummary'
import type { StorageUsageResponse } from './generated/StorageUsageResponse'
import type { SymbolArchive } from './generated/SymbolArchive'
import type { VarReport } from './generated/VarReport'
import type { VersionSummary } from './generated/VersionSummary'

/** Re-exported so callers of {@link marketDataArchive} do not need their own import. */
export type ArchiveFileInfo = _ArchiveFileInfo

/** Thrown when a request completes but the server reports an error status. */
export class ApiError extends Error {
  readonly status: number

  constructor(status: number, message: string) {
    super(message)
    this.name = 'ApiError'
    this.status = status
  }
}

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path)
  if (!response.ok) {
    throw new ApiError(response.status, await describeError(response))
  }
  return (await response.json()) as T
}

async function postJson<T>(path: string, body?: unknown): Promise<T> {
  const response = await fetch(path, {
    method: 'POST',
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!response.ok) {
    throw new ApiError(response.status, await describeError(response))
  }
  // A 204 No Content response has no body to parse.
  if (response.status === 204) return undefined as T
  return (await response.json()) as T
}

async function del(path: string, body?: unknown): Promise<void> {
  const response = await fetch(path, {
    method: 'DELETE',
    headers: body === undefined ? undefined : { 'Content-Type': 'application/json' },
    body: body === undefined ? undefined : JSON.stringify(body),
  })
  if (!response.ok) {
    throw new ApiError(response.status, await describeError(response))
  }
}

async function describeError(response: Response): Promise<string> {
  try {
    const body: unknown = await response.json()
    if (
      typeof body === 'object' &&
      body !== null &&
      'error' in body &&
      typeof (body as { error: unknown }).error === 'object'
    ) {
      const error = (body as { error: { message?: unknown } }).error
      if (typeof error.message === 'string') return error.message
    }
  } catch {
    // Fall through to the generic message below.
  }
  return `request failed with status ${response.status}`
}

export const api = {
  health: (): Promise<HealthResponse> => getJson('/api/health'),

  positions: (): Promise<PositionRisk[]> => getJson('/api/positions'),
  addPosition: (spec: PositionSpec): Promise<PositionRisk> => postJson('/api/positions', spec),
  removePosition: (id: string): Promise<void> => del(`/api/positions/${encodeURIComponent(id)}`),

  greeks: (): Promise<GreeksResponse> => getJson('/api/greeks'),
  riskSummary: (): Promise<RiskSummaryResponse> => getJson('/api/risk-summary'),
  pnl: (): Promise<PnlReport> => getJson('/api/pnl'),
  varReport: (): Promise<VarReport> => getJson('/api/var'),
  alerts: (): Promise<AlertsResponse> => getJson('/api/alerts'),

  control: (request: ControlRequest): Promise<void> => postJson('/api/control', request),
  pause: (): Promise<void> => postJson('/api/control', { action: 'pause' }),
  resume: (): Promise<void> => postJson('/api/control', { action: 'resume' }),
  setSpeed: (speed: number): Promise<void> =>
    postJson('/api/control', { action: 'set_speed', speed }),

  /** `since` is the last `seq` already known; omit it for the full buffered history. */
  history: (since?: number): Promise<HistoryPoint[]> =>
    getJson(since === undefined ? '/api/history' : `/api/history?since=${since}`),

  storageUsage: (): Promise<StorageUsageResponse> => getJson('/api/storage'),
  sessions: (): Promise<SessionSummary[]> => getJson('/api/sessions'),
  sessionDetail: (id: string): Promise<SessionDetail> =>
    getJson(`/api/sessions/${encodeURIComponent(id)}`),
  sessionHistory: (id: string, maxPoints?: number): Promise<HistoryPoint[]> =>
    getJson(
      `/api/sessions/${encodeURIComponent(id)}/history${maxPoints === undefined ? '' : `?max_points=${maxPoints}`}`,
    ),
  sessionEvents: (id: string, type?: string): Promise<SessionEvent[]> =>
    getJson(
      `/api/sessions/${encodeURIComponent(id)}/events${type === undefined ? '' : `?type=${encodeURIComponent(type)}`}`,
    ),

  portfolioVersions: (): Promise<VersionSummary[]> => getJson('/api/portfolio/versions'),
  savePortfolioVersion: (request: SaveVersionRequest): Promise<SavedVersionResponse> =>
    postJson('/api/portfolio/versions', request),
  loadPortfolioVersion: (file: string): Promise<LoadedVersionResponse> =>
    postJson(`/api/portfolio/versions/${encodeURIComponent(file)}/load`),

  backtests: (): Promise<BacktestListItem[]> => getJson('/api/backtests'),
  archiveCoverage: (): Promise<ArchiveCoverage> => getJson('/api/backtests/archive-coverage'),
  backtestStatus: (id: string): Promise<BacktestStatus> =>
    getJson(`/api/backtests/${encodeURIComponent(id)}`),
  backtestResults: (id: string): Promise<BacktestResults> =>
    getJson(`/api/backtests/${encodeURIComponent(id)}/results`),
  /** Starts a run from the local archive. */
  createArchiveBacktest: (
    request: Omit<Extract<BacktestCreateRequest, { source: 'archive' }>, 'source'> = {},
  ): Promise<BacktestStatus> => postJson('/api/backtests', { source: 'archive', ...request }),
  /** Starts a run from an uploaded price CSV. A 400 carries the server's line-numbered message. */
  uploadBacktest: async (file: File): Promise<BacktestStatus> => {
    const form = new FormData()
    form.append('file', file)
    const response = await fetch('/api/backtests', { method: 'POST', body: form })
    if (!response.ok) {
      throw new ApiError(response.status, await describeError(response))
    }
    return (await response.json()) as BacktestStatus
  },
  backtestControl: (id: string, request: BacktestControlRequest): Promise<BacktestStatus> =>
    postJson(`/api/backtests/${encodeURIComponent(id)}/control`, request),
  /** Frees a run's memory on the server. Its saved files stay. */
  stopBacktest: (id: string): Promise<void> => del(`/api/backtests/${encodeURIComponent(id)}`),
  /** Deletes a run's saved files. `confirm` must repeat the run id. */
  deleteBacktestFiles: (id: string): Promise<void> =>
    del(`/api/backtests/${encodeURIComponent(id)}/files`, { confirm: id }),

  marketDataArchive: (): Promise<SymbolArchive[]> => getJson('/api/market-data/archive'),
}

/** Frees a run on the server as the page unloads. `fetch` with `keepalive` outlives the page;
 * `navigator.sendBeacon` is POST only, so it cannot send a DELETE. Errors are ignored: there
 * is no page left to show them on. */
export function releaseBacktestOnUnload(id: string): void {
  void fetch(`/api/backtests/${encodeURIComponent(id)}`, {
    method: 'DELETE',
    keepalive: true,
  }).catch(() => undefined)
}
