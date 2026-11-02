---
name: retrospective
description: >-
  The standardized end-of-period report: landed work, process findings, a per-lane
  sub-agent review with keep/adjust/scale/retire calls, deviations, a numbered row-call
  table for the principal, and where everything was persisted. Use when a period of work
  closes — a week, a session spanning several increments — and the principal asks for a
  wrap-up. NOT-FOR: the per-run ISA Log entry at a single increment's close; that is
  `increment` step 8's job, and this skill harvests those entries later.
---

# retrospective — the standardized end-of-period report

Retrospectives happen anyway. What did not exist was a **shape**: different periods produced
different shapes for the same nominal activity, so "are the process changes working?" was
unanswerable because the units changed every time. This skill fixes the units.

The report is **source-shaped, not memory-shaped.** Every fact carries its source class; what no
source carries reads `not recorded`. A period report that prints "adversary lane: 6 findings, 2
stalls" over an invented count is the same lie class as a size that was never measured.

## When to use

When a period closes and the principal asks for a retrospective, a period report, or a wrap-up.
NOT-FOR: the per-run entry at one increment's close — that is `increment` step 8, and this skill
harvests those entries later.

## The window

A `(from, to)` pair of **UTC timestamps stated in the report header**, chosen by the principal,
defaulting to the span since the last recorded retrospective, or — when there is none — this
session's span, stated either way. The window is load-bearing: every harvest command in the
report carries the same window strings, so a verifier re-running them gets the same counts.

## Who runs it

The main session, not a sub-agent. It needs the repo, the memory ledger, and principal
interaction for the row-call rulings. A sub-agent may pre-harvest sections 1 and 6 mechanically;
that is an optimization, not the design.

## The honesty rule — source classes

Every section and every numbered row carries a source tag, checkable without trusting the author:

- `[harvested: <exact command>]` — the command, with the window substituted, re-runnable as cited.
- `[narrated: session recollection — unverified]` — remembered, not sourced.
- `not recorded` — no source and no memory: an honest absence.

There is no dispatch log in this repository. Per-lane dispatch counts, stall durations and
brief-quality misses are **not harvestable** — never invent them, and never launder narration
into numbers that look sourced.

## The six sections

### 1. Landed `[harvested]`

- `git log --oneline --since "<from>" --until "<to>" main` — the commits, with merges marked.
- `git log --merges --since … ` — one row per merged increment branch.
- Claims closed in the window: read `ISA.md`'s Log section, which is where step 8 writes them.

### 2. Process findings — what worked, what did not, with evidence

Three harvest lanes:

- `.claude/memory-proposals.drained.jsonl` — every ledger proposal minted in the window, by
  `drained_at`.
- `.claude/memory-proposals.jsonl` — the undrained queue. Undrained is a disclosure, not a
  failure.
- `ISA.md` Log entries and fix-wave commit messages, which name the findings they fold.

Narrated additions are permitted, tagged narrated.

### 3. Sub-agent review — the section nothing else owns

One block per lane dispatched in the period: `increment-designer`, `increment-builder`,
`adversary`, `mechanism-critic`, `code-reviewer`, `standards-governor`, `perf-measurer`.
Verdict and finding yield are `[harvested]` from the period's review records and ISA Log
entries — this section **cites** those records and never re-ledgers them. Dispatch counts and
stalls are `[narrated]` or `not recorded` wherever no entry carries them.

Each lane gets **exactly one call — keep / adjust-brief / scale / retire — with the evidence
that drove it.** A lane with no period data gets `not recorded` fields and **no call**: a call
without evidence is the over-claim this skill exists to prevent.

### 4. Deviations and disclosures

Judgment calls made without explicit sign-off. Seed it `[harvested]` from merges and ISA
deferrals; the narrated residue is the point — this is the formal slot for "did X without
asking."

### 5. Numbered row-call table

Every process recommendation needing the principal's call, numbered (R1, R2, …). Each row
carries: the recommendation, its evidence citation, what adopting it would cost, and **the
falsifier** — the evidence that would show the recommendation is wrong. Rows the evidence does
not support are not offered.

**Zero rows is a legal outcome**, and it must carry the considered-and-declined list with
reasons. An empty table with no negation proof is report theater, and so is a table padded to
look thorough.

### 6. Where everything went `[harvested]`

- `.claude/memory-drain-receipt.json` and `-receipts.jsonl` — counts, `last_run_at`, outcomes.
- `.claude/memory-proposals.jsonl` — the queue.
- `.claude/memory-proposals.drained.jsonl` — the archive: every engram id the drain produced.
- `git log --since "<from>" --until "<to>" -- .claude/ CLAUDE.md ISA.md` — skills, agents and
  doctrine amended in-window.

Pin counts "as at <the drain receipt's timestamp>" so a verifier re-derives at the same moment.

## Persistence, in order

1. **Present the report to the principal in-session.** It is not committed to the repo by
   default: a period report is operational narration, not a pre-commitment proof.
2. **One ledger proposal per retrospective** — the verdict plus the row calls, with the
   principal's rulings appended when they arrive. One concept per memory; review findings are
   cited, never re-proposed.
3. **Accepted row calls become their durable artifacts** — ISA amendments, skill changes, or an
   increment. The report itself stays narration.

## Failure modes to watch for

- **A `[harvested]` tag on a dispatch count or stall figure.** No source exists for those.
- **Report theater.** Narrated fluff filling the format; every period returning a table; rows
  without falsifiers; a zero-row table without its negation proof.
- **A report shaped like memory of the window.** Render from sources: a field you cannot source
  reads `not recorded` even when you remember it.
- **The step-8 pointer dangling.** If this skill moves, `increment` step 8's reference lies.
