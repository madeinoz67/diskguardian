// check-isa-claims — a ticked claim must carry its evidence.
//
// BenchWeave does not need this. Its record is GitHub objects: an issue is closed
// because it IS closed, a PR merged because it WAS merged. Nothing in prose can claim
// otherwise. diskguardian's record is `ISA.md`, where "closed" is a checkbox a person
// ticks — and on 2026-11-02/03 that checkbox was ticked for a claim with no evidence
// (C27, before its screenshot existed) and left unticked for one that had been verified
// (C8, while the commit message said otherwise).
//
// So this checks the one property a prose record can be made to hold mechanically:
// every `[x]` claim carries a line that names how it was checked. A claim marked done
// with nothing to point at is the defect, whatever the author intended.
//
// Not checked, stated here rather than implied: whether the evidence is *true*. This
// finds a missing probe, never a lying one. The review lanes own that.
import { readFileSync } from 'node:fs'

/** Markers that count as evidence inside a ticked claim. */
const EVIDENCE = [
  /^\*\*Probe:\*\*/m,
  /^\*\*Verified\b/m,
  /^\*\*Closed\b/m,
  /^\*\*Measured\b/m,
  /^\*\*Evidence:\*\*/m,
]

export interface Claim {
  id: string
  title: string
  ticked: boolean
  /** The claim's own text, from its heading to the next heading of the same rank. */
  body: string
  line: number
}

/** Split ISA.md into claims. Headings look like `### C27 — Title  `[x]``. */
export function parseClaims(markdown: string): Claim[] {
  const lines = markdown.split('\n')
  const claims: Claim[] = []
  let current: { id: string; title: string; ticked: boolean; line: number; body: string[] } | null =
    null

  const flush = (): void => {
    if (!current) return
    claims.push({
      id: current.id,
      title: current.title,
      ticked: current.ticked,
      line: current.line,
      body: current.body.join('\n'),
    })
    current = null
  }

  lines.forEach((line, index) => {
    const heading = /^###\s+(C\d+)\s+—\s+(.*?)\s*(`\[[ x]\]`)?\s*$/.exec(line)
    if (heading) {
      flush()
      current = {
        id: heading[1] ?? '',
        title: heading[2] ?? '',
        ticked: heading[3] === '`[x]`',
        line: index + 1,
        body: [],
      }
      return
    }
    if (/^#{1,3}\s/.test(line)) {
      flush()
      return
    }
    if (current) current.body.push(line)
  })
  flush()
  return claims
}

/** Claims marked done that name nothing for how they were checked. */
export function unsupportedTicks(claims: Claim[]): Claim[] {
  return claims.filter(
    (claim) => claim.ticked && !EVIDENCE.some((marker) => marker.test(claim.body)),
  )
}

/**
 * Claims ticked in `current` but not in `base` — the ratchet.
 *
 * Ten claims in this file were ticked before the rule existed, and demanding evidence of
 * them at once would either block everything or invite invented probe names. So the gate
 * enforces the rule on what this change newly closes, and `--all` audits the backlog when
 * someone picks it up. A gate that cannot land is not a gate.
 */
export function newlyTicked(current: Claim[], base: Claim[]): Claim[] {
  const wasTicked = new Set(base.filter((claim) => claim.ticked).map((claim) => claim.id))
  return current.filter((claim) => claim.ticked && !wasTicked.has(claim.id))
}

function main(): number {
  const args = process.argv.slice(2)
  const all = args.includes('--all')
  const baseIndex = args.indexOf('--base')
  const path = args.find((arg) => !arg.startsWith('--') && !/^[0-9a-f]{7,40}$/.test(arg)) ?? 'ISA.md'

  const claims = parseClaims(readFileSync(path, 'utf8'))
  const ticked = claims.filter((claim) => claim.ticked).length

  if (claims.length === 0) {
    console.error(`check-isa-claims: no claims parsed from ${path} — the parser or the file is wrong`)
    return 1
  }

  let scope = claims
  let scopeNote = 'all claims'
  if (!all) {
    const baseRef = baseIndex >= 0 ? args[baseIndex + 1] : undefined
    if (!baseRef) {
      console.error('check-isa-claims: pass --all to audit every claim, or --base <ref> to check only')
      console.error('what this change newly closes. Refusing to guess which mode you meant.')
      return 2
    }
    const baseText = Bun.spawnSync(['git', 'show', `${baseRef}:${path}`])
    if (baseText.exitCode !== 0) {
      console.error(`check-isa-claims: cannot read ${path} at ${baseRef}`)
      return 2
    }
    const base = parseClaims(baseText.stdout.toString())
    scope = newlyTicked(claims, base)
    scopeNote = `newly ticked since ${baseRef} (${scope.length} of ${ticked} ticked)`
  } else {
    scope = claims.filter((claim) => claim.ticked)
  }

  const bad = unsupportedTicks(scope)
  if (bad.length > 0) {
    console.error(
      `check-isa-claims: ${bad.length} claim(s) closed by this change name no probe, no verification and no evidence — ${scopeNote}:`,
    )
    for (const claim of bad) {
      console.error(`  ${path}:${claim.line}  ${claim.id} — ${claim.title}`)
    }
    console.error('\nA claim is closed on evidence, or it is not closed. Add the probe line,')
    console.error('or untick it and say what is still missing.')
    return 1
  }

  console.log(`check-isa-claims: ${scopeNote} — every claim closed here carries evidence`)
  return 0
}

if (import.meta.main) process.exit(main())
