---
name: increment
description: The repeatable build loop diskguardian holds itself to — design, build RED-first, vet independently, adversarially refute, measure on a real tree, then merge. Use for any non-trivial feature or fix ("build X", "add Y", "fix Z", "next increment"). Encodes the discipline that keeps the app honest about what is on the disk, instead of merely compiling.
---

# increment — the diskguardian build loop

Design before code, RED before GREEN, refute before merge, measurement before a claim.
The north star is the product's own two invariants: **sizes are physical allocation**, and
**unmeasurable space is reported, never dropped**. Everything below exists because a gate
proves the code compiles and the happy path works; it never tries to break it.

## When to use

Any non-trivial feature or fix. A one-line typo, just do it. Anything touching size
accounting, restricted reporting, the deletion guard, the view pruning, the scan walk, the
privileged path, or a claim in `ISA.md`, run the loop.

## The non-negotiables (the lens)

1. **Measure, don't claim.** Every increment names a number before it is built and reports
   it after. "It works" is not a result; "1,056,336 nodes in 21.5 s at 388.8 MB against
   `du`'s 44.0 s" is.
2. **Never trust a sub-agent's green.** Re-run the gates yourself and read the counts from
   the output, never from a summary line. This has caught two agent-reported greens that
   were not.
3. **Read the output of a gate, never its echo.** `cargo fmt --all -- --check >/dev/null &&
   echo OK` hid a real diff for two CI rounds.
4. **Appearance claims close on a screenshot whose pixels someone looked at**, and behaviour
   claims close on driving the built app. A passing test does not tell you what the window
   shows.
5. **Defer-if-buggy over ship-to-hit-a-scope.** If refutation finds a real defect that is
   not cleanly fixable here, ship the clean half and name the deferral in the ISA with its
   reopen trigger.
6. **Honest negatives are first-class.** A measurement that shows no value, or a claim that
   turns out badly worded, is a result. Say it and correct the record.
7. **Every claim names its falsifier**, and a claim that cannot be checked in one command is
   not finished.

## The loop

1. **Design.** Spawn `increment-designer` grounded in the real code — it reads, it does not
   assume — plus `ISA.md` (claims, anti-claims, decisions, roadmap) and `CLAUDE.md`. It
   delivers: the mechanism, the minimal first slice with explicit deferrals, which claims
   and anti-claims move, the measurable proof with a **pre-committed acceptance rule**, the
   top risks, and its build-or-DON'T-BUILD recommendation. Designs live in `.claude/designs/`
   and are committed so the acceptance rule is provably pre-committed.
2. **Decide.** Read the design. Surface any genuine product fork to the principal; otherwise
   take the defensible default and say which you took. If the design refutes its own premise,
   that is the finding — say so before building anything. A fork that cannot be defaulted
   (doctrine, two defensible shapes, a measurement contradicting a closed claim) goes to the
   **`panel`** skill, which reaches a decision against a rule written before it ran.
3. **Build, RED-first.** `increment-builder` in a fresh worktree under `.worktrees/` (never
   the primary checkout). Every behaviour change lands with a test that fails without it,
   and the dispatch carries the evidence for that failure. Gate line for every commit:
   **`cargo fmt --all -- --check` (read the diff output), `cargo clippy --workspace
   --all-targets -- -D warnings`, `cargo test --workspace`, and from `apps/desktop`
   `bunx tsc --noEmit` + `bun test`.** Full battery once immediately before merge.
4. **Vet (you, independently).** Re-run all five. Then RED-check that the new guards
   discriminate — disable the mechanism and confirm the test fails — using a `cp` backup,
   never `git checkout` over uncommitted work.
5. **Refute.** `adversary` with a REFUTE mandate is **mandatory** on any change to size
   accounting, restricted-space reporting, the deletion guard, or the pruning merge, and on
   any claim marked at risk. Add `mechanism-critic` whenever arithmetic, a threshold, a
   rounding rule, or a partial/unmeasured distinction moves. Add `code-reviewer` for
   ordinary correctness across the diff, and `standards-governor` when ISA claims, pinned
   thresholds, or version/lockfile surfaces move. A lane that never ran is a skipped gate,
   not a pass. Scale the slate to the diff; the refute scales down to `adversary` alone on a
   small change, never to nothing.
6. **Measure.** `perf-measurer` on a **real tree**, aggregate numbers only, against the
   pre-committed rule. Correctness of the instrument comes first: prove the measurement can
   see what it claims to see (a deliberately wrong fixture, an RSS sanity probe) before
   quoting it.
7. **Merge.** Open a PR, and merge it **only** through `scripts/merge-verified.sh`: it watches
   the checks to terminal state, prints the COMPLETE rollup, and refuses on any fail, pending or
   empty result before merging. The local battery is a pre-filter, not the gate — it can only
   prove things about this machine, and on 2026-11-03 a Windows compile break and an ext4-only
   assertion both reached `main` behind a locally-green report. Every real finding folded or
   deferred by name before the PR opens.
8. **Run close.** Remove the worktree, delete the merged local branch, and record in the ISA
   what changed, what was verified and how, and what stayed open. A claim closed on a
   screenshot says which screenshot. That entry is the raw material for the **`retrospective`**
   skill, which harvests a period's worth of them; do not run a period report at a run close.

## Traps this repo has already paid for

- **`stage-admin.ts` copies from a hard-coded `../../target`.** Building a worktree with
  `CARGO_TARGET_DIR` set fails at the copy step.
- **A fresh worktree needs `bun install --frozen-lockfile` and `bun run stage:admin`** before
  the gates will run at all.
- **Tests written from one platform fail on the other two.** Gate platform-specific helpers
  and imports, not just the test.
- **`symlink_metadata` reports EPERM exactly like "missing".** Never let an existence check
  refuse a whole job.
- **The screen locks mid-verification.** A claim whose appearance was not captured stays
  open; it is not closed on tests.

## The bar

An increment is done when its claim is closed on evidence a stranger could re-run, its
deferrals are named with reopen triggers, and the ISA says what is now true that was not
before. Anything less is a commit, not an increment.
