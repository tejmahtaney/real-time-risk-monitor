/**
 * Records a scripted walk of about 90 seconds through the running dashboard as a Playwright video
 * (1,440 × 900, WebM). `make demo` runs this and then converts the video to
 * `docs/demo.gif` and `docs/demo.mp4` with ffmpeg.
 *
 * The flow:
 *   1. The live dashboard updating for a few seconds.
 *   2. Hover the gamma tile and the P&L headline.
 *   3. Add a large short straddle (a short call and a short put) through the form, so one or
 *      two limits breach and the alert pane reacts.
 *   4. Speed up to 600× and let a session close, until the simulated day rolls over.
 *   5. Open the backtester, use the archived history, run to the end and show the results.
 *   6. Open the History page and show the live session just recorded, with its daily P&L.
 *
 * Requires both halves of the app already running (`make dev`, or `cargo run -p server` and
 * `npm run dev`), like `screenshot.ts`. The clock should be at its default speed when it
 * starts. Run it against a throwaway storage folder (`RRM_STORAGE_DIR=$(mktemp -d)`), since it
 * adds positions and starts a session that is saved to whatever folder the server uses.
 *
 * Environment:
 *   RRM_BASE_URL         where the dashboard is served (default http://localhost:5173)
 *   RRM_DEMO_OUT         where the WebM is written (default `demo.webm` in a fresh temp folder)
 *   RRM_DEMO_QUANTITY    contracts short on each leg of the straddle (default 15)
 *
 * Usage:
 *   npm run demo
 */
import { copyFile, mkdir, mkdtemp, rm } from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

import { chromium, type Locator, type Page } from 'playwright'

const BASE_URL = process.env.RRM_BASE_URL ?? 'http://localhost:5173'
const QUANTITY = process.env.RRM_DEMO_QUANTITY ?? '15'
const VIEWPORT = { width: 1440, height: 900 }
const STEP_TIMEOUT_MS = 20_000
const BACKTEST_TIMEOUT_MS = 120_000
/** A 6.5-hour session at 600× lasts about 39 seconds. The step ends as soon as the simulated
 * date changes, and this is only the most it waits for that. */
const SESSION_CLOSE_MS = 50_000

/** Waits for the dashboard's "waiting" placeholders to clear, as `screenshot.ts` does. */
async function waitForLiveData(page: Page): Promise<void> {
  try {
    await page.waitForFunction(
      () =>
        !document.body.innerText.includes('Waiting for data') &&
        !document.body.innerText.includes('Waiting for history'),
      { timeout: STEP_TIMEOUT_MS },
    )
  } catch {
    console.warn('demo: the dashboard still shows a "waiting" placeholder; is the backend running?')
  }
}

/** Runs one step of the flow. A step that fails is logged and skipped, so one changed label
 * does not lose the whole recording. */
async function step(name: string, run: () => Promise<void>): Promise<void> {
  console.log(`demo: ${name}`)
  try {
    await run()
  } catch (err) {
    console.warn(`demo: step "${name}" failed and was skipped:`, err)
  }
}

/** Scrolls so `first` (and everything down to the bottom of `last`, when given) is fully in
 * the viewport, with the sticky header clear of it, then waits for the smooth scroll to settle.
 * Called before each pause, so the panel being shown is never half off screen. */
async function scrollIntoView(first: Locator, last: Locator = first): Promise<void> {
  await first.evaluate((el) => {
    ;(el as HTMLElement).style.scrollMarginTop = '72px'
    el.scrollIntoView({ block: 'start', behavior: 'instant' })
  })
  // If the block is shorter than the viewport, centre it rather than hugging the top edge.
  const [top, bottom] = await Promise.all([
    first.boundingBox(),
    last.boundingBox(),
  ])
  if (top !== null && bottom !== null) {
    const height = bottom.y + bottom.height - top.y
    const spare = VIEWPORT.height - 72 - height
    if (spare > 0) await first.page().mouse.wheel(0, -Math.min(spare / 2, 40))
  }
  await first.page().waitForTimeout(400)
}

/** Adds one short option leg through the Add position form, typing at a readable pace. */
async function addShortLeg(page: Page, kind: 'call' | 'put'): Promise<void> {
  const form = page.getByRole('form', { name: 'Add position' })
  await form.getByLabel('Underlying').fill('')
  await form.getByLabel('Underlying').pressSequentially('SPY', { delay: 50 })
  await form.getByLabel('Kind').selectOption(kind)
  await form.getByLabel('Strike').fill('')
  await form.getByLabel('Strike').pressSequentially('ATM', { delay: 50 })
  await form.getByLabel('Expiry').fill('')
  await form.getByLabel('Expiry').pressSequentially('+30d', { delay: 50 })
  await form.getByLabel('Quantity').fill('')
  await form.getByLabel('Quantity').pressSequentially(`-${QUANTITY}`, { delay: 50 })
  await page.waitForTimeout(300)
  await form.getByRole('button', { name: 'Add position' }).click()
  await page.waitForTimeout(800)
}

async function main(): Promise<void> {
  const videoDir = await mkdtemp(path.join(os.tmpdir(), 'rrm-demo-'))
  const output = process.env.RRM_DEMO_OUT ?? path.join(videoDir, 'demo.webm')
  await mkdir(path.dirname(output), { recursive: true })

  const browser = await chromium.launch()
  try {
    const context = await browser.newContext({
      viewport: VIEWPORT,
      recordVideo: { dir: videoDir, size: VIEWPORT },
    })
    const page = await context.newPage()
    await page.goto(BASE_URL, { waitUntil: 'networkidle' })
    await waitForLiveData(page)

    await step('1. live dashboard for a few seconds', async () => {
      // The summary cards and the cumulative chart together, before the pause.
      await scrollIntoView(page.getByTestId('summary-cards'), page.getByRole('region', { name: 'Cumulative P&L' }))
      await page.waitForTimeout(5_000)
    })

    await step('2. hover the gamma tile and the P&L headline', async () => {
      await page.getByTestId('tile-dollar_gamma_1pct').hover()
      await page.waitForTimeout(2_000)
      await page.getByTestId('pnl-headline').hover()
      await page.waitForTimeout(2_000)
    })

    await step('3. add a large short straddle', async () => {
      await addShortLeg(page, 'call')
      await addShortLeg(page, 'put')
      await page
        .getByTestId('active-alert')
        .first()
        .waitFor({ state: 'visible', timeout: STEP_TIMEOUT_MS })
      await scrollIntoView(page.getByTestId('alerts-pane'))
      await page.getByTestId('alerts-pane').hover()
      await page.waitForTimeout(3_500)
    })

    await step('4. speed up to 600× and let a session close', async () => {
      await page.evaluate(() => window.scrollTo({ top: 0, behavior: 'instant' }))
      const simDate = (): Promise<string> =>
        page.getByTestId('sim-time').innerText().then((text) => /[A-Za-z]{3} \d{1,2}/.exec(text)?.[0] ?? text)
      const before = await simDate()
      await page.getByRole('group', { name: 'Clock speed' }).getByRole('button', { name: '600×' }).click()
      // Ends at the day roll rather than after a fixed wait, so there is no idle tail.
      await page.waitForFunction(
        (start) => {
          const text = document.querySelector('[data-testid="sim-time"]')?.textContent ?? ''
          return (/[A-Za-z]{3} \d{1,2}/.exec(text)?.[0] ?? start) !== start
        },
        before,
        { timeout: SESSION_CLOSE_MS },
      )
      await page.waitForTimeout(1_500)
    })

    await step('5. backtest on the archived history', async () => {
      await page.getByRole('navigation', { name: 'Pages' }).getByRole('link', { name: 'Backtest' }).click()
      const start = page.getByRole('button', { name: 'Use archived real history' })
      await start.waitFor({ state: 'visible', timeout: STEP_TIMEOUT_MS })
      await page.waitForFunction(() => !document.body.innerText.includes('Reading the archive'), {
        timeout: STEP_TIMEOUT_MS,
      })
      await page.waitForTimeout(500)
      await start.click()
      await page.getByRole('button', { name: 'Run to end' }).click({ timeout: STEP_TIMEOUT_MS })
      await page.getByTestId('results-view').waitFor({ state: 'visible', timeout: BACKTEST_TIMEOUT_MS })
      await page.waitForTimeout(6_000)
    })

    await step('6. History: the live session just recorded', async () => {
      await page.getByRole('navigation', { name: 'Pages' }).getByRole('link', { name: 'History' }).click()
      await page.getByRole('table', { name: 'Sessions' }).waitFor({ timeout: STEP_TIMEOUT_MS })
      await page.waitForTimeout(1_000)
      // Rows are newest first, so the first Open button is the session just recorded.
      await page.getByRole('button', { name: 'Open' }).first().click()
      await page.getByTestId('session-detail').waitFor({ timeout: STEP_TIMEOUT_MS })
      // The daily P&L table fully in view before the pause.
      await scrollIntoView(page.getByRole('region', { name: 'Daily P&L' }))
      await page.waitForTimeout(5_000)
    })

    // The video is only finalised once the page, then the context, are closed.
    const video = page.video()
    await page.close()
    await context.close()
    if (video === null) {
      throw new Error('Playwright recorded no video')
    }
    await copyFile(await video.path(), output)
    console.log(`demo: saved ${output}`)
  } finally {
    await browser.close()
    await rm(videoDir, { recursive: true, force: true })
  }
}

main().catch((err: unknown) => {
  console.error(err)
  process.exitCode = 1
})
