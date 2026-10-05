import { describe, expect, it } from 'vitest'

import type { KupiecTest } from '../api/generated/KupiecTest'
import { describeVarBacktest } from './varBacktest'

const base: KupiecTest = {
  n: 500,
  exceptions: 5,
  expected_exceptions: 5,
  lr: 0,
  p_value: 0.5,
  reject_5pct: false,
  low_power: false,
}

describe('describeVarBacktest', () => {
  it('says the model understates risk when it is rejected with more exceptions than expected', () => {
    const text = describeVarBacktest({
      ...base,
      n: 100,
      exceptions: 5,
      expected_exceptions: 1,
      p_value: 0.004,
      reject_5pct: true,
    })
    expect(text).toContain(
      '5 exceptions against 1.0 expected. p = 0.004, so the model understates risk.',
    )
  })

  it('says the model overstates risk when it is rejected with fewer exceptions than expected', () => {
    const text = describeVarBacktest({
      ...base,
      exceptions: 0,
      expected_exceptions: 5,
      p_value: 0.02,
      reject_5pct: true,
    })
    expect(text).toContain(
      '0 exceptions against 5.0 expected. p = 0.020, so the model overstates risk.',
    )
  })

  it('says the result is consistent when the model is not rejected', () => {
    const text = describeVarBacktest(base)
    expect(text).toContain('so the result is consistent with a correct model.')
    expect(text).not.toContain('little power')
  })

  it('warns about low power under 250 days and uses the singular for one exception', () => {
    const text = describeVarBacktest({
      ...base,
      n: 80,
      exceptions: 1,
      expected_exceptions: 0.8,
      p_value: 0.8,
    })
    expect(text).toContain('1 exception against 0.8 expected.')
    expect(text).toContain('With only 80 days the test has little power')
  })

  it('handles a run with no back-test and very small p-values', () => {
    expect(describeVarBacktest(null)).toContain('no days')
    expect(
      describeVarBacktest({ ...base, p_value: 0.00002, reject_5pct: true, exceptions: 9 }),
    ).toContain('p < 0.001')
  })
})
