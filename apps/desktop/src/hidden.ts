// What the hidden-space panel says, kept free of the DOM so it can be tested.
//
// The honesty rules this module owns (C29's falsifiers, each pinned by a test):
//
// 1. **No decomposition category renders at all while the breakdown is held.** Not
//    "0 other volumes", not an empty section: the backend's held note is the section,
//    passed through verbatim, so an unmeasured category can never be read as an empty
//    one. (Held 2026-11-03: the acceptance run was underpowered pending the
//    external-volume cross-check.)
// 2. **A negative residual is words, not a zero-width petal.** The note rides it; a
//    figure within rounding of zero may read as zero, but the words never disappear.
// 3. **Purgeable is "up to", never freed space and never a slice of the hidden figure** —
//    the system may free space the tree already counts (measured: purgeable can exceed
//    the whole residual on a real disk).
// 4. **The scan-time line claims only what was measured at scan time** — the purgeable
//    annotation. With no purgeable figure there is no line, because a "measured at"
//    stamp over an unmeasured panel would be a claim.

import { formatBytes } from './format'
import type { HiddenSpaceReport } from './types'

export interface HiddenModel {
  headline: string
  /** The bytes the hidden petal stands for; null when there is no petal to draw. */
  petalBytes: number | null
  /** The hidden figure as text, or the words that replace it when it is negative. */
  hiddenFigure: string
  negativeNote: string | null
  /** The decomposition's held state, in the backend's own words, verbatim. */
  heldLine: string
  purgeableLine: string | null
  skippedLine: string | null
  /** "Purgeable measured at scan time"; null when nothing was measured at scan time. */
  measuredLine: string | null
}

/** A residual more negative than this fraction of `used` is reported in words alone. */
const NEGATIVE_TOLERANCE = 0.005

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

  const purgeableLine =
    report.purgeable_bytes === null
      ? null
      : `macOS can free up to ${formatBytes(report.purgeable_bytes)} (${report.purgeable_note ?? 'purgeable: snapshots, caches and offloaded content'})`

  return {
    headline: negative
      ? `Hidden space · ${NEGATIVE_FIGURE_WORDS}`
      : `Hidden space · ${formatBytes(hidden)} of ${formatBytes(report.used_bytes)} used`,
    petalBytes,
    hiddenFigure,
    negativeNote,
    heldLine: report.decomposition_held_note,
    purgeableLine,
    skippedLine:
      report.skipped_mounts.length === 0
        ? null
        : `${report.skipped_mounts.length} mount${report.skipped_mounts.length === 1 ? '' : 's'} the scan did not enter (another volume's content): ${report.skipped_mounts.join(', ')}`,
    measuredLine:
      report.purgeable_bytes === null
        ? null
        : `Purgeable measured at scan time (${new Date(report.measured_at * 1000).toISOString()})`,
  }
}
