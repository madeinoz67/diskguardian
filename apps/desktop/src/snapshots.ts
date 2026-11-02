// What the local-snapshots panel says, kept free of the DOM so it can be tested.
//
// Two honesty rules live here:
//
// 1. **No size per snapshot.** macOS reports which local snapshots exist, not what each
//    occupies — they share blocks with the live filesystem and with each other. Any number
//    beside a snapshot would be invented, so the rows carry timestamps and nothing else.
// 2. **A failed listing is not an empty one.** If `tmutil` could not be read, the panel
//    says so; "0 snapshots" would be a claim the app has not earned.

import type { PurgeReport, SnapshotReport } from './types'

export interface SnapshotRowModel {
  name: string
  /** `1 Nov 2026, 08:59` — the stamp made readable. */
  when: string
  newest: boolean
}

export interface SnapshotsModel {
  headline: string
  /** Set when the listing itself failed; the rows below are then not to be trusted. */
  error: string | null
  rows: SnapshotRowModel[]
  /** What to say when there is nothing to list and nothing went wrong. */
  empty: string | null
  canPurge: boolean
}

const MONTHS = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
]

/**
 * `2026-11-01-085943` → `1 Nov 2026, 08:59`.
 *
 * Anything that is not that exact shape is returned unchanged rather than guessed at: a
 * snapshot the app cannot read is better shown verbatim than shown wrongly.
 */
export function formatStamp(stamp: string): string {
  const match = /^(\d{4})-(\d{2})-(\d{2})-(\d{2})(\d{2})(\d{2})$/.exec(stamp)
  if (!match) return stamp
  const [, year, month, day, hour, minute] = match
  const index = Number(month) - 1
  const name = MONTHS[index]
  if (!name) return stamp
  return `${Number(day)} ${name} ${year}, ${hour}:${minute}`
}

export function snapshotsModel(report: SnapshotReport): SnapshotsModel {
  if (!report.supported) {
    return {
      headline: 'Local snapshots',
      error: null,
      rows: [],
      empty: 'Local Time Machine snapshots exist only on macOS.',
      canPurge: false,
    }
  }

  const error = report.error && report.error.length > 0 ? report.error : null
  const rows = report.snapshots.map((snapshot, index) => ({
    name: snapshot.name,
    when: formatStamp(snapshot.stamp),
    newest: index === report.snapshots.length - 1,
  }))

  const count = rows.length
  const headline =
    count === 0
      ? 'No local snapshots'
      : `${count} local snapshot${count === 1 ? '' : 's'} · macOS does not report how much each holds`

  return {
    headline,
    error,
    rows,
    empty: count === 0 && error === null ? 'Time Machine is not holding any local snapshots.' : null,
    canPurge: count > 0 && error === null,
  }
}

/** The line shown after a purge attempt: the backend's own words, never a paraphrase. */
export function purgeMessage(report: PurgeReport): string {
  const trimmed = report.message.trim()
  return trimmed.length > 0 ? trimmed : 'The purge finished without a message from macOS.'
}
