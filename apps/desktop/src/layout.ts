// Pure geometry. No DOM, no canvas: this is the part that can be tested.
//
// One ring per depth. Within a node, children tile the parent's angular span strictly
// in proportion to their allocated bytes, sorted largest first. Angle 0 is 12 o'clock
// and grows clockwise.

import type { ViewNode } from './types'

export interface Arc {
  node: number
  depth: number
  start: number
  end: number
}

export interface Geometry {
  cx: number
  cy: number
  inner: number
  ring: number
  rings: number
  arcs: Arc[]
}

export function maxDepth(view: ViewNode[]): number {
  let depth = 0
  const walk = (index: number, level: number): void => {
    depth = Math.max(depth, level)
    const node = view[index]
    if (!node) return
    for (const child of node.children) walk(child, level + 1)
  }
  walk(0, 0)
  return depth
}

export function computeGeometry(view: ViewNode[], width: number, height: number): Geometry {
  const size = Math.min(width, height)
  const cx = width / 2
  const cy = height / 2
  const inner = size * 0.085
  const outer = size * 0.475
  const rings = Math.max(1, maxDepth(view))
  const ring = (outer - inner) / rings

  const arcs: Arc[] = []
  const root = view[0]
  if (root) {
    layoutChildren(view, 0, 0, Math.PI * 2, arcs)
  }

  return { cx, cy, inner, ring, rings, arcs }
}

function layoutChildren(
  view: ViewNode[],
  index: number,
  start: number,
  end: number,
  arcs: Arc[],
  depth = 1,
): void {
  const node = view[index]
  if (!node) return
  const children = [...node.children].sort((a, b) => (view[b]?.bytes ?? 0) - (view[a]?.bytes ?? 0))
  const total = children.reduce((sum, child) => sum + (view[child]?.bytes ?? 0), 0)
  if (total <= 0) return

  let cursor = start
  for (const child of children) {
    const span = ((end - start) * (view[child]?.bytes ?? 0)) / total
    const childEnd = cursor + span
    arcs.push({ node: child, depth, start: cursor, end: childEnd })
    layoutChildren(view, child, cursor, childEnd, arcs, depth + 1)
    cursor = childEnd
  }
}

/** Angle in radians, 0 at 12 o'clock, growing clockwise. */
export function angleOf(dx: number, dy: number): number {
  const raw = Math.atan2(dx, -dy)
  return raw < 0 ? raw + Math.PI * 2 : raw
}

/** Which arc, if any, sits under a point in canvas coordinates. */
export function hitTest(geometry: Geometry, x: number, y: number): Arc | null {
  const dx = x - geometry.cx
  const dy = y - geometry.cy
  const radius = Math.hypot(dx, dy)
  const depth = Math.floor((radius - geometry.inner) / geometry.ring) + 1
  if (depth < 1 || depth > geometry.rings) return null

  const angle = angleOf(dx, dy)
  for (const arc of geometry.arcs) {
    if (arc.depth === depth && angle >= arc.start && angle < arc.end) return arc
  }
  return null
}

/** Radius of a depth, for drawing and for the centre disc. */
export function radiusOf(geometry: Geometry, depth: number): number {
  return geometry.inner + geometry.ring * depth
}

/** A hue per depth-1 branch, inherited by everything below it. */
export function assignHues(view: ViewNode[]): number[] {
  const hues = new Array<number>(view.length).fill(210)
  const palette = [142, 48, 12, 205, 268, 320, 88, 178, 25, 300]
  const root = view[0]
  if (!root) return hues

  const paint = (index: number, hue: number): void => {
    const node = view[index]
    if (!node) return
    hues[index] = hue
    for (const child of node.children) paint(child, hue)
  }

  const branches = [...root.children].sort((a, b) => (view[b]?.bytes ?? 0) - (view[a]?.bytes ?? 0))
  branches.forEach((child, i) => paint(child, palette[i % palette.length] ?? 210))
  return hues
}

export function colorFor(node: ViewNode, hue: number, depth: number): string {
  // A deleted item is a ghost: visible enough to strike through, obviously not live.
  if (node.missing) return 'hsl(220 6% 26%)'
  switch (node.kind) {
    case 'restricted':
      return 'hsl(276 68% 52%)'
    case 'aggregate':
      return 'hsl(220 8% 62% / 0.42)'
    case 'measuring':
      return 'hsl(220 8% 34%)'
    case 'file':
    case 'symlink':
      return `hsl(${hue} 10% ${58 + Math.min(depth, 4) * 3}%)`
    default: {
      const lightness = 50 + Math.min(depth, 5) * 5
      return `hsl(${hue} 62% ${lightness}%)`
    }
  }
}
