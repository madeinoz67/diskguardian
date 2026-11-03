# CLAUDE.md — diskguardian

**An open-source, cross-platform disk-space visualiser with a GUI, macOS-first, modelled on
DaisyDisk.** Rust workspace: `crates/core` is the UI-free scanning engine, `apps/desktop` is
the Tauri 2 application (Rust shell + TypeScript canvas frontend).

## The two invariants

Everything in this repo is judged against these. Break either and the product is lying to
the user about their disk.

1. **Sizes are physical allocation**, `st_blocks * 512`, never logical file length. Sparse
   files occupy less; the last block is rounded up.
2. **Unmeasurable space is reported**, never dropped. A directory the process cannot read
   becomes a `Restricted` node with its reason, and its parent is flagged `partial`.

## Commands

```bash
cargo test -p diskguardian-core          # engine tests (real filesystem fixtures)
cargo clippy -p diskguardian-core --all-targets -- -D warnings
cd apps/desktop && bun test              # sunburst layout maths
cd apps/desktop && bunx tsc --noEmit     # frontend types
cd apps/desktop && bun run tauri:dev     # the app
cd apps/desktop && bun run tauri:build   # the bundle
```

## Layout

- `crates/core/src/scan.rs` — parallel walker. `entry.metadata()` is used deliberately: it
  does not follow symlinks, so a link costs only its own metadata.
- `crates/core/src/model.rs` — `own_bytes` (this node's own space) and `total_bytes`
  (own plus all children). The tree invariant `total = own + Σ children` is tested.
- `crates/core/src/view.rs` — display pruning. Hundreds of thousands of nodes cannot be
  drawn; big children are kept, the tail merges into one "smaller objects" petal.
- `crates/core/src/delete.rs` — the guard. Narrow on purpose: it protects system trees, not
  the prefixes they live under, so the temp directory and Homebrew caches stay cleanable.
- `apps/desktop/src/layout.ts` — pure geometry. No DOM. This is the testable half of the
  renderer; `sunburst.ts` only paints it.

## Rules

- **Never widen the delete guard while "fixing" a test.** If a guard test fails, decide
  whether the guard or the fixture is wrong, and say which in the commit message.
- **Never make the UI right by changing the engine's arithmetic.** The engine is the truth;
  the view is a projection.
- **Work on a branch; `main` is merged into, never committed to** (changed 2026-11-03, after the
  first release; supersedes "merge to main while the project is pre-rules"). Open a PR, and merge
  it **only** through `scripts/merge-verified.sh`, which watches the checks to terminal state,
  prints the COMPLETE rollup and refuses on any fail, pending or empty result. `gh pr checks |
  tail` is not the rollup — it hides failing lanes and masks the exit code. The old rule is what
  let a Windows compile break and an ext4-only assertion reach `main`: the local battery proves
  things about this machine, and nothing read CI before the merge.
- **Test files allow `dead_code` and `unused_imports`; production code does not.** Platform-gated
  tests orphan their helpers on the platforms that skip them, and that failure cannot be
  reproduced on a single machine — it has cost three CI rounds. The allowance is scoped to
  `crates/core/tests/*.rs` with the reason written at the top of each file. It does not extend
  to library or binary code, where `-D warnings` still holds.
- **A platform-gated test gates its imports and helpers too.** Gating the test but leaving its
  `use` or its fixture helper ungated is clean on the developer's machine and a `-D warnings`
  failure on the platforms that skip the test. CI runs the engine on macOS, Ubuntu and Windows
  precisely so this is caught; clippy on one platform is not a gate.
- **A closed claim names how it was checked.** `scripts/check-isa-claims.ts` runs in CI: a claim
  this change newly ticks must carry a `**Probe:**`, `**Verified**`, `**Closed**`, `**Measured**`
  or `**Evidence:**` line, or the lane fails. It checks that evidence is *named*, never that it is
  true — the review lanes own that. `--all` audits the historical ticks.
- **No "should work".** Engine claims close on `cargo test` output; appearance claims close
  on a screenshot whose pixels someone looked at.
- **Parallel work happens in a worktree, never in the primary checkout.** Every dispatched
  agent and every second session gets its own `git worktree`; the primary tree is the
  integration checkout and stays on `main`. This is not tidiness: on 2026-11-02 an agent
  switched the shared checkout onto its own branch, the main session's commits landed there,
  and a `git add -A` swept the agent's half-finished work into a commit about something else.
  Nothing about that was visible from either side until the tests went red.
- **Stage explicit paths, never `git add -A`,** in any tree that another process may be
  writing to.
- **Non-trivial work runs the `increment` loop** (`.claude/skills/increment/SKILL.md`):
  design, build RED-first, vet independently, refute with the agent lanes, measure, merge.
  The lanes are not optional garnish — on 2026-11-02 four fixes went straight from gates to
  merge without them, and three independent review passes then found sixteen findings,
  including a fix that did not fix its own defect. Gates prove it compiles; they never try
  to break it.

## Memory

Findings go in the ledger, not in chat: `node .claude/hooks/memory-propose.mjs` (vault
`diskguardian`). The full contract is `.claude/memory-protocol.md`. Review findings are
mandatory at every severity, including LOW and NIT.

## Where the plan lives

`ISA.md` at the repo root holds the claims, anti-claims and the decisions taken so far.
Read it before proposing a feature; add a claim before building one.
