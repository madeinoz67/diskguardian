import { describe, expect, test } from 'bun:test'
import { assignHues, computeGeometry, hitTest, radiusOf } from './layout'
import type { ViewNode } from './types'

function node(index: number, name: string, bytes: number, children: number[] = []): ViewNode {
  return {
    index,
    node_id: index,
    name,
    bytes,
    kind: 'dir',
    children,
    aggregated_count: 0,
    partial: false,
  }
}

// Array position must equal `index`: children refer to positions, and a fixture that
// disagrees with itself tests the fixture, not the layout.
const view: ViewNode[] = [
  node(0, 'root', 1000, [1, 2, 3, 4]),
  node(1, 'big', 500, [5, 6]),
  node(2, 'medium', 300),
  node(3, 'small', 150),
  node(4, 'tiny', 50),
  node(5, 'big-a', 300),
  node(6, 'big-b', 200),
]

describe('geometry', () => {
  test('children tile the circle with no gaps and no overlap', () => {
    const geometry = computeGeometry(view, 800, 800)
    const top = geometry.arcs.filter((arc) => arc.depth === 1).sort((a, b) => a.start - b.start)

    expect(top).toHaveLength(4)
    expect(top[0]!.start).toBeCloseTo(0, 9)
    expect(top[top.length - 1]!.end).toBeCloseTo(Math.PI * 2, 9)

    for (let i = 1; i < top.length; i += 1) {
      expect(top[i]!.start).toBeCloseTo(top[i - 1]!.end, 9)
    }
  })

  test('each arc angle is proportional to its bytes', () => {
    const geometry = computeGeometry(view, 600, 400)
    const parentTotal = 1000
    for (const arc of geometry.arcs.filter((a) => a.depth === 1)) {
      const span = arc.end - arc.start
      const expected = (Math.PI * 2 * (view[arc.node]!.bytes / parentTotal))
      expect(span).toBeCloseTo(expected, 9)
    }
  })

  test('deeper rings stay inside the parent span', () => {
    const geometry = computeGeometry(view, 800, 800)
    const parent = geometry.arcs.find((arc) => arc.node === 1)!
    const children = geometry.arcs.filter((arc) => arc.node === 5 || arc.node === 6)
    expect(children).toHaveLength(2)
    for (const child of children) {
      expect(child.start).toBeGreaterThanOrEqual(parent.start - 1e-9)
      expect(child.end).toBeLessThanOrEqual(parent.end + 1e-9)
      expect(child.depth).toBe(2)
    }
  })
})

describe('hit testing', () => {
  test('every arc is found by a point inside it', () => {
    const geometry = computeGeometry(view, 800, 800)
    for (const arc of geometry.arcs) {
      const mid = (arc.start + arc.end) / 2
      const radius = radiusOf(geometry, arc.depth - 0.5)
      const x = geometry.cx + Math.sin(mid) * radius
      const y = geometry.cy - Math.cos(mid) * radius
      const hit = hitTest(geometry, x, y)
      expect(hit?.node).toBe(arc.node)
      expect(hit?.depth).toBe(arc.depth)
    }
  })

  test('the centre and the outside return nothing', () => {
    const geometry = computeGeometry(view, 800, 800)
    expect(hitTest(geometry, geometry.cx, geometry.cy)).toBeNull()
    expect(hitTest(geometry, 1, 1)).toBeNull()
    expect(hitTest(geometry, 2000, 2000)).toBeNull()
  })
})

describe('colours', () => {
  test('top-level branches get distinct hues and children inherit', () => {
    const hues = assignHues(view)
    const top = hues.slice(1, 5)
    expect(new Set(top).size).toBe(4)
    expect(hues[5]).toBe(hues[1])
    expect(hues[6]).toBe(hues[1])
  })
})
