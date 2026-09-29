/**
 * Opens the running dashboard and saves full-page screenshots at 1,280 and
 * 1,920 pixels wide into `docs/screenshots/`, for visually checking a part's
 * work against `docs/ROADMAP.md`'s feature list. Part 24 reuses this script
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
