import { describe, expect, test } from 'bun:test'
import { dirName, duplicatesModel, fileName, FALLBACK_CAVEAT } from './duplicates'
import type { DuplicateReport } from './types'

const MB = 1024 * 1024

function report(overrides: Partial<DuplicateReport> = {}): DuplicateReport {
  return {
    groups: [],
    partial: false,
    files_hashed: 0,
    errors: 0,
    excluded_links: 0,
    duration_ms: 12,
    ...overrides,
  }
}

const caveat = 'content-identical (whole-file BLAKE3); may share storage'

function pair(): DuplicateReport {
  return report({
    groups: [
      {
        bytes_each: 128 * MB,
        wasted_bytes: 128 * MB,
        caveat,
        members: [
          { node: 4, path: '/Users/you/a/report.pdf', keep: false },
          { node: 9, path: '/Users/you/b/report copy.pdf', keep: true },
        ],
      },
    ],
  })
}

describe('the duplicates panel model', () => {
  test('every group carries the caveat as visible text', () => {
    const model = duplicatesModel(pair())
    expect(model.groups).toHaveLength(1)
    expect(model.groups[0]?.caveat).toBe(caveat)
  })

  test('a report without a caveat still cannot render bare', () => {
    const stripped = pair()
    const group = stripped.groups[0]
    if (group) group.caveat = '   '
    expect(duplicatesModel(stripped).groups[0]?.caveat).toBe(FALLBACK_CAVEAT)
  })

  test('the size beside a group is a ceiling, never a promise', () => {
    const model = duplicatesModel(pair())
    expect(model.groups[0]?.wasted.startsWith('up to ')).toBe(true)
    expect(model.headline).toContain('up to')
    expect(model.headline).toContain('could be freed')
  })

  test('the keeper is marked and the other copy is offered for collection', () => {
    const members = duplicatesModel(pair()).groups[0]?.members ?? []
    expect(members).toHaveLength(2)
    expect(members[0]?.role).toBe('keep')
    expect(members.filter((m) => m.role === 'collect')).toHaveLength(1)
    expect(members[0]?.name).toBe('report copy.pdf')
  })

  test('an empty report says so, and a spent budget says which so', () => {
    expect(duplicatesModel(report()).headline).toBe('No duplicate files above this size')
    expect(duplicatesModel(report({ partial: true })).headline).toContain('before the time budget')
  })

  test('a partial pass, unreadable files and excluded links are all spoken', () => {
    const note = duplicatesModel(
      report({ partial: true, errors: 2, excluded_links: 1 }),
    ).note
    expect(note).toContain('more duplicates may exist')
    expect(note).toContain('2 files could not be read')
    expect(note).toContain('hard-linked name excluded')
  })

  test('nothing to report means no note at all', () => {
    expect(duplicatesModel(report()).note).toBeNull()
  })
})

describe('paths, because a duplicate list is mostly paths', () => {
  test('both separators, because Windows exists', () => {
    expect(fileName('/Users/you/a/report.pdf')).toBe('report.pdf')
    expect(fileName('C:\\Users\\you\\report.pdf')).toBe('report.pdf')
    expect(dirName('/Users/you/a/report.pdf')).toBe('/Users/you/a')
    expect(dirName('C:\\Users\\you\\report.pdf')).toBe('C:\\Users\\you')
  })

  test('a bare name has no directory, and a root keeps its slash', () => {
    expect(dirName('report.pdf')).toBe('')
    expect(dirName('/report.pdf')).toBe('/')
  })
})
