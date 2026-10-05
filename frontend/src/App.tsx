import { useEffect, type ReactElement } from 'react'
import { Route, Routes } from 'react-router-dom'

import { startStream } from './api/stream'
import { BacktestPage } from './components/backtest/BacktestPage'
import { Dashboard } from './components/Dashboard'
import { Header } from './components/Header'
import { ModeSwitch } from './components/ModeSwitch'

/** The app shell: the live header and mode switch, then the live dashboard at `/` or the
 * backtester at `/backtest`. The live stream starts here, once, so the dashboard keeps
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
