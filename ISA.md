---
title: diskguardian
principal_stated_goal: "I want to setup a rust project called diskguardian, will be an open source disk space visualizer and free up space, cross platform. ... will be a graphical ui, primary for macos. ... research disk daisy, I want it to be identical"
phase: climbing
progress: 19/27
iteration: 1
created: 2026-10-30
---

# diskguardian

## Goal

An open-source, cross-platform disk space visualiser written in Rust, with a graphical UI, macOS-first, whose look, feel and functionality match DaisyDisk.

## Vision

Open it and the disk draws itself as a flower: every folder a coloured petal, every file grey, petal width exactly the space it occupies. Click a petal and you fall into it, click the middle and you climb back out. Hover and the right-hand column tells you what you are looking at. Drag the big ones into a collector, watch the number climb, then delete once — and get the space back. No wizard, no categories, no sixty-step cleanup script. The user sees where the space went in one glance, and it costs them nothing but a download.

## Out of Scope (v0.1)

- ~~Admin / root scanning~~ — **moved in scope as R3** (see C23). The core loop still works
  with user privileges only; elevation is an optional, consented extra.
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
- **A5.** The core loop must never require elevated privileges. *(Interpretive note, 2026-11-02: still true. R3 adds an optional privileged pass taken only with the user's OS-level consent; refuse it and everything except the restricted petals works as before.)*
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
**Scope note (2026-11-02):** dedup is per walk, and it is Unix-only — inode identity keys on
`(dev, ino)` and Windows has no inode through std, so on Windows no name is marked skipped
and hard-linked bytes are counted once per name. Stated, not hidden; the test is gated to
Unix for that reason rather than passing vacuously. An elevated pass walks restricted roots
separately, so a hard link spanning a restricted root and a readable tree counts twice. A
stated limit, not a weakening within a single walk.

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

> **Correction 2026-11-02:** this claim was closed on unit tests of `Tree::forget` alone and
> was false end to end. `delete_collected` aligned deletion outcomes to node ids by path,
> and `guard` canonicalises paths (`/var/...` becomes `/private/var/...` on macOS), so the
> lookup missed, nothing was pruned, and "freed" reported a directory's metadata size. The
> principal saw it as "delete does nothing". Now aligned by position, and pinned by
> `delete_removes_the_node_and_returns_a_smaller_view` in `src-tauri/tests/ipc.rs`, which
> drives the real command on a mock runtime.
After a delete the backend prunes the deleted nodes from the tree, recomputes every
ancestor's totals and returns a fresh view, so the petal disappears and the totals shrink
immediately. The default action moves items to the macOS Trash; holding Option on the Delete
button removes them permanently, which always asks first.
**Probe:** `cargo test` on `Tree::forget` (deleted node leaves its parent, every ancestor
loses exactly the deleted bytes, tree arithmetic still holds, the scan root cannot be
forgotten) plus the existing Trash-recovery test.
**Falsifier:** a delete that triggers a full rescan, or a parent whose total does not shrink.

### C18 — Collecting from the list actually works  `[x]`
Clicking the `+` beside a legend row adds that node to the collector: the collector amount
rises, the row switches to a tick, the Delete button enables, and the petal is outlined.
**Reported failing 2026-10-30 by the principal:** clicking `+` changed nothing.
**Current probe:** the app now calls `collected` straight after every scan, so a miswired
multi-argument command raises a visible banner instead of failing silently on first click;
`showError` surfaces any collect failure in a dismissible red banner.
**Falsifier:** a `+` click that leaves the collector at 0 bytes.
**Suspected class:** every command that takes more than one argument (`collect`, `view`,
`delete_collected`, `reveal`, `quicklook`) is unexercised; `scan` and `peek_root`, which take
one or none, are known good. That points at the IPC argument binding, not the handler.
**2026-11-02 — that hypothesis is now refuted.** `src-tauri/tests/ipc.rs` builds the app's
real command surface on Tauri's mock runtime and calls it with the exact JSON the frontend
sends: `collect` returns `count: 1` and the right total, `collect` with `add: false` clears
it, `view` returns a pruned tree, and an unknown scan is a typed error. Five tests, all
passing. The backend of this feature works; the defect is in the frontend click path, and
the new banner exists to name it. Also fixed here: collecting the scan root itself is now
refused, because the guard will never delete it, so it only built a collector that could not
be emptied.
**2026-11-02 — CONFIRMED, and the frontend was never broken.** One click of the `+` beside
`alpha` in the contents list of the built app, scanned against a two-folder fixture: the
collector went from `0 bytes` to `4 MB` with `in the collector · 1 item`, the row's `+`
became `✓`, the Delete button went from grey to enabled red, and the petal drew its
collected outline. The four sub-claims all held in the one interaction, so nothing was
changed to make it pass. What was actually wrong on 2026-10-30 was the delete path — the
misaligned node ids of C17 — and the report "collecting does nothing" was made while that
defect was live. The fix for C17, and the IPC suite that arrived with it, are the whole
story; the inferred IPC argument-binding fault never existed.
**Evidence:** `/tmp/dg-c18.png` (screenshot, pixels inspected). The commands behind the
click were already pinned by `src-tauri/tests/ipc.rs`.
**Standing probe for regressions:** `cargo test -p diskguardian-app --test ipc` — the
`collect` tests fail loudly if the command surface changes under the button.

### C19 — The scan target is chosen, not assumed  `[x]`
The app opens on a "Disks and folders" screen listing every mounted volume with a usage gauge
and a free-space figure, plus the home folder; each row has a Scan button. A folder can also
be chosen with the system picker, or dropped onto the window. The breadcrumb leads back to
that screen from any scan.
**Probe:** `bun test` on `overview.ts` (half-empty is healthy, a quarter is tight, a tenth is
critical, nonsense numbers clamp instead of drawing off the bar, `basename` handles both
separators); the volume list comes from `sysinfo`, so it is per-platform rather than `df`.
**Falsifier:** an app that can only scan one hard-coded root.

### C20 — The view stops claiming things that are gone  `[x]`
Regaining window focus re-stats the current level and prunes nodes whose paths no longer
exist, reporting how many vanished and the new total. Deletions made outside the app
(Finder, terminal) therefore stop being drawn as if they were still on disk.
**Probe:** `revalidate` command walks the current node's children with one `Path::exists`
per child and calls `Tree::forget` on the missing ones; the UI calls it on focus, throttled
to once every 4 seconds, and reports the count in the status line.
**Falsifier:** a file deleted in Finder still drawn after the window regains focus.
**Note:** this is deliberately shallow, one level at a time, because a deep re-stat on every
focus would cost as much as a rescan. Deeper staleness is refreshed when you drill in, or by
Rescan.

### C21 — The space actually comes back  `[x]`
Because deletion moves to the Trash, the overview measures the Trash and offers to empty it,
naming the size before doing so. Without this the product promises space it never returns.
**Probe:** `trash_stats` walks the platform's trash (`.Trash` on macOS, `~/.local/share/Trash`
or `$XDG_DATA_HOME/Trash` on Linux, `Clear-RecycleBin` on Windows) and reports bytes and item
count; `empty_trash` refuses without `confirmed`, removes each entry, and reports what it
freed. The UI confirms with the measured size and re-renders the overview afterwards.
**Falsifier:** an app that can only move files into a Trash it never helps clear.
**Note:** emptying the platform trash is global and irreversible, which is why it asks with a
number attached rather than a bare confirm.

### C22 — A deleted item is struck through, not silently gone  `[x]`
Deleting (or losing) an item leaves it in the flower, marked `missing`, drawn with a strike
across the petal and a struck-through name in the list, worth zero bytes and excluded from
every ancestor's total. A rescan clears the ghosts.
**Probe:** `cargo test` — `marking_a_node_missing_zeroes_it_and_updates_every_ancestor`
(the node stays listed, is flagged, totals drop by exactly its subtree) and the IPC test
`delete_strikes_the_node_through_and_shrinks_the_totals` (65536 bytes removed, not a
directory's metadata size). The view gives a missing node a floor width of parent/200 so it
stays visible while counting as zero.
**Falsifier:** a deletion the user cannot see in the flower, or one that leaves the totals
unchanged.

### C23 — Restricted space is measurable with consent  `[ ]`
With the user's explicit, OS-mediated consent, the areas drawn as purple petals are measured
and grafted into the same tree, under the same physical-allocation rules as everything else.
**Design:** `.claude/designs/2026-11-02-admin-scanning-design.md` (prompt-per-scan, because
this repo has no signing identity and `SMAppService` will not register an unsigned helper on
macOS 13+).
**Probes (pre-committed, design §8):** graft arithmetic against a readable twin fixture; the
helper as root against `sudo du -sk` within 1%, plus a RED control where the same pipeline
unprivileged must recover exactly zero; mock-runtime IPC tests including injection
invariance; one manual human run. Real-machine sizing: ship at >= 1 GB recovered, kill R3
below 100 MB, and treat 100 MB to 1 GB as underpowered.
**Falsifier:** a restricted root reported at a size the filesystem does not corroborate, or
any root-level process receiving a path chosen by the UI.
**Honest limit:** root does not bypass TCC, so Full Disk Access-protected petals (Mail,
Messages) may stay restricted. Reported, never hidden.
**Status 2026-11-02 (merged, 40d239b):** AC1 and AC3 closed — graft arithmetic including the
readable-twin tolerance, and 14 IPC tests covering the declined dialog, an unknown scan, a
mismatched root, helper stderr, and hostile names that stay inert in the job file while the
dialog source and argv remain byte-identical to the fixed template. The helper is
mechanically pinned as having no delete callsites. **Not closed:** AC2's elevated arm (needs
sudo, which the builder correctly refused to automate) and AC4 (a manual human run). The
ship/kill sizing rule therefore has no real-machine number yet.
**Cadence gap closed on a branch, not on main:** `r3-session-scanning` (`fd853b1`, rebased)
implements one prompt per app run, with the three liveness mechanisms and their tests. It is
green (ipc 16, serve 6, job 10, model 5, scan 18) and unmerged, because merging it would ship
a privileged behaviour that has not yet run against a real dialog on a real machine.

### C24 — Duplicate groups are content-identical, never a space promise  `[x]`
A duplicate pass over a finished scan (never during one) groups only files whose whole-file
BLAKE3 digests match, after a free size-grouping step and a 16 KiB head/tail pre-filter;
hard-linked inodes are excluded entirely (both the zero-byte extra names and, by a fresh
`nlink` check, the name that carries the bytes); files below the caller's floor are never
opened; and every reported group carries the caveat that content-identical files may share
physical storage (APFS clones), so deleting a member can free less than the group's
`wasted_bytes`. A pass stopped by its deadline or candidate cap reports `partial` rather
than pretending completeness, and a vanished scan root is an error rather than a silent
"no duplicates".
**Probe:** `cargo test -p diskguardian-core --test duplicates` — 11 tests, including a
same-size pair differing only in the 4 KiB band between the hashed head and tail (the
fixture that catches an implementation trusting the edges), hard-link exclusion at
`min_bytes: 0`, and a progress-callback seam proving a file without a size twin is never
opened. Each honesty property was verified RED by neutralising its mechanism before the
implementation was trusted.
**Falsifier:** any reported pair whose whole files differ, a hard-linked name proposed as a
duplicate, a below-floor file hashed, a spent deadline reported as complete, or a group
shown without its caveat.
**Known gaps:** APFS clones are not detected (no `ATTR_CMNEXT_CLONEPARENT`, roadmap R6),
which is why the caveat rides every group. Grouping starts from the tree's allocated
sizes, so a same-content pair whose *allocations* differ (an HFS+-compressed file beside a
raw copy of the same bytes) is not detected — a stated limit, not a silent behaviour, and
one that can only produce false negatives, never a false "duplicate" claim.

### C25 — An unreadable root is a result, not a refused job  `[x]`
A path the platform will not show (SIP, TCC) is measured as best it can be and reported as
restricted for that one root, instead of failing the whole pass.
**Probe:** `cargo test -p diskguardian-admin` —
`an_unreadable_path_becomes_a_restricted_result_not_a_refusal` (a missing path yields a
restricted result and `completed: 2`) and `malformed_paths_are_refused_before_anything_is_walked`
(relative and `..` paths are still refused as malformed).
**Falsifier:** a single protected root aborting a multi-root pass.
**Why it happened (2026-11-02, live):** `symlink_metadata` reports EPERM exactly as it reports
"missing", and SIP-protected roots answer EPERM even to root. A 57-path pass was refused
because `/private/var/db/ConfigurationProfiles/Store` was invisible, which is the opposite of
the rule that unmeasurable space is reported rather than dropped. Validation is now shape-only
(absolute, no `..`); readability belongs to the walk.

### C26 — Duplicates are shown with their caveat, never as a space promise  `[x]`
A panel beside Cleanup runs the duplicate pass over the finished scan and shows every group
with the engine's own caveat as **visible text**, the newest copy marked `keep`, and a `+` on
each other copy that goes through the ordinary collector and the unchanged guard. A pass that
ran out of time says so, rather than reading as "no duplicates".
**Probe:** `cargo test -p diskguardian-app --test ipc duplicates` — the pair is found and the
same-size file with different middle bytes is not; the group's caveat equals
`CONTENT_IDENTICAL_CAVEAT`; a floor above the pair returns an empty report with nothing hashed;
a zero budget returns `partial: true` with no groups; an unknown scan is a typed error; and a
name of a shared inode is excluded and counted while the genuine pair beside it is still found.
`bun test` on `duplicates.ts` proves the panel model cannot render a group without a caveat
(a blank engine caveat falls back to its own words), that every figure that could be read as
free space is written "up to", and that exactly one member is the keeper.
**Falsifier:** a group shown without its caveat, a hard-linked name offered for collection, or a
spent budget presented as "no duplicates".
**Honest limit:** the panel offers collection, not deletion; the clone caveat is the reason,
and clone detection remains R6.

### C27 — Local snapshots are visible, and the purge tells the truth about privilege  `[ ]`
The overview reports how many APFS local Time Machine snapshots the boot volume holds and when
each was taken. Nothing invents a size for them: macOS reports existence, not occupancy, so the
list carries timestamps and the only measured figure — how far free space actually moved — is
read before and after a purge attempt. Purging runs `tmutil` **as the user and never elevated**;
when macOS refuses for want of administrator rights, the refusal is its own reported outcome,
naming the cause and the route that works, rather than a silent "0 freed".
**Probe:** `cargo test -p diskguardian-app` — 5 unit tests on `parse_listing`/`is_stamp` (the
header and the empty answer are not snapshots, seven malformed stamps are ignored, a snapshot
name inside a sentence line is not a snapshot, stamps are the only accepted date shape) and 2
IPC tests driving the real command surface through an injected `tmutil` runner: the root-rights
refusal returns `refused: true`, `purged: false`, a message naming administrator rights and
System Settings, and the snapshot still listed; a success returns `purged: true` with an empty
list; and an unconfirmed purge is refused before anything runs. `bun test` on `snapshots.ts`
proves no size is ever rendered beside a snapshot, that a failed listing is never rendered as
an empty one, and that purge is offered only when there is something to purge.
**Falsifier:** a per-snapshot size shown, a failed listing shown as "none", a refusal shown as
success, or a privileged delete call site added anywhere.
**Why this claim is still open despite green tests:** the panel has never been looked at. The
bundle was rebuilt and launched to photograph it, and the screen locked before the capture, so
the one check this project requires for appearance — a screenshot whose pixels someone looked
at — did not happen. Everything else is verified: the wiring is closed by `tsc`, by every
element id `main.ts` requests existing in `index.html`, and by the model tests. The claim closes
on the first visual pass, which is one launch away.
**Not observed, stated rather than glossed:** the purge has never been executed against real
snapshots. This machine holds 24 of them and deleting them was not authorised, so the command
path is proven through the injected runner and the real refusal branch is inferred from
tmutil's own `requires root privileges` string. The first real purge is the observation that
closes that gap.

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
- **Walker benchmarked against `du`, not a target** (2026-10-30, `du -sh ~/Library` = 79.6 s vs our 87.9 s on 974,623 files). The walk is within 10% of the filesystem floor for that tree, so the original "10 seconds" C14 target was invented rather than measured. **Correction (2026-11-02):** the earlier note that our kernel time is "double `du`'s, from a redundant per-directory stat" does not survive scrutiny. `sys` is summed across all threads, and this walker is parallel while `du` is single-threaded, so the comparison says nothing about extra syscalls. There is one `stat` per directory and there is no free way to remove it (the parent's directory entry does not carry the child directory's block count). R2 is therefore closed as not-a-defect rather than fixed.
- **Delete prunes, it does not rescan** (2026-10-30). The first working version re-scanned the whole volume after every delete, which is minutes on a real disk and made the feature feel broken. `Tree::forget` now removes the deleted nodes, recomputes ancestors and re-projects the view, so the flower updates in the same frame.
- **Trash by default, Option for permanent** (2026-10-30). The button moves to the Trash; holding Option deletes permanently and always confirms. Trash runs over 5 GB also confirm.
- **Admin scanning is prompt-per-session, not a signed helper** (2026-11-02; cadence set by the
  principal: "one per privileged session"). Built on branch `r3-session-scanning` (rebased to
  `fd853b1`), green, not yet merged. The original design's §10 said a resident privileged
  daemon should not be built before a signing identity exists; the principal's cadence decision
  overrides that judgement deliberately, and the liveness tests below are the price of it. The first privileged pass raises the
  OS prompt; later passes in the same app run reuse that helper rather than asking again. The
  helper must therefore die with the app and after an idle timeout, since a long-lived root
  process is a wider window than a one-shot: both are pinned by tests.
  The repo has no signing identity and `SMAppService` will not register an unsigned daemon on
  macOS 13+, so the signed path is unavailable rather than merely harder. The helper is a
  file-driven binary behind a launcher-agnostic job/results protocol, so a future signed runner
  replaces one module and no protocol.
- **The resident helper's liveness is three mechanisms, not one** (2026-11-02). A parent-pid
  watch is impossible because the launcher deliberately backgrounds the helper so the dialog can
  close, orphaning it to launchd within milliseconds. Instead: an app-owned heartbeat the helper
  reads every five seconds (exits at thirty stale, which covers a crash), disappearance of the
  session's staging directory (which the app's exit removes), and a fifteen-minute idle timeout.
  Each has a test, plus a negative control proving it stays alive while the heartbeat is fresh.
- **The helper cannot attribute a job cryptographically** (2026-11-02, stated rather than
  papered over). Same-uid writers in a 0700 staging directory are indistinguishable, so its
  defences are schema, size and path validation. The worst case in the threat model therefore
  remains *root measuring into a 0700 directory*, never mutation: the helper has no delete code.
- **Measurement only runs elevated** (2026-11-02). The privileged helper has no delete call
  sites; deletion stays in the unprivileged app behind the unchanged guard. Visibility of a
  protected path must never become deletability of it, and a test pins that.
- **Disk-derived strings never reach a shell** (2026-11-02). The `osascript` call is a fixed
  template whose only interpolations are two app-generated, charset-asserted paths, so a
  folder named to look like shell syntax is inert.
- **Narrow deletion guard** (2026-10-30, from a failing test that was right). Protecting `/private` and `/usr` wholesale refused the user's own temp directory (through the `/tmp` symlink) and every Homebrew cache. The guard now names the system's own trees, and a second test pins the narrow behaviour so a future "fix" cannot re-widen it.

## Roadmap — DaisyDisk parity (opened 2026-11-02)

Ordered by space actually found on a real Mac, not by polish. Each becomes a claim when its
increment opens.

- **R1 — C18**: DONE (2026-11-02). One `+` click on the built app took the collector from
  0 bytes to 4 MB, flipped the row to a tick, enabled Delete and outlined the petal. The
  "collecting is broken" report belonged to the C17 delete defect, not to this path.
- **R6 — struck-through items**: DONE (C22).
- **R12 — Manual path entry**: DONE (2026-11-02). Type a path in the overview and scan it; `~` expands, Cmd-L focuses the field. Drag-and-drop and the folder picker remain, but neither needs a dialog now.
- **R13 — Duplicate finder**: DONE (C24 engine, C26 panel). Size, then head/tail, then full
  BLAKE3; content-identical with the clone caveat shown as visible text.
- **R3 — Admin scanning**: read restricted petals by scanning as root. Largest real win.
- **R4 — APFS local snapshots**: DONE (C27) as far as this project's rules allow. Listed with
  timestamps, purge offered as the user, refusal reported in words. A privileged purge stays
  unbuilt by decision (2026-11-02): measurement is the only thing that runs elevated, and the
  privileged side has no delete call sites.
- **R5 — Hidden space**: purgeable, other APFS volumes, still-hidden items as a virtual node.
- **R6 — APFS clone detection**: accuracy, so clones stop counting twice.
- **R7 — Reveal in Finder + Quick Look wiring** on the selected petal.
- **R8 — Favourite folders** (principal, 2026-11-02). Any folder can be flagged as a
  favourite from where it is being viewed — the scan screen, the picker, or the overview —
  and the favourites then appear on the "Disks and folders" front page above the volumes,
  each with a Scan button. Any favourite can be unstarred from the same place. Favourites
  persist across launches; a favourite whose folder has since gone is shown as unavailable
  rather than silently dropped (A3); starring something already starred is a no-op, never a
  duplicate row.
- **R9 — Packages** shown solid but expandable.
- **R10 — Real-time gauges**, Option folds purgeable into the bar.
- **R11 — Lower**: cloud accounts, parallel multi-disk scanning, disk images, network
  volumes, Forget scan results.

## Not yet specified

In scope eventually, too dim to state as claims today:

- Streaming traversal with a flat arena, so a scan of a multi-million-file volume does not hold every node in memory at once.
- APFS clone detection (`ATTR_CMNEXT_CLONEPARENT`), the largest known accuracy gap.
- Incremental rescan: reusing a previous tree and re-walking only paths whose mtime changed.
- Admin/root scanning and the privileged helper that would carry it.
- Animation on drill-down, which DaisyDisk has and v0.1 does not.

## Log

- 2026-11-02 — Local snapshots shipped as far as the rules allow (C27, open on appearance).
  A "Local snapshots" row on the overview, a panel listing every APFS local Time Machine
  snapshot with its timestamp and no invented size, and a purge that runs `tmutil` as the user
  and reports a macOS refusal as its own outcome with the cause and the route that works.
  Principal decision recorded: a privileged purge stays unbuilt — measurement is the only thing
  that runs elevated, and the privileged side has no delete call sites. 5 unit tests on the
  listing parser, 2 IPC tests through an injected runner, 10 frontend model tests, and a
  bridge test that pins the browser demo to never claim a purge succeeded.
  Two things are honestly not done: the purge has never run against real snapshots (24 exist on
  this machine and deleting them was not authorised), and the panel has never been photographed
  — the bundle was rebuilt and launched, and the screen locked before the capture. The claim
  stays open on that second point alone.

- 2026-11-02 — C18 closed on one click, and the honest reading is that nothing was broken.
  A `+` in the contents list of the built app moved the collector from 0 bytes to 4 MB, turned
  the row's `+` into `✓`, enabled Delete and outlined the petal, all in one interaction
  (`/tmp/dg-c18.png`, pixels inspected). The 2026-10-30 report of "collecting does nothing" was
  made while the C17 delete defect was live — the same session that fixed delete also fixed
  what looked like this. The suspected IPC argument-binding fault never existed; it was
  refuted by execution and is recorded as refuted, not quietly dropped.

- 2026-11-02 — Duplicates panel shipped (C26): the engine's duplicate finder now has a face.
  Groups render with the APFS-clone caveat as visible text (the model falls back to its own
  words if a payload ever arrives without one), the newest copy is marked `keep` and every
  other copy carries a `+` into the ordinary collector and the unchanged guard. The command is
  async because a whole-file hash is seconds of I/O; a spent budget reports `partial` rather
  than "no duplicates". Five IPC tests and nine frontend model tests. One fixture was wrong
  first: a lone hard-link pair never reaches the nlink check, because size grouping settles it
  for free, so `excluded_links` reads 0 by design — the fixture was rebuilt to share a size
  bucket with a real pair rather than the assertion being relaxed.

- 2026-11-02 — Duplicate finder shipped (C24): `crates/core/src/duplicates.rs`, a
  cheap-first pass (size group, 8 KiB head/tail BLAKE3, whole-file BLAKE3) that runs on
  demand over a finished scan. Hard links excluded entirely, missing nodes excluded,
  deadline and candidate cap both report partial rather than pretending completeness,
  and every group carries the APFS-clone caveat so the UI cannot show a space promise
  the disk may not keep. 11 acceptance tests, each honesty property verified RED by
  neutralising its mechanism first; one real control-flow defect caught that way (a cap
  hit mid-group discarded the already-read pair before whole-file verification).

- 2026-11-02 — Overview shipped, and the app now starts there instead of guessing at `$HOME`. Cross-platform pass alongside it: volumes come from `sysinfo` rather than `df`, reveal/Quick Look have per-OS branches (Quick Look itself is macOS-only, so elsewhere the item opens in its default handler), and the deletion guard builds its protected list per platform. Recorded honestly: two earlier scripted patches (the error banner and the post-scan collector probe) silently failed to match and never applied, so a commit claimed a diagnostic that did not exist. Every subsequent edit asserts its anchor.

- 2026-10-30 — Progressive paint shipped (`d59013e`). Evidence: `bun test` 10/10 including four new tests on `partial.ts`; `cargo test` 10/10; clippy silent; bundle rebuilt and launched. Not photographed: the growth animation itself, because browser automation is unavailable this run. C15 closed on the layout tests; C14 still open and now measured against `du`.

- 2026-10-30 — Verification: `cargo test -p diskguardian-core` 10/10, `cargo clippy --all-targets -D warnings` silent, `bun test` 6/6, frontend build clean, and the built app renders a real 4.2 GB / 51,518-file tree (`/tmp/dg3.png`, viewed). C8 stays open: browser automation is unavailable because the Interceptor test profile is unset, and the install is browser-only, so no click was driven end to end.

- 2026-10-30 — Run opened. DaisyDisk feature surface captured from its own guide (sunburst structure, colour semantics, hover/click navigation, collector, physical-size accounting, excluded hard links/clones, restricted-space handling, virtual "smaller objects" and "hidden space" rows).
- 2026-10-30 — Engine complete and tested (10 tests). Two real defects found by tests and fixed: the deletion guard was too broad, and `file_count` double-counted every file. A third was found by running the app: the empty-state overlay ignored the `hidden` attribute because a CSS `display` rule outranked it.
