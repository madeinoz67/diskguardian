import { describe, expect, test } from 'bun:test'
import { formatStamp, purgeMessage, snapshotsModel } from './snapshots'
import type { SnapshotReport } from './types'

function report(overrides: Partial<SnapshotReport> = {}): SnapshotReport {
  return { supported: true, mount: '/', snapshots: [], error: null, ...overrides }
}

const two = [
  { name: 'com.apple.TimeMachine.2026-11-01-085943.local', stamp: '2026-11-01-085943' },
  { name: 'com.apple.TimeMachine.2026-11-02-080842.local', stamp: '2026-11-02-080842' },
]

describe('the snapshots panel model', () => {
  test('timestamps are made readable and the newest is marked', () => {
    const model = snapshotsModel(report({ snapshots: two }))
    expect(model.rows.map((r) => r.when)).toEqual(['1 Nov 2026, 08:59', '2 Nov 2026, 08:08'])
    expect(model.rows[1]?.newest).toBe(true)
    expect(model.rows[0]?.newest).toBe(false)
  })

  test('no size is ever claimed for a snapshot', () => {
    const model = snapshotsModel(report({ snapshots: two }))
    expect(model.headline).toContain('does not report')
    for (const row of model.rows) {
      expect(JSON.stringify(row)).not.toMatch(/\d+\s?(bytes|KB|MB|GB)/)
    }
  })

  test('a failed listing is not an empty one', () => {
    const failed = snapshotsModel(report({ error: 'tmutil: not found' }))
    expect(failed.error).toBe('tmutil: not found')
    expect(failed.empty).toBeNull()
    expect(failed.canPurge).toBe(false)
  })

  test('an honest empty answer says Time Machine holds nothing', () => {
    const empty = snapshotsModel(report())
    expect(empty.headline).toBe('No local snapshots')
    expect(empty.empty).toContain('not holding any')
    expect(empty.canPurge).toBe(false)
  })

  test('purge is only offered when there is something to purge', () => {
    expect(snapshotsModel(report({ snapshots: two })).canPurge).toBe(true)
  })

  test('off macOS the panel says so instead of pretending to look', () => {
    const mac = snapshotsModel(report({ supported: false }))
    expect(mac.canPurge).toBe(false)
    expect(mac.empty).toContain('only on macOS')
  })
})

describe('stamps', () => {
  test('an unreadable stamp is shown verbatim, never guessed at', () => {
    expect(formatStamp('not-a-stamp')).toBe('not-a-stamp')
    expect(formatStamp('2026-13-01-085943')).toBe('2026-13-01-085943')
    expect(formatStamp('')).toBe('')
  })

  test('a real stamp formats', () => {
    expect(formatStamp('2026-11-02-080842')).toBe('2 Nov 2026, 08:08')
  })
})

describe('purge messages', () => {
  test("the backend's words are used as they arrive", () => {
    const text = 'macOS refused: deleting local snapshots needs administrator rights'
    expect(purgeMessage({ message: text } as never)).toBe(text)
  })

  test('an empty message still produces a sentence rather than a blank line', () => {
    expect(purgeMessage({ message: '  ' } as never)).toContain('without a message')
  })
})
