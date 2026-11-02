// The browser demo's snapshot answers. The real listing and purge come from Rust through
// IPC; these tests pin the honesty rules of the fallback that serves the plain-browser
// build, where no `tmutil` exists: the listing is shape-complete, and a purge never
// claims to have deleted anything.

import { describe, expect, test } from 'bun:test'
import { isTauri, localSnapshots, purgeLocalSnapshots } from './bridge'
import { formatStamp } from './snapshots'

describe('the demo snapshot listing', () => {
  test('serves plausible snapshots whose stamps are readable', async () => {
    // Bun has no webview, so this run is the demo path by construction; the assertion
    // keeps the test from passing vacuously if it is ever run somewhere that is not.
    expect(isTauri).toBe(false)

    const report = await localSnapshots()
    expect(report.supported).toBe(true)
    expect(report.snapshots.length).toBeGreaterThanOrEqual(2)
    for (const snapshot of report.snapshots) {
      // A demo stamp that does not format is a stamp the panel would show verbatim,
      // which would make the browser demo look broken rather than synthetic.
      expect(formatStamp(snapshot.stamp)).not.toBe(snapshot.stamp)
    }
  })
})

describe('the demo purge', () => {
  test('never claims a purge succeeded', async () => {
    expect(isTauri).toBe(false)

    const report = await purgeLocalSnapshots(true)
    expect(report.purged).toBe(false)
    expect(report.freed_bytes).toBe(0)
    expect(report.remaining.length).toBeGreaterThan(0)
    expect(report.message.trim().length).toBeGreaterThan(0)
  })
})
