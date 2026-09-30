/** Shared test fixtures for positions-panel tests. */
import type { PositionRisk } from '../api/generated/PositionRisk'

const ZERO_GREEKS = { price: 0, delta: 0, gamma: 0, vega: 0, theta: 0, rho: 0, vanna: 0, volga: 0 }
const ZERO_REPORTING = {
  share_delta: 0,
  dollar_delta: 0,
  share_gamma: 0,
  dollar_gamma_1pct: 0,
  vega: 0,
  theta: 0,
  rho: 0,
}

export function position(overrides: Partial<PositionRisk> = {}): PositionRisk {
  return {
    id: 'p1',
    description: 'AAPL 2026-10-16 C 230',
    underlying: 'AAPL',
    kind: 'call',
    quantity: 1,
    multiplier: 100,
    spot: 230,
    vol: 0.3,
    time_to_expiry: 0.1,
    mark: 5,
    market_value: 500,
    entry_price: 4,
    unrealised_pnl: 100,
    realised_pnl: 0,
    adjusted: false,
    greeks: { ...ZERO_GREEKS },
    reporting: { ...ZERO_REPORTING },
    status: { state: 'active' },
    ...overrides,
  }
}
