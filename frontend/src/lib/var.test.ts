import { describe, expect, it } from 'vitest'

import type { VarMethod } from '../api/generated/VarMethod'
import type { VarMethodReport } from '../api/generated/VarMethodReport'
import type { VarReport } from '../api/generated/VarReport'
import { VAR_METHOD_LABEL, headlineReport, headlineVar99 } from './var'

function methodReport(var99: number): VarMethodReport {
  return {
    var_95: var99 * 0.8,
    var_99: var99,
    es_97_5: var99 * 1.2,
    scenario_count: 10_000,
    window_start: null,
    window_end: null,
    compute_micros: 100,
  }
}

function report(headlineMethod: VarMethod): VarReport {
  return {
    headline_method: headlineMethod,
    delta_normal: methodReport(100),
    delta_gamma_mc: methodReport(200),
    historical: methodReport(300),
    total_compute_micros: 500,
    freshly_computed: true,
    recompute_every_ticks: 1,
  }
}

describe('headlineReport', () => {
  it.each([
    ['delta_normal', 100],
    ['delta_gamma_mc', 200],
    ['historical', 300],
  ] as const)('picks the %s sub-report when it is the headline method', (method, expectedVar99) => {
    expect(headlineReport(report(method)).var_99).toBe(expectedVar99)
  })
})

describe('headlineVar99', () => {
  it("is the headline sub-report's var_99", () => {
    expect(headlineVar99(report('delta_gamma_mc'))).toBe(200)
  })
})

describe('VAR_METHOD_LABEL', () => {
  it('has a label for every VarMethod', () => {
    const methods: VarMethod[] = ['delta_normal', 'delta_gamma_mc', 'historical']
    for (const method of methods) {
      expect(VAR_METHOD_LABEL[method]).toBeTruthy()
    }
  })
})
