import { describe, expect, test } from 'bun:test'
import { parseClaims, unsupportedTicks } from './check-isa-claims'

const ISA = `# diskguardian

## Claims

### C1 — Something checked  \`[x]\`
It holds.
**Probe:** \`cargo test\` — a_named_test.

### C2 — Something ticked with nothing behind it  \`[x]\`
It is claimed to hold.

### C3 — Still open  \`[ ]\`
Not yet.
`

describe('parsing the claim record', () => {
  test('ticked, unticked and their bodies come out separately', () => {
    const claims = parseClaims(ISA)
    expect(claims.map((c) => c.id)).toEqual(['C1', 'C2', 'C3'])
    expect(claims[0]?.ticked).toBe(true)
    expect(claims[2]?.ticked).toBe(false)
    expect(claims[0]?.body).toContain('a_named_test')
  })

  test('a body stops at the next claim, not at the end of the file', () => {
    const claims = parseClaims(ISA)
    expect(claims[1]?.body).not.toContain('a_named_test')
  })
})

describe('the rule', () => {
  test('a tick with no evidence is the failure, and it names the line', () => {
    const bad = unsupportedTicks(parseClaims(ISA))
    expect(bad.map((c) => c.id)).toEqual(['C2'])
    expect(bad[0]?.line).toBe(9)
  })

  test('every evidence shape counts, not just Probe', () => {
    for (const marker of [
      '**Probe:** x',
      '**Verified 2026-11-02 on the built app.** x',
      '**Closed 2026-11-02.** x',
      '**Measured 2026-10-30:** x',
      '**Evidence:** x',
    ]) {
      const doc = `### C9 — T  \`[x]\`\n${marker}\n`
      expect(unsupportedTicks(parseClaims(doc))).toHaveLength(0)
    }
  })

  test('an open claim needs no evidence, so the untick is always available', () => {
    const doc = '### C9 — T  `[ ]`\nNothing here.\n'
    expect(unsupportedTicks(parseClaims(doc))).toHaveLength(0)
  })
})
