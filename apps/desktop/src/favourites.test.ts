import { describe, expect, test } from 'bun:test'
import {
  favouritesModel,
  framesInvalidatedByStarToggle,
  MISSING_FAVOURITE_SENTENCE,
  type FavouriteRowModel,
} from './favourites'
import { favourites, isTauri, setFavourite } from './bridge'
import type { FavouritesReport } from './types'

function report(overrides: Partial<FavouritesReport> = {}): FavouritesReport {
  return { favourites: [], notice: null, ...overrides }
}

describe('the favourites overview model', () => {
  test('a missing favourite is kept, not filtered, and says the exact sentence', () => {
    const model = favouritesModel(
      report({ favourites: [{ path: '/Volumes/Backup/project', exists: false }] }),
    )
    expect(model.rows.length).toBe(1)
    const row = model.rows[0] as FavouriteRowModel
    expect(row.unavailable).toBe(true)
    expect(row.detail).toBe(MISSING_FAVOURITE_SENTENCE)
    // The exact words are pinned here, not just the constant: the overview shows this
    // sentence as visible text, and a paraphrase would be a different promise.
    expect(MISSING_FAVOURITE_SENTENCE).toBe('not on this disk right now')
  })

  test('an existing favourite shows its path and stays available', () => {
    const model = favouritesModel(
      report({ favourites: [{ path: '/Users/you/Projects', exists: true }] }),
    )
    const row = model.rows[0] as FavouriteRowModel
    expect(row.unavailable).toBe(false)
    expect(row.detail).toBe('/Users/you/Projects')
    expect(row.name).toBe('Projects')
    expect(row.path).toBe('/Users/you/Projects')
  })

  test('a mixed list keeps every row: the model never filters by existence', () => {
    const model = favouritesModel(
      report({
        favourites: [
          { path: '/a', exists: true },
          { path: '/b', exists: false },
          { path: '/c', exists: true },
        ],
      }),
    )
    expect(model.rows.length).toBe(3)
    expect(model.rows.map((row) => (row as FavouriteRowModel).unavailable)).toEqual([
      false,
      true,
      false,
    ])
  })

  test('store order is preserved, with no sorting and no dedup', () => {
    // Dedup is the store's contract, tested in Rust; if two identical entries ever
    // arrive here the model renders both rather than quietly repairing them.
    const model = favouritesModel(
      report({
        favourites: [
          { path: '/zzz', exists: true },
          { path: '/aaa', exists: true },
          { path: '/zzz', exists: true },
        ],
      }),
    )
    expect(model.rows.map((row) => (row as FavouriteRowModel).path)).toEqual([
      '/zzz',
      '/aaa',
      '/zzz',
    ])
  })

  test('the recovery notice travels with the rows and is never swallowed', () => {
    const noticed = favouritesModel(
      report({ notice: 'the favourites list could not be read and was reset' }),
    )
    expect(noticed.notice).toBe('the favourites list could not be read and was reset')

    const quiet = favouritesModel(report({ favourites: [{ path: '/a', exists: true }] }))
    expect(quiet.notice).toBeNull()
  })
})

describe('which frames a star toggle makes stale', () => {
  test('a heading star also invalidates the parent level, whose child mark it changes', () => {
    // up() re-renders the parent from its cache without refetching; a heading star
    // that did not invalidate it left a hollow star on a starred folder.
    expect(framesInvalidatedByStarToggle([null, 5], 5)).toEqual([1, 0])
  })

  test('a heading star on the scan root has no parent level', () => {
    expect(framesInvalidatedByStarToggle([null], null)).toEqual([0])
  })

  test('deeper stacks invalidate the frame and the one directly below it', () => {
    expect(framesInvalidatedByStarToggle([null, 2, 5, 9], 5)).toEqual([2, 1])
  })

  test('a row star of a child with no frame of its own touches only the level viewed', () => {
    expect(framesInvalidatedByStarToggle([null, 5], 9)).toEqual([1])
  })

  test('an empty stack has nothing to invalidate', () => {
    expect(framesInvalidatedByStarToggle([], 9)).toEqual([])
  })
})

describe('the demo favourites', () => {
  test('are seeded with one existing and one missing entry', async () => {
    // Bun has no webview, so this run is the demo path by construction; the assertion
    // keeps the test from passing vacuously if it is ever run somewhere that is not.
    expect(isTauri).toBe(false)

    const demo = await favourites()
    expect(demo.favourites.length).toBe(2)
    expect(demo.favourites.filter((entry) => entry.exists)).toHaveLength(1)
    expect(demo.favourites.filter((entry) => !entry.exists)).toHaveLength(1)
    expect(demo.notice).toBeNull()
  })

  test('unstar works in memory and does not pretend to persist', async () => {
    expect(isTauri).toBe(false)

    const demo = await favourites()
    const existing = demo.favourites.find((entry) => entry.exists)
    expect(existing).toBeDefined()

    const remaining = await setFavourite(existing!.path, false)
    expect(remaining).not.toContain(existing!.path)

    // Restored for whichever test runs next: the seed is module state.
    await setFavourite(existing!.path, true)
  })
})
