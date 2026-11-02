import { describe, expect, test } from 'bun:test'
import { advance, applyBranch, findChildByName, NOMINAL, partialView, startBranches } from './partial'
import { computeGeometry } from './layout'

describe('progressive paint', () => {
  test('unmeasured branches draw as equal petals and all say they are measuring', () => {
    const branches = startBranches(['a', 'b', 'c'])
    const view = partialView('root', branches)

    expect(view).toHaveLength(4)
    expect(view[0]?.children).toEqual([1, 2, 3])
    for (const node of view.slice(1)) expect(node.kind).toBe('measuring')

    const geometry = computeGeometry(view, 800, 800)
    const arcs = geometry.arcs.filter((arc) => arc.depth === 1)
    expect(arcs).toHaveLength(3)
    const spans = arcs.map((a) => a.end - a.start)
    for (const span of spans) expect(span).toBeCloseTo((Math.PI * 2) / 3, 9)
  })

  test('a completed branch switches to a folder and takes its real width', () => {
    const branches = startBranches(['big', 'small'])
    expect(applyBranch(branches, 'big', 900)).toBe(true)
    expect(branches[0]?.done).toBe(true)

    // Ease to rest.
    for (let i = 0; i < 200 && advance(branches); i += 1) {
      /* spin */
    }

    const view = partialView('root', branches)
    expect(view[1]?.kind).toBe('dir')
    expect(view[1]?.bytes).toBe(900)
    expect(view[2]?.kind).toBe('measuring')

    const geometry = computeGeometry(view, 800, 800)
    const big = geometry.arcs.find((a) => a.node === 1)
    // big 900, small still nominal 1: nearly the whole circle.
    expect(big!.end - big!.start).toBeGreaterThan(Math.PI * 1.9)
  })

  test('an unknown branch is ignored rather than added', () => {
    const branches = startBranches(['a'])
    expect(applyBranch(branches, 'ghost', 5000)).toBe(false)
    expect(branches).toHaveLength(1)
    expect(branches[0]?.target).toBe(NOMINAL)
  })

  test('the root total always equals the sum of the petals', () => {
    const branches = startBranches(['x', 'y'])
    applyBranch(branches, 'x', 4096)
    const view = partialView('root', branches)
    const sum = view.slice(1).reduce((s, n) => s + n.bytes, 0)
    expect(view[0]?.bytes).toBe(sum)
  })
})

describe('deferred drill', () => {
  const view = [
    { index: 0, node_id: 1, name: 'root', bytes: 10, kind: 'dir' as const, children: [1, 2], aggregated_count: 0, partial: false, missing: false },
    { index: 1, node_id: 7, name: 'Library', bytes: 6, kind: 'dir' as const, children: [], aggregated_count: 0, partial: false, missing: false },
    { index: 2, node_id: null, name: 'smaller objects', bytes: 4, kind: 'aggregate' as const, children: [], aggregated_count: 9, partial: false, missing: false },
  ]

  test('resolves a measuring branch to its real node id', () => {
    expect(findChildByName(view, 'Library')).toBe(7)
  })

  test('a synthetic child with no id is never a drill target', () => {
    expect(findChildByName(view, 'smaller objects')).toBeNull()
  })

  test('an unknown name resolves to nothing rather than guessing', () => {
    expect(findChildByName(view, 'Ghost')).toBeNull()
  })
})
