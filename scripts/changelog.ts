// changelog — regenerates CHANGELOG.md from merged pull requests.
//
// Not from commit subjects. This repository's commits are prose ("Handle a fallible disk
// list: sysinfo 0.40 made the constructor return Result"), which is good writing and
// useless grouping: a conventional-commit tool would file every one of them under
// "Other". The pull request is where the change is actually described, and its head
// branch already carries the one classifying signal this repo produces reliably — its
// prefix. So the category is inferred from the branch, and a branch that names no
// category lands in Other rather than being guessed at.
//
// Two things it must get right, because a changelog nobody trusts is worse than none:
//
//   1. **Unreleased is a real section.** Merges accumulate there between tags, so the
//      file is current the moment `main` moves, not at the next release.
//   2. **New contributors are computed, never remembered.** An author is new to a
//      release when no pull request of theirs was merged before that release's window
//      opened. Remembering it in the file would let it drift.
//
// The renderer is pure and its tests are in `changelog.test.ts`. Fetching lives in
// `main`, so nothing in the tested half shells out or hits the network.
import { execFileSync } from 'node:child_process'
import { readFileSync, writeFileSync } from 'node:fs'

export const REPO = 'madeinoz67/diskguardian'
const REPO_URL = `https://github.com/${REPO}`

/** One merged pull request, reduced to what the changelog renders. */
export interface PullRequest {
  number: number
  title: string
  branch: string
  author: string
  /** ISO timestamp; the boundary the release windows are cut on. */
  mergedAt: string
}

/** A release tag and the moment its commit was created. */
export interface Tag {
  version: string // "0.1.5", no leading v
  date: string // ISO
}

export const CATEGORIES = [
  'Features',
  'Bug Fixes',
  'Documentation',
  'Dependencies',
  'CI',
  'Security',
  'Other',
] as const
export type Category = (typeof CATEGORIES)[number]

/** Branches that must never appear as a changelog entry. */
const SKIP_BRANCH_PREFIXES = ['release/']

/**
 * The heading a pull request belongs under, from its head branch.
 *
 * Deliberately a prefix table rather than a text search: `docs/...` is documentation
 * because of how it is named, not because its title happens to contain the word.
 * Unrecognised prefixes return `Other`, which is an honest "not classified" rather
 * than a guess.
 */
export function categoryFor(branch: string): Category | null {
  for (const skip of SKIP_BRANCH_PREFIXES) {
    if (branch.startsWith(skip)) return null
  }
  if (branch.startsWith('feat/') || branch.startsWith('feature/')) return 'Features'
  if (branch.startsWith('fix/') || branch.startsWith('bugfix/')) return 'Bug Fixes'
  if (branch.startsWith('docs/')) return 'Documentation'
  if (branch.startsWith('deps/') || branch.startsWith('dependabot/')) return 'Dependencies'
  if (branch.startsWith('ci/')) return 'CI'
  if (branch.startsWith('security/')) return 'Security'
  return 'Other'
}

interface Window {
  title: string // "Unreleased" or "0.1.5"
  date: string | null
  prs: PullRequest[]
}

/**
 * Cut the pull requests into release windows.
 *
 * A window runs from the previous tag's date (exclusive) to its own tag's date
 * (inclusive); everything after the last tag is Unreleased. Sorting by merge time and
 * walking tags in order is what makes the boundaries total: every pull request lands in
 * exactly one window.
 */
export function windows(prs: PullRequest[], tags: Tag[]): Window[] {
  const sorted = [...tags].sort((a, b) => a.date.localeCompare(b.date))
  const ordered = [...prs].sort((a, b) => a.mergedAt.localeCompare(b.mergedAt))

  const out: Window[] = []
  let taken = 0
  for (const tag of sorted) {
    const inWindow: PullRequest[] = []
    while (taken < ordered.length && ordered[taken].mergedAt <= tag.date) {
      inWindow.push(ordered[taken])
      taken += 1
    }
    out.push({ title: tag.version, date: tag.date, prs: inWindow })
  }
  out.push({ title: 'Unreleased', date: null, prs: ordered.slice(taken) })
  return out
}

/**
 * Authors whose first merged pull request falls inside this window.
 *
 * `before` is every login that had already merged something earlier in the file, so this
 * has to be called in release order. Bot accounts are excluded: "dependabot made their
 * first contribution" is true and meaningless, and it would bury a real person's first
 * entry under a dependency bump.
 */
export function newContributors(
  window: Window,
  alreadySeen: Set<string>,
): Array<{ login: string; pr: PullRequest }> {
  const found: Array<{ login: string; pr: PullRequest }> = []
  const local = new Set<string>()
  for (const pr of [...window.prs].sort((a, b) => a.number - b.number)) {
    if (pr.author.endsWith('[bot]')) continue
    if (alreadySeen.has(pr.author) || local.has(pr.author)) continue
    local.add(pr.author)
    found.push({ login: pr.author, pr })
  }
  return found
}

function renderPr(pr: PullRequest): string {
  return `- ${pr.title} ([#${pr.number}](${REPO_URL}/pull/${pr.number})) by @${pr.author}`
}

/** Render the whole file. Pure: same inputs, same bytes, no clock and no network. */
export function renderChangelog(prs: PullRequest[], tags: Tag[]): string {
  const header = [
    '# Changelog',
    '',
    'All notable changes to DiskGuardian, generated from merged pull requests by',
    '`scripts/changelog.ts`. Do not edit this file by hand: the changelog workflow',
    'regenerates it on every merge to `main` and on every release tag.',
    '',
  ]

  // Built oldest-first, because "new contributor" is a question about what came before,
  // then reversed for the page: a changelog opens on Unreleased and works backwards.
  const seen = new Set<string>()
  const sections: string[][] = []

  for (const window of windows(prs, tags)) {
    const lines: string[] = []
    const heading =
      window.date === null
        ? '## [Unreleased]'
        : `## [${window.title}] - ${window.date.slice(0, 10)}`
    lines.push(heading, '')

    const byCategory = new Map<Category, PullRequest[]>()
    for (const pr of window.prs) {
      const category = categoryFor(pr.branch)
      if (category === null) continue
      const bucket = byCategory.get(category) ?? []
      bucket.push(pr)
      byCategory.set(category, bucket)
    }

    if (byCategory.size === 0) {
      // A released version with no entries is a fact about that release, not a promise:
      // "nothing yet" belongs to Unreleased alone.
      lines.push(window.date === null ? '_Nothing yet._' : '_No changes recorded._', '')
    } else {
      for (const category of CATEGORIES) {
        const bucket = byCategory.get(category)
        if (!bucket) continue
        lines.push(`### ${category}`, '')
        for (const pr of bucket.sort((a, b) => a.number - b.number)) {
          lines.push(renderPr(pr))
        }
        lines.push('')
      }
    }

    // Unreleased has no fixed author list, so "new" has no meaning there yet; the
    // section appears on the release that actually credits them.
    if (window.date !== null) {
      const fresh = newContributors(window, seen)
      if (fresh.length > 0) {
        lines.push('### New Contributors', '')
        for (const { login, pr } of fresh) {
          lines.push(
            `- @${login} made their first contribution in ([#${pr.number}](${REPO_URL}/pull/${pr.number}))`,
          )
        }
        lines.push('')
      }
    }

    for (const pr of window.prs) seen.add(pr.author)
    sections.push(lines)
  }

  const body = sections.reverse().flat()
  return [...header, ...body].join('\n').replace(/\n+$/, '\n')
}

// --- fetching (untested on purpose: it shells out to git and gh) ---------------------

function git(args: string[]): string {
  return execFileSync('git', args, { encoding: 'utf8' })
}

function readTags(): Tag[] {
  const names = git(['tag', '--list', 'v*']).split('\n').filter(Boolean)
  return names.map((name) => ({
    version: name.replace(/^v/, ''),
    date: git(['log', '-1', '--format=%cI', name]).trim(),
  }))
}

function readPullRequests(): PullRequest[] {
  const raw = execFileSync(
    'gh',
    ['api', '--paginate', `repos/${REPO}/pulls?state=closed&per_page=100`],
    { encoding: 'utf8', maxBuffer: 64 * 1024 * 1024 },
  )
  // `--paginate` emits one JSON array per page; join them before parsing.
  const pages: unknown[] = JSON.parse(
    `[${raw.trim().split(/\]\s*\[/).join('],[')}]`,
  ) as unknown[]
  const flat = pages.flat() as Array<Record<string, any>>
  return flat
    .filter((pr) => pr.merged_at)
    .map((pr) => ({
      number: pr.number as number,
      title: (pr.title as string).trim(),
      branch: (pr.head?.ref as string) ?? '',
      author: (pr.user?.login as string) ?? 'ghost',
      mergedAt: pr.merged_at as string,
    }))
}

function main(): void {
  const toStdout = process.argv.includes('--stdout')
  const next = renderChangelog(readPullRequests(), readTags())

  if (toStdout) {
    process.stdout.write(next)
    return
  }

  const path = 'CHANGELOG.md'
  let current = ''
  try {
    current = readFileSync(path, 'utf8')
  } catch {
    current = ''
  }
  if (current === next) {
    console.log('CHANGELOG.md is already current')
    return
  }
  writeFileSync(path, next)
  console.log('CHANGELOG.md regenerated')
}

if (import.meta.main) main()
