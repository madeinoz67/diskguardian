---
name: panel
description: >-
  Run competing positions against each other and come out with a DECISION, not a pile of
  opinions. Use when a genuine fork has to be resolved and the evidence does not obviously
  pick a side — an architecture question, a design that returned two defensible shapes, or a
  measurement that contradicts a closed claim. Independent panelists, a decision rule
  written before anyone runs, and a judge whose powers are deliberately bounded.
---

# panel — dialectical inquiry that terminates

The value is not "more opinions." It is **independence followed by reconciliation against a
rule written in advance**, which is what stops the exercise from becoming a summary nobody can
act on.

## When it is allowed to run

Any ONE of these qualifies. Nothing else does:

1. A **doctrine fork** — a decision that changes what the product means, is expensive to
   reverse, or contradicts an anti-claim (A1–A6) or a closed claim in `ISA.md`.
2. **The designer returned two or more defensible shapes** and could not choose between them
   on the available evidence.
3. **A measurement contradicts a closed claim** or a recorded decision.

Anything else: take the defensible default and proceed, exactly as `increment` step 2 says.
Three independent runs plus a judge is a real cost. **Do not run a panel on trivia, and do not
run one to avoid making a call you could make.**

## The procedure

### 1. Write the question and the decision rule FIRST

Before any panelist runs, write into `.claude/designs/`:

- the exact question, phrased so it can be answered;
- what result picks A, what picks B, what would pick something else;
- what result means **the panel was the wrong instrument** and this needs a measurement or an
  owner decision instead.

A rule written after the arguments arrive is not a rule, it is a rationalization.

Two disqualifiers apply before argument starts, and any panelist may invoke them: a shape that
**breaks a product invariant** (sizes must be physical allocation; unmeasurable space must be
reported rather than dropped) is out, and a shape that **cannot be verified by a probe this
repo could run** cannot win. Say so in the record rather than letting a panelist argue for it.

### 2. Three independent panelists, with different MANDATES

Not different personalities. Different jobs:

- **Argue A** on the strongest available evidence.
- **Argue B** on the strongest available evidence.
- **Reject both and find a third shape.**

That third mandate is not a courtesy; it is the one that most often produces the answer.
Run them **concurrently and blind** — no panelist sees another's output, and where possible use
different models, because the independence is the whole point. Every panelist reads the real
code, `ISA.md` and the design record, and cites `file:line`. A panelist reasoning from the brief
alone is producing an opinion, and opinions are what this procedure exists to avoid.

### 3. The judge, with bounded powers

The judge may do exactly three things:

1. **Mark each claim** verified, refuted, or unverifiable — against the live code, the
   invariants and the claim record, never against plausibility. Unverifiable claims are
   **discarded**, not weighed.
2. **Apply the pre-written rule** to what survives.
3. **DEFER to the principal** when the rule does not discriminate, saying precisely what
   additional evidence would.

**The judge may not introduce a new option.** That is what panelist three was for. A judge that
invents an answer has skipped the independence the procedure is built on.

### 4. Output a decision, and keep the losing arguments

Append to the design record: the decision, the rule that produced it, and **the arguments that
lost, with their strongest points intact.** A panel that ends in a summary has failed. The losing
case is the most valuable artifact months later when someone asks why the project went this way,
and it is what makes the decision reversible on evidence rather than on memory. Where the outcome
changes what the product claims, it becomes an ISA amendment when it lands — and the amendment
names the panel, so the reasoning is one hop away.

## Failure modes to watch for

- **Panelists converging because they read each other.** Blind, or the result is one opinion
  wearing three hats.
- **A judge that splits the difference.** Most forks have no coherent midpoint; averaging two
  designs usually produces one with neither's virtues.
- **A rule vague enough to justify anything.** If you cannot say in advance what would change
  your mind, you are collecting support, not running a panel.
- **Running it to diffuse responsibility.** The decision is still the principal's. The panel
  informs it; it does not launder it.

## See also

`increment` step 2 is where a fork is noticed; this skill is what happens when it cannot be
defaulted. If this file moves, that reference lies — re-check it.
