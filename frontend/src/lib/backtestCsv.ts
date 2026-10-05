import type { DailyRow } from '../api/generated/DailyRow'

/** The daily table's columns, in file order. */
export const DAILY_COLUMNS: ReadonlyArray<keyof DailyRow> = [
  'date',
  'delta',
  'gamma',
  'vega',
  'theta',
  'rho',
  'higher_order',
  'dividends',
  'residual',
  'actual',
  'cumulative_pnl',
  'var_99',
  'exception',
  'breaches',
]

/** The daily table as CSV text: a header line, then one line per date, with every number
 * written in full so the file reads back to the same values. */
export function dailyRowsToCsv(rows: DailyRow[]): string {
  const lines = [DAILY_COLUMNS.join(',')]
  for (const row of rows) {
    lines.push(DAILY_COLUMNS.map((column) => String(row[column])).join(','))
  }
  return `${lines.join('\n')}\n`
}

/** Offers `text` to the browser as a file download. */
export function downloadText(filename: string, text: string): void {
  const url = URL.createObjectURL(new Blob([text], { type: 'text/csv;charset=utf-8' }))
  const link = document.createElement('a')
  link.href = url
  link.download = filename
  document.body.appendChild(link)
  link.click()
  link.remove()
  URL.revokeObjectURL(url)
}
