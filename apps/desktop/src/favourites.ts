// What the favourites block on the "Disks and folders" page says, kept free of the DOM
// so it can be tested.
//
// Two honesty rules live here:
//
// 1. **A missing favourite is reported, never filtered.** The store already decided
//    what is in the list; the model renders what it is given. A favourite whose folder
//    has gone carries the missing sentence as its detail line, and unstarring is the
//    way to clear it.
// 2. **The recovery notice travels with the rows.** A corrupt store that was reset is
//    visible text, not a quiet "no favourites" — the same rule as a failed snapshot
//    listing.

import type { FavouritesReport } from './types'
import { basename } from './overview'

/** The visible-text reason on a favourite whose folder is gone. Exact words, pinned by test. */
export const MISSING_FAVOURITE_SENTENCE = 'not on this disk right now'

export interface FavouriteRowModel {
  /** Basename of the stored path. */
  name: string
  /** The store's own string, passed back verbatim on unstar. */
  path: string
  /** Visible text: the path, or the missing sentence. */
  detail: string
  unavailable: boolean
}

export interface FavouritesModel {
  rows: FavouriteRowModel[]
  /** The store's recovery notice when present; rendered, never swallowed. */
  notice: string | null
}

export function favouritesModel(report: FavouritesReport): FavouritesModel {
  return {
    // Store order, no sorting, no dedup: dedup is the store's contract, tested in Rust.
    rows: report.favourites.map((entry) => ({
      name: basename(entry.path),
      path: entry.path,
      detail: entry.exists ? entry.path : MISSING_FAVOURITE_SENTENCE,
      unavailable: !entry.exists,
    })),
    notice: report.notice !== null && report.notice.length > 0 ? report.notice : null,
  }
}

/**
 * Which frames' cached marks a star toggle makes stale, as indices into the stack.
 *
 * A row star changes only the level being viewed (its children marks). A heading star
 * changes one more thing the design's staleness argument missed: the toggled folder is
 * a *child* of the level below, and `up()` re-renders that level from its cache
 * without refetching — so the parent's child mark must be refreshed at toggle time or
 * a starred folder shows a hollow star after climbing. A toggled folder with no frame
 * of its own on the stack touches only the top frame.
 */
export function framesInvalidatedByStarToggle(
  stack: ReadonlyArray<number | null>,
  toggled: number | null,
): number[] {
  const own = stack.lastIndexOf(toggled)
  if (own === -1) {
    return stack.length > 0 ? [stack.length - 1] : []
  }
  return own > 0 ? [own, own - 1] : [own]
}
