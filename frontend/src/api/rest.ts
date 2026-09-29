/**
 * Typed wrappers for every REST route the backend exposes (`crates/server/src/routes.rs`
 * and `crates/server/src/storage_routes.rs`). Every request and response type here is
 * generated from the Rust types by `ts-rs` (see `src/api/generated/`), so a route's
 * shape can never silently drift between the backend and this file.
 */
import type { AlertsResponse } from './generated/AlertsResponse'
import type { ArchiveFileInfo as _ArchiveFileInfo } from './generated/ArchiveFileInfo'
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

async function del(path: string): Promise<void> {
  const response = await fetch(path, { method: 'DELETE' })
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

  marketDataArchive: (): Promise<SymbolArchive[]> => getJson('/api/market-data/archive'),
}
