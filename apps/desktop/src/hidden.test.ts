// The hidden-space panel model, and the renders it must refuse.
//
// C29's falsifiers, as tests: a per-snapshot size shown anywhere; a missing decomposition
// shown as "0 other volumes"; a negative residual swallowed into a zero figure or a
// zero-width petal; purgeable space presented as freed or as part of the residual.

import { describe, expect, test } from 'bun:test'
import { hiddenModel } from './hidden'
import type { HiddenSpaceReport } from './types'

const SNAPSHOT_NOTE = 'macOS does not report how much each snapshot holds'

function report(overrides: Partial<HiddenSpaceReport> = {}): HiddenSpaceReport {
  return {
    mount: '/',
    used_bytes: 2_000_000_000,
    tree_bytes: 1_800_000_000,
    hidden_bytes: 200_000_000,
    other_volumes: [
      { name: 'Preboot', bytes: 100_000 },
      { name: 'VM', bytes: 50_000 },
    ],
    snapshot_count: 3,
    snapshot_note: SNAPSHOT_NOTE,
    restricted_roots: [{ path: '/private/one', note: 'permission denied' }],
    purgeable_bytes: 164_000_000_000,
    purgeable_note: 'measured as the difference between two capacity figures the OS reports',
    still_hidden_bytes: 199_850_000,
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

  test('a snapshot is never given a size', () => {
    const model = hiddenModel(report({ snapshot_count: 3 }))
    // Exact equality: the line is a count and the OS's own limitation, and nothing else —
    // any byte figure beside a snapshot would be invented (C27's doctrine, extended).
    expect(model.snapshotLine).toBe(`3 local snapshots · ${SNAPSHOT_NOTE}`)
  })

  test('a snapshot count of one is singular, and zero is the note alone', () => {
    expect(hiddenModel(report({ snapshot_count: 1 })).snapshotLine).toBe(
      `1 local snapshot · ${SNAPSHOT_NOTE}`,
    )
    expect(hiddenModel(report({ snapshot_count: 0 })).snapshotLine).toBe(SNAPSHOT_NOTE)
  })

  test('a missing decomposition is words, never a zero', () => {
    const model = hiddenModel(report({ other_volumes: null, still_hidden_bytes: null }))
    expect(model.otherVolumes).toBeNull()
    // The line must not be parseable as a figure: "0 other volumes" would claim a
    // measurement the OS never made.
    expect(model.otherVolumesLine).toBe('Breakdown not available on this volume.')
    expect(model.otherVolumesLine).not.toMatch(/^\d/)
  })

  test('other volumes are listed with figures and a total', () => {
    const model = hiddenModel(report())
    expect(model.otherVolumes?.map((v) => v.name)).toEqual(['Preboot', 'VM'])
    expect(model.otherVolumesLine).toBe('2 other volumes in this container · 146 KB')
  })

  test('purgeable is an upper bound, never freed space, and absent when unmeasured', () => {
    const model = hiddenModel(report())
    expect(model.purgeableLine).toContain('up to')
    expect(model.purgeableLine).toContain('153 GB')
    // Purgeable never appears as part of the hidden figure: the components the model
    // renders for the residual are other volumes and still-hidden only.
    expect(model.otherVolumesLine).not.toContain('purgeable')

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

  test('a still-hidden figure that would be negative is words about the inconsistency', () => {
    // other volumes exceeding the whole residual is the boundary-leak signal: the model
    // must not render a negative byte count any more than a silent zero.
    const model = hiddenModel(report({ still_hidden_bytes: -1_000 }))
    expect(model.stillHiddenLine).toContain('rescan')
    expect(model.stillHiddenLine).not.toMatch(/-/)
  })

  test('the Full Disk Access hint appears only while protected areas remain', () => {
    const hinted = hiddenModel(report())
    expect(hinted.fdaHint).toContain('Full Disk Access')

    const without = hiddenModel(report({ restricted_roots: [] }))
    expect(without.fdaHint).toBeNull()
    expect(without.protectedAreas).toEqual([])
  })

  test('protected areas are named with their reason', () => {
    const model = hiddenModel(report())
    expect(model.protectedAreas).toEqual([
      { path: '/private/one', note: 'permission denied' },
    ])
  })

  test('skipped mounts are listed, not silently absent', () => {
    const model = hiddenModel(report())
    expect(model.skippedLine).toContain('com.apple.TimeMachine.localsnapshots')
    const none = hiddenModel(report({ skipped_mounts: [] }))
    expect(none.skippedLine).toBeNull()
  })

  test('the decomposition figures say when they were measured', () => {
    const model = hiddenModel(report())
    expect(model.measuredLine).toContain('measured at scan time')
    expect(model.measuredLine).toContain('2026')
  })
})
