import { useState, type FormEvent, type ReactElement } from 'react'

import type { InstrumentKind } from '../api/generated/InstrumentKind'
import { api, ApiError } from '../api/rest'
import {
  buildPositionSpec,
  validateAddPositionForm,
  type AddPositionFieldErrors,
  type AddPositionFormValues,
} from '../lib/positionForm'

const SPEEDS = [60, 120, 300, 600] as const

const EMPTY_FORM: AddPositionFormValues = {
  underlying: '',
  kind: 'call',
  strike: '',
  expiry: '',
  quantity: '',
}

function SpeedSelector({
  currentSpeed,
  disabled,
}: {
  currentSpeed: number | null
  disabled: boolean
}): ReactElement {
  const [pending, setPending] = useState<number | null>(null)

  const setSpeed = async (speed: number): Promise<void> => {
    setPending(speed)
    try {
      await api.setSpeed(speed)
    } finally {
      setPending(null)
    }
  }

  return (
    <div className="flex items-center gap-2" role="group" aria-label="Clock speed">
      <span className="text-xs uppercase tracking-wide text-neutral">Speed</span>
      {SPEEDS.map((speed) => (
        <button
          key={speed}
          type="button"
          disabled={disabled || pending !== null}
          aria-pressed={currentSpeed === speed}
          onClick={() => void setSpeed(speed)}
          className={`rounded border px-2 py-1 text-sm disabled:opacity-50 ${
            currentSpeed === speed
              ? 'border-up bg-up/20 text-up'
              : 'border-border hover:bg-surface'
          }`}
        >
          {speed}×
        </button>
      ))}
    </div>
  )
}

function AddPositionForm({ onAdded }: { onAdded?: () => void }): ReactElement {
  const [values, setValues] = useState<AddPositionFormValues>(EMPTY_FORM)
  const [fieldErrors, setFieldErrors] = useState<AddPositionFieldErrors>({})
  const [serverError, setServerError] = useState<string | null>(null)
  const [pending, setPending] = useState(false)

  const setField = <K extends keyof AddPositionFormValues>(key: K, value: AddPositionFormValues[K]): void => {
    setValues((v) => ({ ...v, [key]: value }))
  }

  const submit = async (e: FormEvent): Promise<void> => {
    e.preventDefault()
    const errors = validateAddPositionForm(values)
    setFieldErrors(errors)
    setServerError(null)
    if (Object.keys(errors).length > 0) return

    setPending(true)
    try {
      await api.addPosition(buildPositionSpec(values))
      setValues(EMPTY_FORM)
      onAdded?.()
    } catch (err) {
      setServerError(err instanceof ApiError ? err.message : 'failed to add position')
    } finally {
      setPending(false)
    }
  }

  const isOption = values.kind !== 'equity'

  return (
    <form onSubmit={(e) => void submit(e)} className="flex flex-col gap-2" aria-label="Add position">
      <div className="grid grid-cols-2 gap-2 sm:grid-cols-5">
        <label className="flex flex-col gap-1 text-xs text-neutral">
          Underlying
          <input
            value={values.underlying}
            onChange={(e) => setField('underlying', e.target.value)}
            className="rounded border border-border bg-surface px-2 py-1 text-sm text-inherit"
            placeholder="AAPL"
          />
          {fieldErrors.underlying && <span className="text-down">{fieldErrors.underlying}</span>}
        </label>
        <label className="flex flex-col gap-1 text-xs text-neutral">
          Kind
          <select
            value={values.kind}
            onChange={(e) => setField('kind', e.target.value as InstrumentKind)}
            className="rounded border border-border bg-surface px-2 py-1 text-sm text-inherit"
          >
            <option value="equity">Equity</option>
            <option value="call">Call</option>
            <option value="put">Put</option>
          </select>
        </label>
        <label className="flex flex-col gap-1 text-xs text-neutral">
          Strike
          <input
            value={values.strike}
            onChange={(e) => setField('strike', e.target.value)}
            disabled={!isOption}
            className="rounded border border-border bg-surface px-2 py-1 text-sm text-inherit disabled:opacity-50"
            placeholder="ATM+5%"
          />
          {fieldErrors.strike && <span className="text-down">{fieldErrors.strike}</span>}
        </label>
        <label className="flex flex-col gap-1 text-xs text-neutral">
          Expiry
          <input
            value={values.expiry}
            onChange={(e) => setField('expiry', e.target.value)}
            disabled={!isOption}
            className="rounded border border-border bg-surface px-2 py-1 text-sm text-inherit disabled:opacity-50"
            placeholder="+30d"
          />
          {fieldErrors.expiry && <span className="text-down">{fieldErrors.expiry}</span>}
        </label>
        <label className="flex flex-col gap-1 text-xs text-neutral">
          Quantity
          <input
            value={values.quantity}
            onChange={(e) => setField('quantity', e.target.value)}
            className="rounded border border-border bg-surface px-2 py-1 text-sm text-inherit"
            placeholder="10"
          />
          {fieldErrors.quantity && <span className="text-down">{fieldErrors.quantity}</span>}
        </label>
      </div>
      {serverError && (
        <p className="text-sm text-down" data-testid="add-position-server-error">
          {serverError}
        </p>
      )}
      <button
        type="submit"
        disabled={pending}
        className="self-start rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
      >
        {pending ? 'Adding…' : 'Add position'}
      </button>
    </form>
  )
}

/** The controls panel: pause/resume, a clock speed selector, and the add-position
 * form. Reads only its props, never the store directly. */
export function Controls({
  paused,
  clockSpeed,
  onPositionsChanged,
  className = '',
}: {
  paused: boolean | null
  clockSpeed: number | null
  onPositionsChanged?: () => void
  className?: string
}): ReactElement {
  const [pending, setPending] = useState(false)

  const togglePause = async (): Promise<void> => {
    setPending(true)
    try {
      await (paused ? api.resume() : api.pause())
    } finally {
      setPending(false)
    }
  }

  return (
    <section
      className={`flex flex-col gap-4 rounded-lg border border-border bg-surface-raised p-4 ${className}`}
      aria-label="Controls"
    >
      <h2 className="text-sm font-semibold uppercase tracking-wide text-neutral">Controls</h2>
      <div className="flex flex-wrap items-center gap-4">
        <button
          type="button"
          onClick={() => void togglePause()}
          disabled={paused === null || pending}
          className="rounded border border-border px-3 py-1 text-sm hover:bg-surface disabled:opacity-50"
        >
          {paused ? 'Resume' : 'Pause'}
        </button>
        <SpeedSelector currentSpeed={clockSpeed} disabled={paused === null} />
      </div>
      <AddPositionForm onAdded={onPositionsChanged} />
    </section>
  )
}
