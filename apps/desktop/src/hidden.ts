// What the hidden-space panel says, kept free of the DOM so it can be tested.
//
// The honesty rules this module owns (C29's falsifiers, each pinned by a test):
//
// 1. **No size per snapshot.** macOS reports which snapshots exist, not what each holds.
// 2. **A missing decomposition is not "none".** `other_volumes: null` is words, never 0.
// 3. **A negative residual is words, not a zero-width petal.** The note rides it; a
//    figure within rounding of zero may read as zero, but the words never disappear.
// 4. **Purgeable is "up to", never freed space and never a slice of the hidden figure** —
//    the system may free space the tree already counts (measured: purgeable can exceed
//    the whole residual on a real disk).
// 5. **A still-hidden figure that would be negative is the boundary-leak signal** —
//    words about the inconsistency, never a negative byte count or a silent zero.

import { formatBytes } from './format'
import type { HiddenSpaceReport } from './types'

export interface HiddenModel {
  headline: string
  /** The bytes the hidden petal stands for; null when there is no petal to draw. */
  petalBytes: number | null
  /** The hidden figure as text, or the words that replace it when it is negative. */
  hiddenFigure: string
  negativeNote: string | null
  otherVolumes: { name: string; bytes: string }[] | null
  otherVolumesLine: string
  snapshotLine: string
  purgeableLine: string | null
  stillHiddenLine: string | null
  protectedAreas: { path: string; note: string }[]
  fdaHint: string | null
  skippedLine: string | null
  measuredLine: string
}

/** A residual more negative than this fraction of `used` is reported in words alone. */
const NEGATIVE_TOLERANCE = 0.005

const FDA_HINT =
  'Some of this is protected by macOS privacy settings; grant Full Disk Access to measure it'

const NEGATIVE_FALLBACK_NOTE =
  'The volume changed more than the scan window — rescan for exact figures.'

const NEGATIVE_FIGURE_WORDS =
  'no figure — the disk changed during or after this scan; rescan'

export function hiddenModel(report: HiddenSpaceReport): HiddenModel {
  const hidden = report.hidden_bytes
  const negative = hidden < 0
  const withinTolerance = negative && -hidden <= report.used_bytes * NEGATIVE_TOLERANCE

  // A negative residual draws nothing (the backend already omitted the petal); the
  // words are the figure. Within rounding of zero the figure may read as zero — with
  // the note still visible, never swallowed.
  const petalBytes = negative ? null : hidden
  const hiddenFigure = negative ? (withinTolerance ? formatBytes(0) : NEGATIVE_FIGURE_WORDS) : formatBytes(hidden)
  const negativeNote = negative ? (report.negative_note ?? NEGATIVE_FALLBACK_NOTE) : null

  const snapshotLine =
    report.snapshot_count > 0
      ? `${report.snapshot_count} local snapshot${report.snapshot_count === 1 ? '' : 's'} · ${report.snapshot_note}`
      : report.snapshot_note

  const otherVolumes =
    report.other_volumes === null
      ? null
      : report.other_volumes.map((entry) => ({
          name: entry.name,
          bytes: formatBytes(entry.bytes),
        }))
  const otherVolumesLine =
    report.other_volumes === null
      ? 'Breakdown not available on this volume.'
      : `${report.other_volumes.length} other volume${report.other_volumes.length === 1 ? '' : 's'} in this container · ${formatBytes(
          report.other_volumes.reduce((sum, entry) => sum + entry.bytes, 0),
        )}`

  const purgeableLine =
    report.purgeable_bytes === null
      ? null
      : `macOS can free up to ${formatBytes(report.purgeable_bytes)} (${report.purgeable_note ?? 'purgeable: snapshots, caches and offloaded content'})`

  const stillHiddenLine =
    report.still_hidden_bytes === null
      ? null
      : report.still_hidden_bytes < 0
        ? 'The named components exceed the hidden figure — the boundary or the anchor is inconsistent; rescan.'
        : `Still hidden: ${formatBytes(report.still_hidden_bytes)} — container overhead, snapshot-held blocks and system areas the walk cannot name`

  const protectedAreas = report.restricted_roots.map((root) => ({
    path: root.path,
    note: root.note ?? 'not measurable by this app',
  }))

  return {
    headline: negative
      ? `Hidden space · ${NEGATIVE_FIGURE_WORDS}`
      : `Hidden space · ${formatBytes(hidden)} of ${formatBytes(report.used_bytes)} used`,
    petalBytes,
    hiddenFigure,
    negativeNote,
    otherVolumes,
    otherVolumesLine,
    snapshotLine,
    purgeableLine,
    stillHiddenLine,
    protectedAreas,
    fdaHint: report.restricted_roots.length > 0 ? FDA_HINT : null,
    skippedLine:
      report.skipped_mounts.length === 0
        ? null
        : `${report.skipped_mounts.length} mount${report.skipped_mounts.length === 1 ? '' : 's'} the scan did not enter (another volume's content): ${report.skipped_mounts.join(', ')}`,
    measuredLine: `Other volumes and purgeable measured at scan time (${new Date(
      report.measured_at * 1000,
    ).toISOString()})`,
  }
}
