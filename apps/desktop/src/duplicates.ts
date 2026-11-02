// What the duplicates panel says, kept free of the DOM so it can be tested.
//
// The rule this file exists to enforce: **a group is never shown bare.** A report is a
// statement about content, not about reclaimable space — two content-identical files can
// be APFS clones sharing one set of blocks, in which case deleting one frees nothing. So
// every group carries the engine's caveat as visible text, and every figure that could be
// mistaken for free space is written as "up to".

import { formatBytes } from './format'
import type { DuplicateGroup, DuplicateReport } from './types'

/**
 * Shown when a report arrives without its caveat. The engine always sends one; this
 * fallback means a malformed or older payload still cannot render an unqualified promise.
 */
export const FALLBACK_CAVEAT =
  'content-identical files: some members may share physical storage (APFS clones), so deleting one can free less than the figure shown'

export interface DuplicateMemberModel {
  node: number
  name: string
  /** The folder, so two copies with the same name are tellable apart. */
  dir: string
  bytes: string
  /** `keep` is the newest copy; the panel offers no delete for it. */
  role: 'keep' | 'collect'
}

export interface DuplicateGroupModel {
  title: string
  wasted: string
  /** Visible text under the group, never a tooltip and never empty. */
  caveat: string
  members: DuplicateMemberModel[]
}

export interface DuplicatesModel {
  headline: string
  /** Partial pass, unreadable files, excluded hard links. `null` when there is nothing to say. */
  note: string | null
  groups: DuplicateGroupModel[]
}

/** The last path component, coping with both separators. */
export function fileName(path: string): string {
  const parts = path.split(/[/\\]/).filter((part) => part.length > 0)
  return parts[parts.length - 1] ?? path
}

/** Everything before the last separator, or an empty string for a bare name. */
export function dirName(path: string): string {
  const index = Math.max(path.lastIndexOf('/'), path.lastIndexOf('\\'))
  return index <= 0 ? (index === 0 ? path.slice(0, 1) : '') : path.slice(0, index)
}

function memberModel(member: DuplicateGroup['members'][number], bytes: number): DuplicateMemberModel {
  return {
    node: member.node,
    name: fileName(member.path),
    dir: dirName(member.path),
    bytes: formatBytes(bytes),
    role: member.keep ? 'keep' : 'collect',
  }
}

function groupModel(group: DuplicateGroup): DuplicateGroupModel {
  const copies = group.members.length
  // Keeper first: the row a user should not touch sits at the top of its group.
  const ordered = [...group.members].sort((a, b) => Number(b.keep) - Number(a.keep))
  return {
    title: `${formatBytes(group.bytes_each)} each · ${copies} copies`,
    wasted: `up to ${formatBytes(group.wasted_bytes)}`,
    caveat: group.caveat.trim().length > 0 ? group.caveat : FALLBACK_CAVEAT,
    members: ordered.map((member) => memberModel(member, group.bytes_each)),
  }
}

export function duplicatesModel(report: DuplicateReport): DuplicatesModel {
  const groups = report.groups.map(groupModel)
  const reclaimable = report.groups.reduce((sum, group) => sum + group.wasted_bytes, 0)

  const headline =
    groups.length === 0
      ? report.partial
        ? 'No duplicates found before the time budget ran out'
        : 'No duplicate files above this size'
      : `${groups.length} group${groups.length === 1 ? '' : 's'} · up to ${formatBytes(reclaimable)} could be freed`

  const notes: string[] = []
  if (report.partial) {
    notes.push('Stopped at the time budget: more duplicates may exist, so press Refresh to look further')
  }
  if (report.errors > 0) {
    notes.push(`${report.errors} file${report.errors === 1 ? '' : 's'} could not be read`)
  }
  if (report.excluded_links > 0) {
    notes.push(
      `${report.excluded_links} hard-linked name${report.excluded_links === 1 ? '' : 's'} excluded: deleting one frees nothing`,
    )
  }

  return { headline, note: notes.length > 0 ? notes.join(' · ') : null, groups }
}
