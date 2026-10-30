// Progressive paint, kept pure so it can be tested without a canvas or a scanner.
//
// A petal's width is its subtree's total, so the first frame cannot be correct from a
// directory listing alone. It can be honest: every branch is drawn at equal width and
// marked "measuring", then each grows to its true width as the scanner reports it.

import type { ViewNode } from './types'

export interface PartialBranch {
  name: string
  target: number
  shown: number
  done: boolean
}

/** Branches the scanner has not measured yet are drawn at a nominal width. */
export const NOMINAL = 1

export function startBranches(names: string[]): PartialBranch[] {
  return names.map((name) => ({ name, target: NOMINAL, shown: NOMINAL, done: false }))
}

/** Apply a completion report. Returns false when the branch is unknown. */
export function applyBranch(branches: PartialBranch[], name: string, bytes: number): boolean {
  const branch = branches.find((b) => b.name === name)
  if (!branch) return false
  branch.target = Math.max(bytes, NOMINAL)
  branch.done = true
  return true
}

/** One easing step toward the targets. Returns true while anything is still moving. */
export function advance(branches: PartialBranch[]): boolean {
  let moving = false
  for (const branch of branches) {
    const delta = branch.target - branch.shown
    if (Math.abs(delta) > Math.max(2048, branch.target * 0.002)) {
      branch.shown += delta * 0.16
      moving = true
    } else if (branch.shown !== branch.target) {
      branch.shown = branch.target
    }
  }
  return moving
}

/** A one-level view: the root plus its branches, no deeper rings. */
export function partialView(rootName: string, branches: PartialBranch[]): ViewNode[] {
  const total = branches.reduce((sum, b) => sum + b.shown, 0) || 1
  return [
    {
      index: 0,
      node_id: null,
      name: rootName,
      bytes: total,
      kind: 'dir',
      children: branches.map((_, i) => i + 1),
      aggregated_count: 0,
      partial: true,
    },
    ...branches.map((branch, i) => ({
      index: i + 1,
      node_id: null,
      name: branch.name,
      bytes: branch.shown,
      kind: (branch.done ? 'dir' : 'measuring') as ViewNode['kind'],
      children: [],
      aggregated_count: 0,
      partial: !branch.done,
    })),
  ]
}

/**
 * Find a child of the view root by name. Used to honour a click made while the petal was
 * still "measuring": the branch has no id yet, so the drill is resolved by name once the
 * real tree arrives.
 */
export function findChildByName(view: ViewNode[], name: string): number | null {
  const root = view[0]
  if (!root) return null
  for (const index of root.children) {
    const node = view[index]
    if (node?.name === name && node.node_id !== null) return node.node_id
  }
  return null
}
