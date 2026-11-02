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
- **Merge to `main` while the project is pre-rules** (principal, 2026-11-02: "we are still in
  early development so merge to main is ok until i enable rules in the repo"). Feature work may
  still go on a branch for isolation, but a green branch merging straight back is the default.
  When branch protection or a PR gate is turned on, this line changes first, not after.
- **Test files allow `dead_code` and `unused_imports`; production code does not.** Platform-gated
  tests orphan their helpers on the platforms that skip them, and that failure cannot be
  reproduced on a single machine — it has cost three CI rounds. The allowance is scoped to
  `crates/core/tests/*.rs` with the reason written at the top of each file. It does not extend
  to library or binary code, where `-D warnings` still holds.
- **A platform-gated test gates its imports and helpers too.** Gating the test but leaving its
  `use` or its fixture helper ungated is clean on the developer's machine and a `-D warnings`
  failure on the platforms that skip the test. CI runs the engine on macOS, Ubuntu and Windows
  precisely so this is caught; clippy on one platform is not a gate.
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

## Memory

Findings go in the ledger, not in chat: `node .claude/hooks/memory-propose.mjs` (vault
`diskguardian`). The full contract is `.claude/memory-protocol.md`. Review findings are
mandatory at every severity, including LOW and NIT.

## Where the plan lives

`ISA.md` at the repo root holds the claims, anti-claims and the decisions taken so far.
Read it before proposing a feature; add a claim before building one.
