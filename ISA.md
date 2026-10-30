---
title: diskguardian
principal_stated_goal: "I want to setup a rust project called diskguardian, will be an open source disk space visualizer and free up space, cross platform. ... will be a graphical ui, primary for macos. ... research disk daisy, I want it to be identical"
phase: climbing
progress: 12/17
iteration: 1
created: 2026-10-30
---

# diskguardian

## Goal

An open-source, cross-platform disk space visualiser written in Rust, with a graphical UI, macOS-first, whose look, feel and functionality match DaisyDisk.

## Vision

Open it and the disk draws itself as a flower: every folder a coloured petal, every file grey, petal width exactly the space it occupies. Click a petal and you fall into it, click the middle and you climb back out. Hover and the right-hand column tells you what you are looking at. Drag the big ones into a collector, watch the number climb, then delete once — and get the space back. No wizard, no categories, no sixty-step cleanup script. The user sees where the space went in one glance, and it costs them nothing but a download.

## Out of Scope (v0.1)

- Admin / root scanning. Core loop must work with user privileges only.
- Cloud accounts: Dropbox, Google Drive, OneDrive, Box.
- APFS local snapshot discovery and purge.
- Disk image (.dmg) mounting and scanning.
- Network volumes.
- Windows and Linux builds. The code is written portable and cross-platform, but only macOS is tested and shipped at v0.1.
- APFS clone detection (`ATTR_CMNEXT_CLONEPARENT`). Hard links are deduplicated; clones are not, yet.

## Anti-claims (what must NOT be true)

- **A1.** It must never delete anything the user did not explicitly put in the collector.
- **A2.** It must never present logical file length where physical allocated size is the meaning.
- **A3.** It must never silently drop unreadable directories from a total as if they were zero.
- **A4.** The UI must never block during a scan; the window stays responsive at all times.
- **A5.** The core loop must never require elevated privileges.
- **A6.** It must never render an arc whose angular width is unrelated to its allocated size.

## Claims (ideal state criteria)

Each claim names the probe that would falsify it.

### C1 — Workspace builds clean  `[x]`
`cargo build --workspace` succeeds on macOS arm64 and `cargo clippy --workspace --all-targets -- -D warnings` is silent.
**Falsifier:** any compile error or clippy diagnostic.

### C2 — Physical, not logical, sizes  `[x]`
A sparse fixture file whose logical length is far larger than its allocation is reported at its allocated size, taken from `st_blocks * 512` (Unix `MetadataExt::blocks`), never `len()`.
**Falsifier:** fixture test where reported bytes equal logical length.

### C3 — Hard links counted once  `[x]`
A fixture with several hard links to a single inode reports exactly one file's allocation for that inode, and the extra names appear as zero-size links.
**Falsifier:** fixture test where total equals N × single-file allocation.

### C4 — Restricted space is visible, not hidden  `[x]`
A directory with mode 000 yields a `Restricted` node carrying a count of the entries it could not measure; the parent's total is flagged as partial.
**Falsifier:** fixture test where the unreadable directory is absent from the tree or reported as a plain empty folder.

### C5 — Tree arithmetic holds  `[x]`
For every node, `allocated(node) >= sum(allocated(children))`, and the remainder equals the total allocation of entries the node itself could not descend into.
**Falsifier:** any walk of a real volume where a parent sums smaller than its children.

### C6 — Arcs are proportional and non-overlapping  `[x]`
In the rendered layout, each visible node produces exactly one arc, arcs tile the circle without gaps or overlap, and each arc's angle over 2π equals its allocated bytes over the sibling total to within 0.1%.
**Falsifier:** layout unit test that disagrees, or a screenshot showing overlapping petals.

### C7 — Hit test round-trips  `[x]`
For every rendered arc, hit-testing a point inside it returns that node; hit-testing outside the ring returns none.
**Falsifier:** property test over generated layouts.

### C8 — Navigation works both directions  `[ ]`
Clicking a petal re-roots the view to that node; clicking the centre returns to the previous root, restoring it exactly.
**Falsifier:** driving the app and observing an unchanged or wrong view.

### C9 — Collector total is exact  `[x]`
The collector's displayed total equals the sum of allocated bytes of the collected set, and collecting an ancestor together with one of its descendants counts the child once.
**Falsifier:** unit test on the collect set.

### C10 — Deletion is guarded  `[x]`
Deletion refuses system trees (`/`, `/System`, `/usr/bin`, `/usr/lib`, `/bin`, `/sbin`, `/private/etc`, `/private/var/db`, `/Library/Apple`) and the scan root itself, while still allowing `/usr/local` and the temp directory, and requires an explicit confirmation when anything collected is a direct child of the scan root (whatever its size) or the total reaches 5 GB.
**Falsifier:** guard unit test that allows a protected path, or a top-level folder deleted
without asking.
**Probe:** `cargo test` — `deletion_guard_refuses_protected_locations`,
`confirmation_is_required_for_top_level_folders_whatever_their_size` (a 4 KB `Desktop` asks,
a small nested folder does not, size alone still triggers). The verdict is computed in
`delete.rs` and returned to the UI as `needs_confirmation`, so the frontend cannot forget it.

### C11 — Deletion is recoverable by default  `[x]`
The default action moves collected items to the macOS Trash, and a file deleted that way is present in `~/.Trash` afterwards.
**Falsifier:** integration test where the item is gone and absent from Trash.

### C12 — It is a real app that draws the disk  `[x]`
`bun run tauri:build` produces `DiskGuardian.app`, launching it scans a volume (the home directory by default; `DISKGUARDIAN_ROOT` overrides) and shows a non-empty flower, verified by a screenshot whose pixels are inspected.
**Falsifier:** no app bundle, blank window, or absent petals.

### C13 — Scanning does not freeze the window  `[ ]`
While a scan of the home volume runs, the window still paints and the UI reports progress; a second scan of another path can be started concurrently.
**Falsifier:** window beachballs or progress stops updating during the scan.

### C14 — A real home directory scans in seconds  `[ ]`
A scan of a developer's home directory completes in under 10 seconds and does not exceed
roughly 1 GB of memory.
**Measured 2026-10-30:** 51,518 files scan in 446 ms (115k files/s), so the walker itself is
fast. But `~/Library` alone exceeds 90 s, and a full `$HOME` scan ran past 4 minutes at
1.2 GB RSS before it was killed. **Falsified.** The suspected cause is the per-level
subtree merge, which copies every node once per ancestor level; the fix is a single
flatten into one arena after the walk, which is recorded under *Not yet specified*.

### C15 — Petals appear while the scan runs  `[x]`
The flower is drawn from the root listing alone, with every branch shown as a "measuring"
petal, and each petal grows to its true width the moment that branch's subtree is measured.
Nothing waits for the last file.
**Probe:** `bun test` on `apps/desktop/src/partial.ts` (equal nominal petals, a completed
branch flipping to a folder at its real width, the root total always equal to the sum of
the petals) plus `scan-progress` events emitted per branch from `crates/core/src/scan.rs`.
**Falsifier:** first paint only after the scan resolves.
**Open:** the in-window appearance of the growth was not captured this run, because browser
automation is unavailable (see C8). The logic is tested; the animation is not photographed.

### C16 — A click while measuring is honoured, not dropped  `[ ]`
Clicking a petal before its branch has finished records the choice and opens that branch the
moment its report lands, instead of doing nothing. Clicking elsewhere or Escape cancels, and
the deferred choice is cleared at the start of every scan.
**Probe:** `bun test` on `findChildByName` (a measuring branch resolves to a real node id, a
synthetic "smaller objects" child never resolves, an unknown name returns nothing).
**Falsifier:** a click during the measuring phase that discards the user's choice.
**Status:** the name-resolution half is tested (13/13 pass). The end-to-end click is not yet
verified in the running app, so the claim stays open until it is.
**Note:** true queue-jumping is not offered, because rayon already walks every branch of the
root concurrently; the honest behaviour is "open on arrival", with no second walk.

### C17 — Deleting refreshes the flower without re-walking the disk  `[x]`
After a delete the backend prunes the deleted nodes from the tree, recomputes every
ancestor's totals and returns a fresh view, so the petal disappears and the totals shrink
immediately. The default action moves items to the macOS Trash; holding Option on the Delete
button removes them permanently, which always asks first.
**Probe:** `cargo test` on `Tree::forget` (deleted node leaves its parent, every ancestor
loses exactly the deleted bytes, tree arithmetic still holds, the scan root cannot be
forgotten) plus the existing Trash-recovery test.
**Falsifier:** a delete that triggers a full rescan, or a parent whose total does not shrink.

## Test Strategy

| Layer | Probe |
|---|---|
| Scan engine arithmetic | `cargo test -p diskguardian-core` on synthetically built fixtures |
| Layout arithmetic | frontend unit tests (`bun test`) on the pure layout + hit-test functions |
| Integration | `cargo test -p diskguardian-core` — `crates/core/tests/scan.rs` builds real fixtures on disk |
| Appearance | launch the built app, `screencapture`, view the pixels |
| Behaviour | drive the running app (hover, drill, collect, delete) and re-read state |

## Decisions

- **Tauri 2 + Rust core over egui or SwiftUI** (2026-10-30, principal elected). Rust owns scanning, tree building, deletion and macOS integration; the system webview owns the canvas. Chosen because visual fidelity to DaisyDisk is a rendering problem the web canvas solves directly, while the hard cross-platform part (physical-size accounting, traversal) stays portable Rust.
- **Workspace, not one crate.** `crates/core` has no UI dependencies so it is unit-testable and reusable by a future CLI.
- **Physical sizes via `st_blocks`.** `len()` would over-report sparse files and under-report block rounding.
- **Trash by default, permanent as an option.** DaisyDisk deletes permanently; an open-source tool should not, by default.
- **Layout computed in the frontend.** Rust ships the tree and sizes; angles are cheap and belong next to the renderer that draws them.
- **No `PathBuf` per node** (2026-10-30, measured). The first build stored an absolute path on every node; a scan of the home directory reached 2.7 GB of RSS and still had not finished. Paths are now reconstructed from the name chain by `Tree::path_of`, which is called a handful of times per user action instead of millions of times per scan. Same scan: 417 MB.
- **Progressive paint, not lazy loading** (2026-10-30, principal asked whether to scan on demand). Lazy expansion cannot draw this product's first frame: a petal's width IS its subtree's total, so from one `readdir` every folder would render at near zero and a 50 GB folder would look identical to an empty one. Lazy also re-walks a subtree on every re-entry unless cached, which is a full scan with extra steps. The scan now emits each root child the moment its subtree finishes and the UI draws equal-width "measuring" petals that grow into place, so the total cost is unchanged and the perceived wait collapses to about a second.
- **Walker benchmarked against `du`, not a target** (2026-10-30, `du -sh ~/Library` = 79.6 s vs our 87.9 s on 974,623 files). The walk is within 10% of the filesystem floor for that tree, so the original "10 seconds" C14 target was invented rather than measured. Remaining known headroom is a redundant per-directory `stat`, visible as double `du`'s kernel time.
- **Delete prunes, it does not rescan** (2026-10-30). The first working version re-scanned the whole volume after every delete, which is minutes on a real disk and made the feature feel broken. `Tree::forget` now removes the deleted nodes, recomputes ancestors and re-projects the view, so the flower updates in the same frame.
- **Trash by default, Option for permanent** (2026-10-30). The button moves to the Trash; holding Option deletes permanently and always confirms. Trash runs over 5 GB also confirm.
- **Narrow deletion guard** (2026-10-30, from a failing test that was right). Protecting `/private` and `/usr` wholesale refused the user's own temp directory (through the `/tmp` symlink) and every Homebrew cache. The guard now names the system's own trees, and a second test pins the narrow behaviour so a future "fix" cannot re-widen it.

## Not yet specified

In scope eventually, too dim to state as claims today:

- Streaming traversal with a flat arena, so a scan of a multi-million-file volume does not hold every node in memory at once.
- APFS clone detection (`ATTR_CMNEXT_CLONEPARENT`), the largest known accuracy gap.
- Incremental rescan: reusing a previous tree and re-walking only paths whose mtime changed.
- Admin/root scanning and the privileged helper that would carry it.
- Animation on drill-down, which DaisyDisk has and v0.1 does not.

## Log

- 2026-10-30 — Progressive paint shipped (`d59013e`). Evidence: `bun test` 10/10 including four new tests on `partial.ts`; `cargo test` 10/10; clippy silent; bundle rebuilt and launched. Not photographed: the growth animation itself, because browser automation is unavailable this run. C15 closed on the layout tests; C14 still open and now measured against `du`.

- 2026-10-30 — Verification: `cargo test -p diskguardian-core` 10/10, `cargo clippy --all-targets -D warnings` silent, `bun test` 6/6, frontend build clean, and the built app renders a real 4.2 GB / 51,518-file tree (`/tmp/dg3.png`, viewed). C8 stays open: browser automation is unavailable because the Interceptor test profile is unset, and the install is browser-only, so no click was driven end to end.

- 2026-10-30 — Run opened. DaisyDisk feature surface captured from its own guide (sunburst structure, colour semantics, hover/click navigation, collector, physical-size accounting, excluded hard links/clones, restricted-space handling, virtual "smaller objects" and "hidden space" rows).
- 2026-10-30 — Engine complete and tested (10 tests). Two real defects found by tests and fixed: the deletion guard was too broad, and `file_count` double-counted every file. A third was found by running the app: the empty-state overlay ignored the `hidden` attribute because a CSS `display` rule outranked it.
