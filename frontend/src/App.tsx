import { useEffect, type ReactElement } from 'react'
import { Route, Routes } from 'react-router-dom'

import { startStream } from './api/stream'
import { BacktestPage } from './components/backtest/BacktestPage'
import { Dashboard } from './components/Dashboard'
import { Header } from './components/Header'
import { ModeSwitch } from './components/ModeSwitch'
import { DataPage } from './components/storage/DataPage'
import { HistoryPage } from './components/storage/HistoryPage'
import { PortfoliosPage } from './components/storage/PortfoliosPage'

/** The app shell: the live header and mode switch, then the live dashboard at `/`, the
 * backtester at `/backtest`, and the storage pages at `/history`, `/portfolios` and `/data`. The live stream starts here, once, so the dashboard keeps
 * updating in the background while a backtest runs. */
export function App(): ReactElement {
  useEffect(() => {
    // `startStream` is a no-op after the first call, so React StrictMode's double
    // mount in development opens exactly one WebSocket connection (see
    // `src/api/stream.ts`'s doc comment).
    startStream()
  }, [])

  return (
    <div className="flex min-h-screen flex-col">
      <Header nav={<ModeSwitch />} />
      <Routes>
        <Route
          path="/backtest"
          element={
            <main className="flex-1 p-4">
              <BacktestPage />
            </main>
          }
        />
        <Route
          path="/history"
          element={
            <main className="flex-1 p-4">
              <HistoryPage />
            </main>
          }
        />
        <Route
          path="/portfolios"
          element={
            <main className="flex-1 p-4">
              <PortfoliosPage />
            </main>
          }
        />
        <Route
          path="/data"
          element={
            <main className="flex-1 p-4">
              <DataPage />
            </main>
          }
        />
        <Route
          path="*"
          element={
            <main className="grid flex-1 grid-cols-1 gap-4 p-4 lg:grid-cols-3">
              <Dashboard />
            </main>
          }
        />
      </Routes>
    </div>
  )
}
