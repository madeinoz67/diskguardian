// The hidden-space panel model, and the renders it must refuse.
//
// C29 narrowed (2026-11-03): the decomposition into other volumes, snapshots and
// protected areas is HELD pending the external-volume cross-check, so the model carries
// no decomposition category at all — an unshown category can never be read as an empty
// one. The refusals that stay, each pinned by a test: a negative residual is words, not
// a zero-width petal; purgeable is "up to", never freed space and never a slice of the
// hidden figure; the held note passes through verbatim, never swallowed or reworded.

import { describe, expect, test } from 'bun:test'
import { hiddenModel } from './hidden'
import type { HiddenSpaceReport } from './types'

const HELD_NOTE =
  'The breakdown into other volumes, snapshots and protected areas is held pending a cross-check on a second volume: until then no category of the hidden figure is shown — not as a figure, and not as zero. The hidden figure itself is measured: the volume\'s used space minus everything this scan could see.'

/** The decomposition's fields: none of these may exist on the model while it is held. */
const HELD_FIELDS = [
  'otherVolumes',
  'otherVolumesLine',
  'snapshotLine',
  'stillHiddenLine',
  'protectedAreas',
  'fdaHint',
] as const

/** The shipped surface: exactly these fields, held line included. */
const SHIPPED_FIELDS = [
  'headline',
  'petalBytes',
  'hiddenFigure',
  'negativeNote',
  'heldLine',
  'purgeableLine',
  'skippedLine',
  'measuredLine',
] as const

function report(overrides: Partial<HiddenSpaceReport> = {}): HiddenSpaceReport {
  return {
    mount: '/',
    used_bytes: 2_000_000_000,
    tree_bytes: 1_800_000_000,
    hidden_bytes: 200_000_000,
    decomposition_held_note: HELD_NOTE,
    purgeable_bytes: 164_000_000_000,
    purgeable_note: 'measured as the difference between two capacity figures the OS reports',
    skipped_mounts: ['/Volumes/com.apple.TimeMachine.localsnapshots/xyz'],
    measured_at: 1_790_000_000,
    negative_note: null,
    ...overrides,
  }
}

describe('the hidden-space model', () => {
  test('the headline names the hidden figure and the volume used', () => {
    const model = hiddenModel(report())
    expect(model.headline).toBe('Hidden space · 191 MB of 1.9 GB used')
    expect(model.petalBytes).toBe(200_000_000)
  })

  test('while the breakdown is held, the model exposes no decomposition category', () => {
    const model = hiddenModel(report()) as unknown as Record<string, unknown>
    const keys = Object.keys(model)
    for (const field of HELD_FIELDS) {
      expect(keys).not.toContain(field)
    }
    for (const field of SHIPPED_FIELDS) {
      expect(keys).toContain(field)
    }
  })

  test('the held note is passed through verbatim, never swallowed or reworded', () => {
    expect(hiddenModel(report()).heldLine).toBe(HELD_NOTE)
    expect(
      hiddenModel(report({ decomposition_held_note: 'different words entirely' })).heldLine,
    ).toBe('different words entirely')
  })

  test('purgeable is an upper bound, never freed space, and absent when unmeasured', () => {
    const model = hiddenModel(report())
    expect(model.purgeableLine).toContain('up to')
    expect(model.purgeableLine).toContain('153 GB')
    // Purgeable never appears as part of the hidden figure: the model renders the
    // residual as one figure with no components at all.
    expect(model.headline).not.toContain('purgeable')

    const unmeasured = hiddenModel(report({ purgeable_bytes: null, purgeable_note: null }))
    expect(unmeasured.purgeableLine).toBeNull()
  })

  test('a negative residual keeps its words and draws no petal', () => {
    const note = 'The volume changed more than the scan window — rescan for exact figures.'
    const model = hiddenModel(report({ hidden_bytes: -50_000_000, negative_note: note }))
    expect(model.petalBytes).toBeNull()
    expect(model.negativeNote).toBe(note)
    expect(model.headline).not.toContain('50')
  })

  test('a negative residual within rounding of zero shows zero, with the note still visible', () => {
    // 5 MB of drift against a 2 GB volume is inside the 0.5% tolerance: the figure may
    // read as zero, the words may not disappear.
    const note = 'The volume changed more than the scan window — rescan for exact figures.'
    const model = hiddenModel(report({ hidden_bytes: -5_000_000, negative_note: note }))
    expect(model.petalBytes).toBeNull()
    expect(model.negativeNote).toBe(note)
    expect(model.hiddenFigure).toBe('0 bytes')
  })

  test('the model never swallows a negative residual the backend failed to word', () => {
    const model = hiddenModel(report({ hidden_bytes: -50_000_000, negative_note: null }))
    expect(model.negativeNote).not.toBeNull()
  })

  test('skipped mounts are listed, not silently absent', () => {
    const model = hiddenModel(report())
    expect(model.skippedLine).toContain('com.apple.TimeMachine.localsnapshots')
    const none = hiddenModel(report({ skipped_mounts: [] }))
    expect(none.skippedLine).toBeNull()
  })

  test('the scan-time line claims only the purgeable measurement, and nothing without one', () => {
    const model = hiddenModel(report())
    expect(model.measuredLine).toContain('Purgeable measured at scan time')
    expect(model.measuredLine).toContain('2026')

    // No purgeable figure means nothing was measured at scan time: no line, because a
    // "measured at" line over an unmeasured panel would be a claim this app refuses.
    const unmeasured = hiddenModel(report({ purgeable_bytes: null, purgeable_note: null }))
    expect(unmeasured.measuredLine).toBeNull()
  })
})
