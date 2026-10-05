/**
 * Opens the running dashboard and saves full-page screenshots at 1,280 and
 * 1,920 pixels wide into `docs/screenshots/`, then starts a backtest on the archived
 * history, runs it to the end and saves the results view as `backtest-results-<width>.png`, then saves
 * the storage pages (`history-`, `session-detail-`, `portfolios-`, `data-` and `backtest-compare-<width>.png`),
 * for visually checking a part's work against `docs/ROADMAP.md`'s feature list. Part 24 reuses this script
 * unchanged for the demo recording, so it takes no part-specific arguments.
 *
 * Requires both halves of the app already running (`make dev`, or
 * `cargo run -p server` and `npm run dev` in separate terminals) — this script
 * only drives a browser against whatever is already serving `RRM_BASE_URL`
 * (default `http://localhost:5173`), it does not start either process itself.
 *
 * Usage:
 *   npm run screenshot
 *   RRM_BASE_URL=http://localhost:4173 npm run screenshot   # e.g. against `vite preview`
 */
import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

import { chromium, type Page } from 'playwright'

const __dirname = path.dirname(fileURLToPath(import.meta.url))
// frontend/scripts/screenshot.ts -> risk-monitor/docs/screenshots
const OUTPUT_DIR = path.resolve(__dirname, '..', '..', 'docs', 'screenshots')
const BASE_URL = process.env.RRM_BASE_URL ?? 'http://localhost:5173'
const WIDTHS = [1280, 1920]
const VIEWPORT_HEIGHT = 1080
const READY_TIMEOUT_MS = 20_000
const BACKTEST_TIMEOUT_MS = 120_000

/** Waits for the dashboard's own "waiting for data" / "waiting for history"
 * placeholders (see `RiskTiles.tsx`, `GreeksCharts.tsx`) to clear, so the
 * screenshot shows real numbers and real charts rather than empty panels. Logs a
 * warning and screenshots anyway on timeout, rather than hanging indefinitely if
 * the backend is not actually running. */
async function waitForLiveData(page: Page): Promise<void> {
  try {
    await page.waitForFunction(
      () =>
        !document.body.innerText.includes('Waiting for data') &&
        !document.body.innerText.includes('Waiting for history'),
      { timeout: READY_TIMEOUT_MS },
    )
  } catch {
    console.warn(
      `screenshot: still showing a "waiting" placeholder after ${READY_TIMEOUT_MS}ms — ` +
        'is `cargo run -p server` running alongside the frontend dev server? Screenshotting anyway.',
    )
  }
}

/** Starts a run on the archived history, runs it to the end and waits for the results view.
 * Needs at least one archived download (the app archives every fetch it makes). Logs a warning
 * and screenshots whatever is showing if the run does not finish in time. */
async function captureBacktestResults(page: Page, width: number): Promise<void> {
  await page.goto(`${BASE_URL}/backtest`, { waitUntil: 'networkidle' })
  const start = page.getByRole('button', { name: 'Use archived real history' })
  try {
    await start.waitFor({ state: 'visible', timeout: READY_TIMEOUT_MS })
    await page.waitForFunction(() => !document.body.innerText.includes('Reading the archive'), {
      timeout: READY_TIMEOUT_MS,
    })
    await start.click({ timeout: READY_TIMEOUT_MS })
    await page.getByRole('button', { name: 'Run to end' }).click({ timeout: READY_TIMEOUT_MS })
    await page
      .getByTestId('results-view')
      .waitFor({ state: 'visible', timeout: BACKTEST_TIMEOUT_MS })
    // Let the charts finish laying out before the picture is taken.
    await page.waitForTimeout(1_000)
  } catch {
    console.warn(
      'screenshot: the backtest did not reach its results view; screenshotting what is showing.',
    )
  }
  const outputPath = path.join(OUTPUT_DIR, `backtest-results-${width}.png`)
  await page.screenshot({ path: outputPath, fullPage: true })
  console.log(`screenshot: saved ${outputPath}`)
}

/** Saves a full-page screenshot as `<name>-<width>.png` after the charts have had a moment to lay out. */
async function save(page: Page, name: string, width: number): Promise<void> {
  await page.waitForTimeout(1_000)
  const outputPath = path.join(OUTPUT_DIR, `${name}-${width}.png`)
  await page.screenshot({ path: outputPath, fullPage: true })
  console.log(`screenshot: saved ${outputPath}`)
}

/** The History page, then the first session that has simulated days opened in detail. */
async function captureHistory(page: Page, width: number): Promise<void> {
  await page.goto(`${BASE_URL}/history`, { waitUntil: 'networkidle' })
  try {
    await page.getByRole('table', { name: 'Sessions' }).waitFor({ timeout: READY_TIMEOUT_MS })
    await save(page, 'history', width)
    // Rows are newest first; open the first one whose simulated-days cell is not zero.
    const rows = page.getByRole('row').filter({ has: page.getByRole('button', { name: 'Open' }) })
    const count = await rows.count()
    let target = rows.first()
    for (let i = 0; i < count; i += 1) {
      const days = await rows.nth(i).getByRole('cell').nth(3).innerText()
      if (days.trim() !== '0') {
        target = rows.nth(i)
        break
      }
    }
    await target.getByRole('button', { name: 'Open' }).click()
    await page.getByTestId('session-detail').waitFor({ timeout: READY_TIMEOUT_MS })
    await page.waitForFunction(() => !document.body.innerText.includes('Loading…'), {
      timeout: READY_TIMEOUT_MS,
    })
  } catch {
    console.warn('screenshot: the History page did not show a session; screenshotting what is showing.')
  }
  await save(page, 'session-detail', width)
}

/** The Portfolios page, and the Data page. */
async function captureStoragePages(page: Page, width: number): Promise<void> {
  await page.goto(`${BASE_URL}/portfolios`, { waitUntil: 'networkidle' })
  try {
    await page.getByRole('region', { name: 'Saved versions' }).getByRole('listitem').first().waitFor({
      timeout: READY_TIMEOUT_MS,
    })
    // Open the load confirmation so the diff is in the picture. Nothing is loaded.
    await page.getByRole('button', { name: 'Load…' }).first().click()
    await page.getByTestId('load-confirm').waitFor({ timeout: READY_TIMEOUT_MS })
  } catch {
    console.warn('screenshot: the Portfolios page showed no versions; screenshotting what is showing.')
  }
  await save(page, 'portfolios', width)

  await page.goto(`${BASE_URL}/data`, { waitUntil: 'networkidle' })
  try {
    await page.getByTestId('aligned-summary').waitFor({ timeout: READY_TIMEOUT_MS })
  } catch {
    console.warn('screenshot: the Data page did not finish loading; screenshotting what is showing.')
  }
  await save(page, 'data', width)
}

/** Picks the first two finished saved runs on the Backtest page and opens their comparison. */
async function captureBacktestCompare(page: Page, width: number): Promise<void> {
  await page.goto(`${BASE_URL}/backtest`, { waitUntil: 'networkidle' })
  try {
    const boxes = page.getByRole('checkbox', { name: /^Compare / }).and(page.locator(':enabled'))
    await boxes.first().waitFor({ timeout: READY_TIMEOUT_MS })
    await boxes.nth(0).check()
    await boxes.nth(1).check()
    await page.getByRole('button', { name: /^Compare selected/ }).click()
    await page.getByTestId('compare-runs').waitFor({ timeout: READY_TIMEOUT_MS })
  } catch {
    console.warn('screenshot: fewer than two finished saved runs; screenshotting what is showing.')
  }
  await save(page, 'backtest-compare', width)
}

async function main(): Promise<void> {
  await mkdir(OUTPUT_DIR, { recursive: true })
  const browser = await chromium.launch()
  try {
    for (const width of WIDTHS) {
      const page = await browser.newPage({ viewport: { width, height: VIEWPORT_HEIGHT } })
      await page.goto(BASE_URL, { waitUntil: 'networkidle' })
      await waitForLiveData(page)
      const outputPath = path.join(OUTPUT_DIR, `dashboard-${width}.png`)
      await page.screenshot({ path: outputPath, fullPage: true })
      console.log(`screenshot: saved ${outputPath}`)
      await captureBacktestResults(page, width)
      await captureHistory(page, width)
      await captureStoragePages(page, width)
      await captureBacktestCompare(page, width)
      await page.close()
    }
  } finally {
    await browser.close()
  }
}

main().catch((err: unknown) => {
  console.error(err)
  process.exitCode = 1
})
