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
