import type { KupiecTest } from '../api/generated/KupiecTest'

/** Below this many tested days the Kupiec test has little power at 99 %. */
export const LOW_POWER_DAYS = 250

/** Formats a p-value to three decimal places, with `< 0.001` for the very small ones. */
export function formatPValue(p: number): string {
  return p < 0.001 ? 'p < 0.001' : `p = ${p.toFixed(3)}`
}

/** The VaR back-test in plain words.
 *
 * Rejecting at 5 % with more exceptions than expected means the model understates risk;
 * rejecting with fewer means it overstates risk; otherwise the result is consistent with a
 * correct model. Under {@link LOW_POWER_DAYS} days a sentence about the test's power is added,
 * because a pass then says little. `null` (no days replayed) gets its own sentence. */
export function describeVarBacktest(test: KupiecTest | null): string {
  if (test === null) return 'No VaR back-test: no days have been replayed yet.'
  const noun = test.exceptions === 1 ? 'exception' : 'exceptions'
  const lead = `${test.exceptions} ${noun} against ${test.expected_exceptions.toFixed(1)} expected. ${formatPValue(test.p_value)}`
  let verdict: string
  if (test.reject_5pct) {
    verdict =
      test.exceptions > test.expected_exceptions
        ? 'so the model understates risk.'
        : 'so the model overstates risk.'
  } else {
    verdict = 'so the result is consistent with a correct model.'
  }
  const power =
    test.n < LOW_POWER_DAYS
      ? ` With only ${test.n} days the test has little power at this sample size, so a pass is weak evidence.`
      : ''
  return `${lead}, ${verdict}${power}`
}
