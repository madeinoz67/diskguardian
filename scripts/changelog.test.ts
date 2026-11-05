// Tests for the changelog renderer. The fetching half shells out to git and gh and is
// not exercised here; everything that decides what the file says is pure and is.
import { expect, test, describe } from 'bun:test'
import { categoryFor, newContributors, renderChangelog, windows } from './changelog'
import type { PullRequest, Tag } from './changelog'

function pr(
  number: number,
  branch: string,
  mergedAt: string,
  author = 'seaton',
  title = `change ${number}`,
): PullRequest {
  return { number, title, branch, author, mergedAt }
}

const TAGS: Tag[] = [
  { version: '0.1.0', date: '2026-11-01T00:00:00Z' },
  { version: '0.1.1', date: '2026-11-03T00:00:00Z' },
]

describe('categoryFor', () => {
  test('each recognised prefix maps to its heading', () => {
    expect(categoryFor('feat/sunburst')).toBe('Features')
    expect(categoryFor('feature/sunburst')).toBe('Features')
    expect(categoryFor('fix/delete-guard')).toBe('Bug Fixes')
    expect(categoryFor('bugfix/delete-guard')).toBe('Bug Fixes')
    expect(categoryFor('docs/isa-publish')).toBe('Documentation')
    expect(categoryFor('deps/toolchain-majors')).toBe('Dependencies')
    expect(categoryFor('dependabot/cargo/sysinfo-0.40.1')).toBe('Dependencies')
    expect(categoryFor('ci/homebrew-tap')).toBe('CI')
    expect(categoryFor('security/codeql')).toBe('Security')
  })

  test('an unrecognised branch is Other, never a guess', () => {
    expect(categoryFor('sunburst-polish')).toBe('Other')
    expect(categoryFor('')).toBe('Other')
  })

  test('a release branch is not a change and is dropped', () => {
    expect(categoryFor('release/0.1.5')).toBeNull()
  })
})

describe('windows', () => {
  test('a pull request lands in exactly one window', () => {
    const prs = [
      pr(1, 'feat/a', '2026-10-31T00:00:00Z'),
      pr(2, 'feat/b', '2026-11-02T00:00:00Z'),
      pr(3, 'feat/c', '2026-11-04T00:00:00Z'),
    ]
    const result = windows(prs, TAGS)
    expect(result.map((w) => w.title)).toEqual(['0.1.0', '0.1.1', 'Unreleased'])
    expect(result[0].prs.map((p) => p.number)).toEqual([1])
    expect(result[1].prs.map((p) => p.number)).toEqual([2])
    expect(result[2].prs.map((p) => p.number)).toEqual([3])
  })

  test('a pull request merged exactly at a tag belongs to that tag', () => {
    const result = windows([pr(1, 'fix/a', '2026-11-03T00:00:00Z')], TAGS)
    expect(result[1].prs.map((p) => p.number)).toEqual([1])
  })

  test('with no tags everything is unreleased', () => {
    const result = windows([pr(1, 'feat/a', '2026-11-01T00:00:00Z')], [])
    expect(result).toHaveLength(1)
    expect(result[0].title).toBe('Unreleased')
    expect(result[0].prs).toHaveLength(1)
  })
})

describe('renderChangelog', () => {
  const prs = [
    pr(1, 'feat/one', '2026-10-31T00:00:00Z'),
    pr(2, 'fix/two', '2026-11-02T00:00:00Z'),
    pr(3, 'wip/three', '2026-11-04T00:00:00Z'),
    pr(4, 'release/0.1.5', '2026-11-04T00:00:00Z'),
  ]
  const md = renderChangelog(prs, TAGS)

  test('unreleased leads the file', () => {
    expect(md).toContain('## [Unreleased]')
    expect(md.indexOf('## [Unreleased]')).toBeLessThan(md.indexOf('## [0.1.1]'))
  })

  test('each release carries its dated heading and category subheadings', () => {
    expect(md).toContain('## [0.1.0] - 2026-11-01')
    expect(md).toContain('## [0.1.1] - 2026-11-03')
    expect(md).toContain('### Features')
    expect(md).toContain('### Bug Fixes')
  })

  test('an entry names the pull request and its author', () => {
    expect(md).toContain(
      '- change 1 ([#1](https://github.com/madeinoz67/diskguardian/pull/1)) by @seaton',
    )
  })

  test('an unclassifiable pull request still appears, under Other', () => {
    expect(md).toContain('### Other')
    expect(md).toContain('[#3]')
  })

  test('release branches are omitted entirely', () => {
    expect(md).not.toContain('[#4]')
  })

  test('render is deterministic', () => {
    expect(renderChangelog(prs, TAGS)).toBe(md)
  })

  test('an empty window says so rather than rendering nothing', () => {
    expect(renderChangelog([], TAGS)).toContain('_Nothing yet._')
  })

  test('a released version with no entries is worded as a fact, not a promise', () => {
    const md = renderChangelog([], TAGS)
    expect(md).toContain('_No changes recorded._')
    // "Nothing yet" belongs to Unreleased alone.
    expect(md.split('_Nothing yet._').length - 1).toBe(1)
  })
})

describe('new contributors', () => {
  test('credits each author once, on the release they first appear', () => {
    const prs = [
      pr(1, 'feat/a', '2026-10-31T00:00:00Z', 'veteran'),
      pr(2, 'feat/b', '2026-11-02T00:00:00Z', 'veteran'),
      pr(3, 'feat/c', '2026-11-02T00:00:00Z', 'newcomer'),
    ]
    const md = renderChangelog(prs, TAGS)
    // Two people are new, each to their own first release; neither is credited twice,
    // and the one who appears twice is credited only for the first.
    expect(md.split('made their first contribution').length - 1).toBe(2)
    expect(md.split('@veteran made their first contribution').length - 1).toBe(1)
    expect(md.split('@newcomer made their first contribution').length - 1).toBe(1)
  })

  test('an author who appears only in unreleased is not yet credited', () => {
    const prs = [pr(9, 'feat/late', '2026-11-04T00:00:00Z', 'fresher')]
    const md = renderChangelog(prs, TAGS)
    expect(md).not.toContain('made their first contribution')
  })

  test('someone already seen in an earlier release is not new again', () => {
    const window = { title: '0.1.2', date: '2026-11-05T00:00:00Z', prs: [pr(5, 'feat/x', '2026-11-04T00:00:00Z', 'veteran')] }
    expect(newContributors(window, new Set(['veteran']))).toHaveLength(0)
  })

  test('a bot is never credited as a new contributor', () => {
    const prs = [pr(7, 'dependabot/cargo/sysinfo-0.40.1', '2026-11-02T00:00:00Z', 'dependabot[bot]')]
    expect(renderChangelog(prs, TAGS)).not.toContain('made their first contribution')
  })
})
