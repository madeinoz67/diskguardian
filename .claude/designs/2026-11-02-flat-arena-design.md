# C14 — "a home directory scans in seconds": root-cause correction, re-baseline, and the contingent node-record shrink

**Status: design for one increment, written before any number in §8 has been looked at.**
The acceptance rule of §8 is pre-committed; the c14-baseline lane's `~/Library` measurements
had not landed when this document was written, and no threshold in it may move after they do.

**High-risk areas named in the first paragraph, as required.** Slice 1 touches **none** of
the three (physical-size accounting, restricted-space reporting, guarded deletion) — it adds
a measurement probe to an example binary and lands an ISA amendment. The contingent Slice 2
touches **restricted-space reporting** (it reshapes the record that carries `Restricted`'s
`note` and `partial`) and — through the admin helper's results file, which is a serialized
`Tree` crossing a process boundary — the **serialized shape of `Tree`**, the high-risk class
this repo names. Slice 2 is scoped so that the wire shape does not change; a round-trip test
pins that. Nothing in either slice touches deletion or privileges.

---

## 1. Root cause: what the record says, what the code says, and a counted model

### 1.1 The premise as given, and where it is stale

The brief and the ISA agree: C14 is falsified (a full `$HOME` scan ran past 4 minutes at
1.2 GB RSS before it was killed; `~/Library` alone exceeded 90 s), the suspected cause is the
per-level subtree merge — every node copied once per ancestor level, O(depth × nodes) — and
the named fix is "a single flatten into one arena after the walk", recorded under *Not yet
specified*.

**The named fix is already on `main`.** Commit `857d662` ("perf: flatten the scan tree once
and cache sort keys", 2026-10-30 18:19 +0800) replaced the per-directory `Vec<Node>` merge
with the two-phase build that is in the tree today:

- `crates/core/src/scan.rs:11-22` — the module doc records the old defect and the fix.
- `crates/core/src/scan.rs:77-117` — the walk builds a nested `Scanned` tree
  (`scan.rs:130-142`), where a child's descendants sit behind its own `Vec` and are never
  re-copied; then `count_nodes` (`scan.rs:144`) sizes the arena once and
  `Vec::with_capacity` (`scan.rs:117`) allocates it once.
- `crates/core/src/scan.rs:150-190` — `flatten` consumes the `Scanned` tree pre-order,
  moving each node's fields exactly once and assigning ids parent-before-child.

The ISA contradicts itself on this point: the C14 paragraph says the fix is unbuilt, while
the *Decisions* entry "Walker benchmarked against `du`" already quotes the **post-fix**
number (`87.9 s` on `~/Library`). The C14 paragraph and the *Not yet specified* bullet
("Streaming traversal with a flat arena…") were written before `857d662` landed later the
same day and were never corrected. What remains genuinely unspecified there is **streaming**
— not holding every node in memory at once — not the flat arena.

**Conclusion up front: do not build the flat arena. It exists.** This design corrects the
mechanism on the record (a good outcome per this project's rules), proves/refutes the
dominance suspicion, re-baselines C14 against what the filesystem allows, and scopes —
contingently — the one build increment the remaining evidence can still justify.

### 1.2 Was the per-level merge the dominant cost? Refuted, with a model specific enough to be wrong

Three independent legs.

**(a) The controlled A/B already in history.** `857d662`'s commit message measured the same
tree on the same machine before and after: `~/Library`, 974,623 files — **105.9 s → 87.9 s**,
peak RSS 378 MB, and `du -sh` on the same tree takes **79.6 s**. The whole bookkeeping layer
after the fix is therefore ~10 % over the filesystem floor, and the combined change (flatten
+ sort-key caching) recovered 17 % of wall clock. A cause that explains a 4-minute scan
cannot shrink to 17 % when removed.

**(b) The code-counted cost model.** The old `walk_dir` (`git show 857d662^:crates/core/src/scan.rs`)
merged each child's `Subtree.nodes` into the parent's with `nodes.extend(sub.nodes)` plus a
fixup pass rewriting every `children`/`parent` id in the copied range. A node at depth *d*
was therefore moved *d* times and its child ids touched *d* times. For `~/Library`:

- bytes moved ≈ Σ depth × `size_of::<Node>()` ≈ 974k × 10 × ~144 B ≈ **1.4 GB**
  (`Node` at that commit: `String` 24 + `kind` 8 + three `u64` 24 + `Vec<NodeId>` 24 +
  `Option<usize>` 16 + flags 4 + `Option<String>` 24, padded ≈ 144 B; the conclusion holds
  anywhere in 64–256 B and for any mean depth 8–12 — both checkable: `size_of::<Node>()`
  and `Tree::depth_of` over a scanned arena);
- id fixups ≈ 974k × 10 × (1 + children) ≈ 15 M word writes;
- total ≈ **0.3–1.5 s** of memory traffic against a 105.9 s wall clock.

The other change in the same commit, `sort_by_key` → `sort_by_cached_key` (`scan.rs:257-261`),
removed an owned-`OsString` construction per comparison — n·log n allocations per directory,
heavy-tailed on the big directories — and is the plausible bulk of the recovered 18 s. The
two changes are confounded in one commit; §8's decomposition probe splits them if the
maintainer wants the causal record counted rather than modelled.

**Falsifier of the model:** if `size_of::<Node>()` exceeds ~1 KB, or mean depth exceeds ~60
on a home tree, the merge could indeed cost tens of seconds and the suspicion revives. Both
figures are one command each.

**(c) The dominant cost is the walk itself.** `du` takes 79.6 s on `~/Library`; we take
87.9 s. `stat`/`readdir` traffic is the floor, and this project already closed R2 as
"not-a-defect" on exactly that reasoning (the *Corrections* note in *Decisions*). No
in-memory restructuring — arena, streaming, or otherwise — removes a single syscall.

### 1.3 What the 4-minute / 1.2 GB figure actually is

No commit, test, or artifact cites that run. It has no attached provenance: it may include
the webview and IPC transient of the app process, or the pre-`PathBuf`-removal build (which
reached 2.7 GB and "still had not finished", per the *No `PathBuf` per node* decision). The
only controlled measurements in the record are `857d662`'s (105.9→87.9 s, 378 MB on
`~/Library`) and the 417 MB post-`PathBuf` `$HOME` figure. Treat "4 minutes at 1.2 GB" as an
estimate from an unrecorded build; do not design against it.

### 1.4 What is actually still open on C14

Two conjuncts, two different fates:

1. **"completes in under 10 seconds"** — unreachable by any bookkeeping change: `~/Library`
   alone is 79.6 s at the `du` floor. The claim text must be amended (§7). The only
   mechanism that could ever honestly deliver "seconds" is a warm **re**-scan of an unchanged
   tree (mtime-based incremental rescan, R11) — a different claim, deferred.
2. **"does not exceed roughly 1 GB of memory"** — unmeasured at ≥1M-file scope post-fix.
   `~/Library` (974k files) peaked at 378 MB; a full dev `$HOME` (~1.5–2M files) extrapolates
   to ~0.6–1.0 GB before app overhead. **This is the arm that can still fail**, and it is
   what the contingent Slice 2 exists for.

---

## 2. The fork, reasoned from evidence

| Option | Verdict | Why |
|---|---|---|
| Build the flat arena | **Refused — already built** (`857d662`, cited above) | Building it again is churn; the worktree for it should stand down |
| Re-baseline + measure (Slice 1) | **Build** | C14 has no standing probe at all today; the claim needs a rule and a repeatable measurement |
| Node-record shrink (Slice 2) | **Scoped, contingent** | The only remaining mechanism that can move peak RSS materially; activates only if the RSS arm fails |
| Streaming traversal (arena built during the walk) | **Deferred** | True parallel id assignment without the `Scanned` layer is a research project (subtrees complete out of order under work-stealing); only justified if Slice 2 cannot hold ≤1 GB on a multi-million-file volume |
| Incremental rescan (cache) | **Deferred to R11** | It changes what "scan" means; folding it into C14 would hide a cache behind a performance claim |
| Micro-optimize bookkeeping (drop the sort, the double stat, etc.) | **Refused** | ≤8 s at stake above `du`, the sort is the determinism guarantee ("two scans of an unchanged tree agree"), and R2 already closed the stat question as not-a-defect |

---

## 3. The mechanism

### 3.1 What is already in the tree (the mechanism the brief asks for)

The question "how the tree gets from per-branch sub-trees to one flat arena, and why that is
O(nodes)" has a factual answer in the code, so the design records it rather than re-proposing
it:

1. **Walk into a nested form.** `Builder::walk_dir` (`scan.rs:210-340`) returns a `Scanned`
   (`scan.rs:130-142`): a node's children are a `Vec<Scanned>` — an owned subtree behind one
   pointer. Rayon's `par_iter` over subdirectories (`scan.rs:266`) completes subtrees
   independently, but nothing inside a subtree is ever copied to merge: the child `Vec`s are
   collected and pushed (`scan.rs:268-277`) by move.
2. **Count, then allocate once.** `count_nodes` (`scan.rs:144`) walks the in-memory tree
   (no syscalls) and `Vec::with_capacity` (`scan.rs:117`) makes the arena one allocation.
3. **Flatten exactly once.** `flatten` (`scan.rs:150-190`) consumes `Scanned` by value,
   pre-order: a node's fields (including its `name: String`) are moved, never cloned; each
   child is recursed once. `NodeId` is the arena index, assigned in the same pass, so a
   parent's id always precedes its children's — the invariant `is_arena_coherent`
   (`model.rs:331`) checks and `Tree::graft` (`model.rs:248`) relies on.

**Why O(nodes) and not O(depth × nodes):** each `Scanned` is consumed exactly once, so the
total work is one move per node plus one link per edge. The old scheme moved each node once
per ancestor and rewrote its ids each time — the O(depth × nodes) term of §1.2(b). Node
handles become nothing new: `NodeId = usize` into `Tree::nodes`, same as every consumer
already assumes.

### 3.2 Slice 2 (contingent): the node-record shrink

Only if §8's RSS arm fails. The precedent is the *No `PathBuf` per node* decision
(2026-10-30): shrink the per-node record, measure RSS — that one took a `$HOME` scan from
2.7 GB to 417 MB. This is the same move one level down.

Current per-node cost (`model.rs:41-70`): ~144 B inline (`name: String` 24, `kind` 8,
counts 24, `children: Vec<NodeId>` 24, `parent: Option<usize>` 16, flags 4,
`newest_modified: Option<u64>` 16, `note: Option<String>` 24) **plus 1–2 heap allocations
per node** (the name, and a `children` buffer per directory).

Mechanism:

- **Names into one buffer.** `Tree` gains `names: Vec<u8>`; `Node.name` becomes a
  `(u32, u32)` offset/len pair (8 B inline). `Tree::name_of(id) -> &str` replaces field
  access. One allocation for all names of a 2M-file tree (~40 MB) instead of 2M mallocs.
- **Children into one span buffer.** `Tree` gains `child_ids: Vec<NodeId>`; `Node.children`
  becomes a `(u32, u32)` span (8 B inline). `Tree::children_of(id) -> &[NodeId]`. The span
  contents preserve today's order exactly (sorted name order at walk time — the
  determinism guarantee stays).
- **`note: Option<String>` → `Option<Box<str>>`** (16 B inline): only `Restricted` nodes and
  skipped hard links carry one; the 2M-node inline cost is pure waste.
- Net: inline ~144 B → ~88 B, per-node heap allocs 1–2 → ~0. Projected peak RSS on a
  ≥1M-file tree: **down 35–45 %** (arena 288→~175 MB at 2M nodes, name heap collapsed into
  one buffer, and the transient `Scanned` layer shrinks with the same layout change).
- **Wire shape unchanged.** `ResultLine.tree: Tree` (`crates/admin/src/main.rs:96-100`,
  mirrored at `apps/desktop/src-tauri/src/admin.rs:198-202`) is a serialized `Tree` crossing
  a process boundary. `Node`'s JSON must keep the same field names and order; `Tree` gets a
  custom `Serialize`/`Deserialize` that emits/consumes the existing wire node (names and
  child ids materialized from the buffers) via a private `WireNode`. `is_arena_coherent`
  gains span-bounds checks. The `version` field already versioning the protocol is the
  fallback if the maintainer prefers a shape change instead — see §12.

---

## 4. Minimal first slice, and the DEFERS list

**Slice 1 — the re-baseline (this increment's mandatory content).**

1. Extend `crates/core/examples/scan.rs` (already "Used for the numbers in the ISA; not
   shipped") to also print peak RSS: `getrusage(RUSAGE_SELF).ru_maxrss` on Unix (kB on
   Linux, bytes on macOS — normalize and say which), wall clock only on Windows with the
   gap stated in the output. ~15 lines, example-only, no API change.
2. Run the §8 protocol: three paired cold runs (our scan, `du -sk`, alternating) on the
   machine's real `$HOME`, plus the c14-baseline lane's `~/Library` numbers as the
   reference point. Byte counts and file counts only in any record — no home paths.
3. Land the §7 amendment per the outcome. No shipped code changes at all.

**Slice 2 — contingent (activates only on an RSS-arm failure):** the node-record shrink of
§3.2, one PR, with the round-trip and coherence tests of §5.

**DEFERS (explicit, not apologies):**

- **Streaming traversal** — the remaining half of the *Not yet specified* bullet. Not
  justified while Slice 2 holds ≤1 GB.
- **Incremental / warm rescan (R11)** — the only path to a literal "scans in seconds", and
  a separate claim when it opens.
- **Name interning across subtrees; SoA splitting of the scalar fields** — further memory
  wins, only if a multi-million-file volume demands them.
- **Removing the deterministic sort, the per-directory stat, or the double `metadata` read**
  — ≤8 s at stake, each closed or load-bearing (§2).
- **App-side `Tree` residency** — required by C17/C22 (delete and vanish without a rescan);
  the full tree is the product, not waste.
- **Peak-RSS reporting on Windows in the probe** — needs a platform API the example does not
  warrant; stated as a gap, not silently missing.
- **App-process RSS as a gate** — recorded as manual corroboration (the engine example is
  the repeatable gate; the webview's share is not the engine's to budget).

---

## 5. The risk of getting it wrong — and the test that catches each

Slice 2 changes the data structure every module reads (`model.rs`, `view.rs`, `collect.rs`,
`delete.rs`, `classify.rs`, `duplicates.rs`, `commands.rs`, `crates/admin/src/main.rs:447`
`measure()` builds `Node` literals). What could silently break:

| Silent-break candidate | Why it is silent | Test that catches it |
|---|---|---|
| Children order changes (span filled in completion order instead of walk order) | Totals stay right; only petal order and tail-merge membership shift | `view_merges_the_long_tail_into_smaller_objects`, `collector_counts_overlapping_items_once`, `delete_removes_the_node_and_returns_a_smaller_view` (ipc) |
| `path_of` reconstruction drifts | Deletion targets the wrong path only at user action time | `paths_are_reconstructed_from_the_name_chain` |
| Totals drift through accessor churn | A parent summing short is exactly C5's falsifier | `assert_tree_arithmetic` (every core test), `totals_are_consistent_throughout_a_nested_tree`, `marking_a_node_missing_zeroes_it_and_updates_every_ancestor` |
| `Restricted` note/partial/kind semantics change | A dropped `note` leaves a purple petal with no reason | `unreadable_directory_is_reported_not_dropped`, `an_expired_deadline_reports_restricted_nodes_never_zero`, `partial_stays_set_while_a_sibling_is_still_restricted` |
| Hard-link note text lost | C3's zero-size links lose their explanation | `hard_links_are_counted_once` |
| Wire-shape drift (helper protocol) | Only bites on a real elevated pass — the deepest failure mode in the repo | **New:** serialize→deserialize→`graft` equals in-memory `graft` (arithmetic, kind, note, partial), plus `is_arena_coherent` on span bounds; existing `crates/admin` job/serve tests |
| Graft remap walks stale field shapes | Corrupts an arena from a privileged pass | `graft_replaces_a_restricted_node_and_preserves_arithmetic`, `graft_refuses_a_subtree_scanned_from_a_different_root` |

Slice 1 risks only that the probe lies: `ru_maxrss` units differ across platforms (normalize
and print the unit) and peak RSS includes allocator retention (report `ru_maxrss`, not
derived sums). A probe that misreports is worse than no probe; the du-pairing in §8
cross-checks wall clock against an independent oracle for exactly this reason.

---

## 6. Surfaces that must move

- **Slice 1:** `crates/core/examples/scan.rs` only. Not the public API
  (`crates/core/src/lib.rs` re-exports untouched), not the IPC surface, not the frontend,
  not the serialized shapes. `ISA.md` moves only by the maintainer landing §7.
- **Slice 2:** `crates/core/src/model.rs` (layout, accessors, custom serde),
  `crates/core/src/scan.rs` (`Scanned` → `Node` construction), and mechanical accessor
  swaps in `view.rs`, `collect.rs`, `delete.rs`, `classify.rs`, `duplicates.rs`,
  `apps/desktop/src-tauri/src/commands.rs`, `crates/admin/src/main.rs`. Frontend: **none** —
  `ViewNode` (`view.rs:12-28`) is the IPC shape and it is unchanged. Serialized `Tree`:
  unchanged by requirement (§3.2). README/`docs/how-it-works.md`: only where they describe
  `Node`'s storage.

---

## 7. Claim and anti-claim impact

**Moves — C14 (amendment required; maintainer lands this text, I do not edit `ISA.md`):**

> ### C14 — A real home directory scans at the filesystem's own pace, within memory budget `[ ]`
>
> A cold scan of a developer's home directory (≥1M files) completes within **1.15×** the wall
> clock of `du -sk` over the same tree, and the scan engine peaks below **1 GB** of RSS.
> **Falsifier:** a home scan whose wall clock exceeds 1.15× `du` on the same tree, or whose
> peak RSS exceeds 1 GB.
> **Retired reading (2026-11-02):** the original "under 10 seconds" target is unreachable —
> `du` itself takes ~80 s on `~/Library` alone and the walker is within 10 % of that floor
> (857d662) — so no bookkeeping change can reach it. A *warm re-scan* of an unchanged tree in
> seconds is a different claim and belongs to R11 (incremental rescan).
> **Root-cause correction (2026-11-02):** the per-level subtree merge is not the dominant
> cost. It was real (each node moved once per ancestor, `nodes.extend` + id fixups,
> 857d662^), but removing it together with the sort-key caching recovered 17 % of wall clock
> on `~/Library` (105.9 s → 87.9 s) where `du` takes 79.6 s. The walk is the cost; the fix
> is on `main` since 857d662, not "recorded under Not yet specified".
> **Probe:** `cargo run --release -p diskguardian-core --example scan -- <home>` reporting wall
> clock and peak RSS, paired with `du -sk` on the same tree; three paired cold runs, medians
> (protocol in `.claude/designs/2026-11-02-flat-arena-design.md` §8).

And the *Not yet specified* bullet "**Streaming traversal with a flat arena**, so a scan of a
multi-million-file volume does not hold every node in memory at once" is corrected to:
"**Streaming traversal** (not holding every node in memory at once) — the flat-arena-after-walk
half landed 2026-10-30 (`857d662`); what remains is streaming."

**Proven unchanged (each has tests that must stay green):**

- **C2 (physical sizes)** — `allocation()` (`scan.rs`) is untouched by both slices;
  `reports_physical_allocation_not_logical_length` stays green.
- **C5 (tree arithmetic)** — no arithmetic moves in either slice; `assert_tree_arithmetic`
  over every core fixture is the gate for Slice 2.
- **C4 (restricted space)** — Slice 1 untouched; Slice 2 must keep `Restricted` kind, `note`
  and `partial` semantics bit-identical (tests listed in §5).
- **C15 (progressive paint)** — `BranchDone`/`scan-progress` (`scan.rs:200-208`, emitted at
  `scan.rs:276-283`, `scan.rs:335-338`) is a separate shape from `Node` and is untouched;
  walk order and sort stay, so petal growth is unchanged.
- **C3 (hard links)** — dedup is walk-side (`claim_inode`, `scan.rs:384`); Slice 2 keeps the
  zero-size-link `note` visible.
- **C17/C22 (delete/missing without rescan)** — `mark_missing`/`recompute_from`
  (`model.rs:104-165`) are accessor churn only under Slice 2.
- **C23 (admin graft)** — Slice 2 touches the serialized `Tree` of the helper protocol; this
  is the one genuinely high-risk move, mitigated by the unchanged wire shape + the new
  round-trip test + the existing `version` field.

**Anti-claims:** A1 (guarded deletion) and A5 (no elevation) are untouched by construction —
neither slice adds a delete call site or a privilege. A2/A3 are the invariants themselves and
move only if a test in §5 fails, which is what those tests are for. A4 (UI never blocks) is
untouched: the scan still runs on `spawn_blocking` and still emits per-branch progress.

---

## 8. Measurable acceptance criteria

Each criterion names its probe; "it works" is not a criterion.

| # | Criterion | Probe | Falsifier |
|---|---|---|---|
| AC1 | The walker's wall clock is at the filesystem floor on a real home | `examples/scan` vs `du -sk`, three **paired alternating** cold runs on full `$HOME` (≥1M files), medians; the c14-baseline lane's `~/Library` numbers are the same-protocol reference | median ratio > 1.15 |
| AC2 | Peak RSS of the scan engine stays under budget on a real home | `ru_maxrss` from the same runs (unit normalized and printed); `~/Library`'s recorded 378 MB at 974k files is the scaling reference | peak > 1.0 GB |
| AC3 | Root cause of the old cost is established, not assumed | decomposition on `~/Library` (3 runs each, medians): `857d662^` as recorded (105.9 s), `857d662^` + only the sort-key fix, `857d662` as recorded (87.9 s). The flatten's share = (sort-only variant) − 857d662 | see interpretation rule below |
| AC4 | Slice 2, if activated, shrinks RSS without breaking anything | full `cargo test -p diskguardian-core` + `crates/admin` tests + `cargo test -p diskguardian-app --test ipc` + the new serde round-trip test, then a repeat of AC2 | any §5 test red, or RSS reduction < 25 % |

**Pre-committed acceptance rule (written 2026-11-02, before any number is looked at).**
*Metrics:* median wall-clock ratio (ours ÷ `du -sk`) and peak RSS over three paired
alternating cold runs; *sample:* one real developer `$HOME` of ≥1M files, with the
`~/Library` run as the paired reference point (a second machine only if the run is ruled
underpowered below). *Both kill directions:*

- **Ships** — AC1 ratio ≤ 1.15 **and** AC2 ≤ 1.0 GB: the §7 amendment lands, C14 ticks, and
  Slice 2 is cancelled.
- **Kills / activates** — AC2 > 1.0 GB: Slice 2 (§3.2) is activated and C14 stays open until
  the repeat of AC2 passes; AC1 > 1.15 while AC2 passes: the efficiency conjunct is
  re-opened for profiling first (the §1.2 model would be wrong) and **no** slice is built
  until a profile names the cost.
- **Underpowered, not conclusive** — the tree holds < 500k files (a pass cannot decide the
  1 GB bound; measure a bigger tree), or the three-run medians vary by > 10 % in either arm
  (cache state not controlled; re-run quiet/cold once — after two noisy runs the arm is
  reported as unmeasurable on this machine). No threshold moves after any number is seen.
- **AC3 is not a gate** — it refines the causal record whichever way it lands: flatten share
  < 1/3 of the 18 s → "merge minor, recorded"; ≥ 2/3 → "merge dominant, the ISA's suspicion
  vindicated and its fix credited"; between → "mixed, both recorded". The ship/kill outcome
  is identical in all three cases, because the `du` floor already forbids the 10-second
  reading in every case.

---

## 9. Top risks, each with what would falsify the design

1. **The RSS arm fails and Slice 2's effect size is optimistic.** The 35–45 % projection is
   arithmetic on `size_of` deltas; allocator retention can eat it. Falsified if the AC4
   repeat of AC2 is still > 1.0 GB; the fallback is streaming traversal (deferred, §2) or a
   further amendment of the memory bound with the number attached.
2. **The wire shape drifts and only a real elevated pass notices** (the C23 graft path). The
   round-trip test is designed to catch it in `cargo test`; falsified if a real admin pass
   ever grafts wrong arithmetic — which is why `is_arena_coherent` and `assert_tree_arithmetic`
   run on the grafted tree too.
3. **The du-pairing is unfair or unrepeatable** (cache state, `du`'s hard-link behavior vs
   ours). Falsified if paired runs vary > 10 % after one quiet re-run — that is the
   underpowered clause, and the claim is then amended to name the measurement limit rather
   than closed on noise.
4. **Slice 2's accessor churn subtly reorders children.** Falsified by the §5 order-sensitive
   tests; if one goes red the slice stops, since petal order is user-visible behaviour the
   tests legitimately pin.
5. **The baseline lane's numbers arrive scoped differently** (app-process RSS vs engine, warm
   vs cold) and cannot be evaluated against this rule. Falsified on arrival; the fallback is
   to run the §8 protocol as written rather than reinterpret their numbers.

---

## 10. What I would refuse to build

- **Re-implementing the flat arena** — it is `857d662`. The `c14-arena` worktree should stand
  down or be repurposed to Slice 2.
- **A scan cache disguised as a performance fix** — incremental rescan is R11 and a separate
  claim; folding it in would make C14 unfalsifiable ("fast" against a cache is not "fast").
- **Dropping the deterministic sort** — two scans of an unchanged tree agreeing is a
  correctness property the tests and the users rely on; 8 s is not worth it.
- **Storing absolute paths per node again** — the measured 2.7 GB mistake; `path_of`
  reconstructs from the name chain for a reason.
- **Streaming traversal before the RSS number demands it** — the deferred item stays deferred
  until a measured volume forces it.

---

## 11. Precedent

- **`857d662` itself** — the flatten being extended "one level down" (record layout) is the
  same discipline: count, allocate once, move once.
- **The *No `PathBuf` per node* decision** (2026-10-30) — shrink the per-node record and
  measure RSS; the exact class of Slice 2, with a 2.7 GB → 417 MB precedent.
- **R2 closed as not-a-defect** (2026-11-02 correction) — the `du`-floor argument this design
  reuses; a bookkeeping fix for a syscall-bound cost is refused there and here.
- **R3's pre-committed rule** (`.claude/designs/2026-11-02-admin-scanning-design.md` §8) —
  the ship / kill / underpowered shape §8 follows verbatim.
- **`is_arena_coherent` + `Tree::graft`** — the arena discipline any Slice-2 span change must
  extend, not relax.

---

## 12. Open questions for the maintainer

1. **C14 wording.** §7's re-baseline (Option A, recommended) vs keeping the seconds target
   and deferring the whole claim to an R11 warm-rescan claim (Option B). Option A closes C14
   on existing behaviour this week; Option B keeps a claim open that no cold scan can pass.
2. **AC3's decomposition probe** — three builds of old code for a causal footnote. Worth it
   for the record, or does the cost model + `857d662`'s recorded A/B settle it? (It is not a
   gate either way.)
3. **The memory bound's measurement point.** The 1 GB figure is measured on the engine
   example; the app process holds the `Tree` plus the webview. If the engine reads 0.9 GB,
   the app will read more. Recommend: gate on the engine, record the app run as corroboration
   (§4 defers), and let a future claim budget the app process.
4. **If Arm M fails narrowly (1.0–1.2 GB)**, is Slice 2's target "under 1 GB" or "under
   0.75 GB to leave the app headroom"? Recommend 0.75 GB for the slice, so the claim's 1 GB
   holds at the app level too.
5. **The stale ISA paragraphs** (C14's "recorded under *Not yet specified*", the bullet
   itself) — the amendment of §7 replaces both; nothing else in the ISA cites the merge
   theory, so the blast radius is those two paragraphs plus a Log entry.
