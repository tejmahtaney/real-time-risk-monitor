import { useEffect, useState, type ReactElement } from 'react'
import { Line, LineChart, YAxis } from 'recharts'

import type { ArchiveFileInfo } from '../../api/generated/ArchiveFileInfo'
import { api } from '../../api/rest'
import { CHART_LINE } from '../../lib/chartTheme'
import { formatSimTime } from '../../lib/format'
import { formatBytes } from '../../lib/usage'
import { UsageBar } from './UsageBar'
import { useLoad } from './useLoad'

/** The closes of one archived download as a small line, fetched from its own route. The chart
 * has a fixed size, so it needs no layout pass; the y-axis scales to the data. */
function CloseSparkline({ symbol, file }: { symbol: string; file: string }): ReactElement {
  const [closes, setCloses] = useState<number[] | null>(null)
  useEffect(() => {
    let cancelled = false
    api
      .archiveFileCloses(symbol, file)
      .then((rows) => {
        if (!cancelled) setCloses(rows.map((r) => r.close))
      })
      .catch(() => {
        if (!cancelled) setCloses([])
      })
    return () => {
      cancelled = true
    }
  }, [symbol, file])
  if (closes === null) return <span className="text-xs text-neutral">…</span>
  if (closes.length === 0) return <span className="text-xs text-neutral">—</span>
  return (
    <LineChart width={120} height={28} data={closes.map((close) => ({ close }))}>
      <YAxis hide domain={['dataMin', 'dataMax']} />
      <Line type="monotone" dataKey="close" stroke={CHART_LINE} strokeWidth={1.5} dot={false} isAnimationActive={false} />
    </LineChart>
  )
}

function FileRow({ symbol, file }: { symbol: string; file: ArchiveFileInfo }): ReactElement {
  return (
    <tr data-testid={`archive-file-${file.file}`}>
      <td className="whitespace-nowrap px-2 py-1">{formatSimTime(file.fetched_at, { seconds: false, withDate: true })}</td>
      <td className="px-2 py-1">{file.source}</td>
      <td className="px-2 py-1 text-right tabular-nums">{file.dates}</td>
      <td className="px-2 py-1 text-right tabular-nums text-neutral">{formatBytes(file.size_bytes)}</td>
      <td className="px-2 py-1">
        <CloseSparkline symbol={symbol} file={file.file} />
      </td>
    </tr>
  )
}

/** The Data page: the market data archive per symbol, what the merged archive covers (the
 * history the backtester can use), and storage usage by folder. */
export function DataPage(): ReactElement {
  const archive = useLoad(() => api.marketDataArchive(), [])
  const coverage = useLoad(() => api.archiveCoverage(), [])
  const usage = useLoad(() => api.storageUsage(), [])

  return (
    <div className="flex flex-col gap-4">
      <h2 className="text-base font-semibold">Data</h2>
      {usage.error && <p role="alert" className="text-sm text-down">{usage.error}</p>}
      {usage.data && <UsageBar usage={usage.data} />}

      <section
        aria-label="Merged archive coverage"
        className="flex flex-col gap-2 rounded-lg border border-border bg-surface-raised p-4"
      >
        <h3 className="text-sm font-semibold uppercase tracking-wide text-neutral">Merged archive coverage</h3>
        {coverage.error && <p role="alert" className="text-sm text-down">{coverage.error}</p>}
        {coverage.data && (
          <>
            <p className="text-sm text-neutral" data-testid="aligned-summary">
              {coverage.data.aligned_dates > 0
                ? `${coverage.data.aligned_dates} dates have a price for every portfolio symbol (${coverage.data.first_date} to ${coverage.data.last_date}). That is the history a backtest can replay.`
                : 'No dates have a price for every portfolio symbol, so a backtest cannot start yet.'}
            </p>
            {coverage.data.missing_symbols.length > 0 && (
              <p className="text-sm text-warning">
                No archived history for {coverage.data.missing_symbols.join(', ')}.
              </p>
            )}
            <table className="w-full max-w-xl border-collapse text-sm tabular-nums">
              <thead>
                <tr className="text-left text-neutral">
                  <th className="px-2 py-1 font-medium">Symbol</th>
                  <th className="px-2 py-1 font-medium">First date</th>
                  <th className="px-2 py-1 font-medium">Last date</th>
                  <th className="px-2 py-1 text-right font-medium">Dates</th>
                </tr>
              </thead>
              <tbody>
                {coverage.data.symbols.map((s) => (
                  <tr key={s.symbol} data-testid={`coverage-${s.symbol}`}>
                    <td className="px-2 py-1">{s.symbol}</td>
                    <td className="px-2 py-1">{s.first_date ?? '—'}</td>
                    <td className="px-2 py-1">{s.last_date ?? '—'}</td>
                    <td className="px-2 py-1 text-right">{s.dates}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </>
        )}
      </section>

      <section
        aria-label="Market data archive"
        className="flex flex-col gap-3 rounded-lg border border-border bg-surface-raised p-4"
      >
        <h3 className="text-sm font-semibold uppercase tracking-wide text-neutral">Market data archive</h3>
        {archive.error && <p role="alert" className="text-sm text-down">{archive.error}</p>}
        {archive.data?.length === 0 && <p className="text-sm text-neutral">Nothing archived yet.</p>}
        {archive.data?.map((symbol) => (
          <div key={symbol.symbol} className="flex flex-col gap-1">
            <h4 className="text-sm font-medium">
              {symbol.symbol} <span className="text-xs text-neutral">{symbol.files.length} downloads</span>
            </h4>
            <div className="overflow-x-auto">
              <table className="w-full min-w-max border-collapse text-sm">
                <thead>
                  <tr className="text-left text-neutral">
                    <th className="px-2 py-1 font-medium">Fetched</th>
                    <th className="px-2 py-1 font-medium">Source</th>
                    <th className="px-2 py-1 text-right font-medium">Dates</th>
                    <th className="px-2 py-1 text-right font-medium">Size</th>
                    <th className="px-2 py-1 font-medium">Closes</th>
                  </tr>
                </thead>
                <tbody>
                  {symbol.files.map((file) => (
                    <FileRow key={file.file} symbol={symbol.symbol} file={file} />
                  ))}
                </tbody>
              </table>
            </div>
          </div>
        ))}
      </section>
    </div>
  )
}
