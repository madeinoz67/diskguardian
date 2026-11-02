# R5 — Hidden space: design

Date: 2026-11-02 · Author: increment-designer · Status: proposed (not implemented)

**High-risk areas, named in the first paragraph as required:** this increment touches
**physical-size accounting at the volume level** — it defines what a volume scan's total
*means* (tree + a residual against an OS-measured anchor) and bounds what a volume scan
walks — and it **extends restricted-space reporting upward** from directories to the whole
volume (the residual names restricted roots, snapshots and other volumes). It does **not**
modify the deletion guard or any delete call site, and the `Tree`'s per-node arithmetic
(C5) is untouched: every new number lives in the view or in panel payloads, never in the
tree. No privileged call is added (A5): every probe below is unprivileged.

**Pre-commitment note.** The OS-source measurements quoted in §0 were taken to establish
what *can* be measured — they are inputs to the mechanism. The scan-side numbers the
acceptance rule (§7) judges — a volume scan's residual, and the band it must land in —
have **not** been observed; no volume scan was run before this document was committed.

---

## 0. What this machine can and cannot measure (evidence, not recall)

All figures measured 2026-11-02 on the build machine's boot container (2.0 TB,
`1,995,218,165,760` bytes, APFS, FileVault on). Volume names and any identifying strings
are deliberately absent; the numbers are the point.

| Source | Call | What it returned |
|---|---|---|
| statfs (via `df -k`, and what `sysinfo::Disks` reads) | `statfs("/")`, `statfs("/System/Volumes/Data")` | **Container-wide numbers.** `f_blocks` = container total; `f_bavail` = 285,683,814,400 B — identical for every volume in the container. So `total − available` = **1,709,534,351,360 B of container used**, from one cheap syscall. |
| `diskutil apfs list` | fixed argv, plist-free text (a `-plist` form exists) | Per-volume used: Data 1,681,893,560,320 · System 12,635,066,368 · Preboot 9,041,682,432 · Recovery 1,307,287,552 · VM 4,295,032,832. Sum = 1,709,172,629,504 — **0.02 % below** the statfs container used; the ~362 MB remainder is container overhead plus one volume (`Update`, disk3s2) that the listing does not name. Container used per diskutil: 1,709,534,621,696 — **0.00002 % from the statfs figure**. Two independent OS sources agree. |
| `NSURL` resource keys (Foundation) | `.volumeAvailableCapacityForImportantUsage` / `.volumeAvailableCapacity` | Important-usage available = **450,059,339,709 B** vs statfs available 285,683,814,400 B. The difference — **164,375,525,309 B ≈ 9.6 % of the disk** — is the purgeable pool as macOS accounts it (snapshots, caches, offloadable content the system may free under pressure). Both endpoints are OS-measured; this is not our arithmetic on guesses. |
| `stat -f %d` | st_dev of candidate roots | **The merged boot namespace is one device id**: `/`, `/Users`, `/System`, `/System/Volumes/Data`, `/System/Volumes/Data/Users` all report `16777232`. Preboot reports `16777231`, VM `16777233`, Update `16777229` — every non-boot-group APFS volume has its own id. Mounted **local-snapshot mounts** (`/Volumes/com.apple.TimeMachine.localsnapshots/…`, present on this machine during a backup) report `16777232` — same id as the boot group. An external backup volume (separate 16 TB container) has its own id. |
| `mount` | mount table | The System volume is a sealed read-only snapshot mounted at `/`; Preboot/VM/Update/xarts/Hardware are nested mounts; external and snapshot volumes mount under `/Volumes`. |

**What cannot be measured, by anybody, on this OS:** per-snapshot occupancy (macOS reports
existence, not size — C27's doctrine, unchanged); the exact *composition* of purgeable
space (it is a system promise, not a file set); and the exact split of the residual
between filesystem overhead and truly-invisible files. The design reports these as named
categories with counts or an explicit "not reported", never an invented number.

## 1. Root cause: what the app shows today, and the lie of omission it already tells

- **The overview gauge** shows `total_bytes`/`free_bytes` from `sysinfo::Disks`
  (`apps/desktop/src-tauri/src/commands.rs:96-127`, `VolumeInfo`) — container-wide
  numbers on APFS. It does not include purgeable as "free" (statfs `f_bavail` excludes
  it), so it understates available space by the purgeable pool — 164 GB on this machine —
  but it is not *wrong* about what it claims.
- **The scan screen** shows `ScanResponse.bytes` = `tree.total_bytes()` only
  (`commands.rs:223-246`). For a folder scan that is honest. For a **volume scan** it is
  the omission the lead's brief asks about: the tree total is presented as *the volume's*
  content while the volume's used space is 100+ GB larger (on this machine: other volumes
  ≈ 14.6 GB, snapshots held, TCC-protected directories, `.Spotlight-V100`-class roots,
  container overhead — none of it named anywhere). **Yes: today the gap is a lie of
  omission, and R5 exists to stop it.**
- **A `/` volume scan also currently walks off the disk.** `Builder::walk_dir`
  (`crates/core/src/scan.rs:~330-350`) has no mount boundary: the C13 log already records
  a macOS *network-volume consent dialog* raised by a `/` scan. Measured now on this
  machine, a `/` scan today enters (a) the nested Preboot/VM/Update mounts, (b) an
  **external 16 TB backup volume** mounted under `/Volumes`, and (c) **read-only local
  snapshot mounts** of the Data volume under `/Volumes/com.apple.TimeMachine.localsnapshots`
  (same st_dev as the boot group, so a naive device filter will not stop them; the inode
  ledger dedups most leaf double-counting but directory metadata double-counts, and the
  external volume's content is pure noise in a "boot disk" total). Any hidden-space
  arithmetic built on today's walk would be meaningless: the residual would be wrong by
  the size of other disks. **Fixing the boundary is a precondition of the anchor, not
  polish.**

## 2. What "hidden" means here (definitions, fixed before the mechanism)

For a **volume scan** (scan root is a mount point; §3.2):

- **U** (anchor) = volume used now, `statfs`-derived: `f_blocks − f_bavail` (bytes). On
  APFS this is *container* used; on a plain volume (HFS+/exFAT/ext4) it is that volume's
  used. Measured fresh at every view projection (one syscall — §3.2).
- **T** = the current tree total (grows/shrinks with grafts, deletes, revalidates).
- **H** (hidden) = `U − T`, reported, never clamped silently. Negative H is a *state*
  ("the disk changed during/after the scan"), shown in words, petal omitted (§3.3).
- **Decomposition** — components of H, each measured or explicitly not:
  - **Other volumes in this container** (`diskutil apfs list -plist`, macOS): Σ used of
    container volumes not covered by the walk. For a `/` scan the walk covers the boot
    group (System + Data — they share one st_dev, §0), so this is Preboot + Recovery +
    VM ≈ 14.6 GB here. For a Data-rooted scan it additionally includes the System volume.
  - **Snapshots**: count + timestamps from the existing C27 command. **No size is
    invented**; the panel says they are included in the still-hidden figure.
  - **Protected areas**: the tree's topmost `Restricted` roots (`Tree::restricted_roots`,
    already public) — named, count listed, sizes unknown by construction.
  - **Still hidden**: `H − other_volumes`. The honest remainder: container overhead,
    snapshot-held blocks, `.Spotlight-V100`-class roots, TCC shadows, whatever else.
- **Purgeable is an annotation, not a slice of H.** `important − avail` counts space the
  system *may* free, and much of that pool is **visible files already inside T** (caches
  a walk sees). Presenting purgeable as a component of H would double-count it against
  the tree; on this machine purgeable (164 GB) exceeds the plausible H precisely because
  it overlaps T. So the panel carries it as its own line — "macOS can free **up to** N
  (purgeable: snapshots, caches and offloaded content, measured as the difference between
  two capacity figures the OS reports)" — with the "up to" as visible text, in the C26
  caveat style. Folding it into the overview gauge stays R10's.

## 3. The mechanism

### 3.1 Core (three small additions; `Tree` per-node arithmetic untouched)

1. **`ScanOptions::same_device_only: bool`** (default `false`) and
   **`ScanOptions::exclude_prefixes: Vec<PathBuf>`** (default empty). When
   `same_device_only`, the root's `st_dev` is captured at scan start and a directory
   entry whose dev differs is **not descended and not added to the tree** — it is
   another volume's content, not unmeasurable space of this one (A3 is about the scanned
   target; the panel reports the exclusion instead, §3.3). `exclude_prefixes` skips
   subtrees by prefix — same treatment. Both classes are recorded in a new
   `Tree::skipped_mounts: Vec<String>` (`#[serde(default)]`; `Tree` crosses the process
   boundary only to/from the admin helper's job files, where a missing field must
   default, not fail). Cost: zero extra syscalls — the directory's `metadata()` is
   already taken in `walk_dir` for `own_bytes`; it moves earlier in the function.
2. **`view::build_volume_view(tree, root, opts, anchor: VolumeAnchor) -> Vec<ViewNode>`**
   where `VolumeAnchor { used_bytes: u64 }` is a plain, portable struct. Behaviour:
   exactly `build_view`, plus one synthetic root child when `hidden > 0`:
   `{ node_id: None, name: "Hidden space", kind: "hidden", bytes: hidden, children: [] }`
   — the same synthetic-node construction as the "smaller objects" petal
   (`crates/core/src/view.rs:126-137`), and the root `ViewNode.bytes` is set to
   `anchor.used_bytes` so the header and the ring sum to the volume. `build_view` itself
   is untouched: every existing caller is unaffected.
3. Why the node lives in the view, not the tree: the tree's invariant is `total = own +
   Σ children` over *measured nodes* (C5); H is a residual that must move when the tree
   moves (delete, graft, revalidate) — computing it at view time from the **current**
   tree makes it self-correcting by construction, and keeps `Tree` free of a virtual
   node that deletion, duplicates and classification would all have to special-case.
   Precedent: the view-pruning merge already owns synthetic presentation nodes.

**Degradation both ways across the IPC boundary** (the high-risk class): the serialized
`ViewNode` shape is unchanged — `kind` is a `&'static str` that gains one value. An old
frontend receiving a new tree renders the hidden petal through `colorFor`'s `default`
arm (`apps/desktop/src/layout.ts:126-137`) as an ordinary petal labelled "Hidden space";
every interactive path guards on `node_id != null` (`main.ts:567` collect, `main.ts:832`
navigate), and the node has none, so it cannot be collected, navigated, revealed or
Quick-Looked. A new frontend against an old backend simply never sees the kind.

### 3.2 App side: one probe module, one command, additive payloads

New `apps/desktop/src-tauri/src/volume.rs`, the shape of `snapshots.rs` (commands + pure
parsers + an injected seam for tests):

- **`volume_used(mount) -> Option<u64>`**: `sysinfo::Disks`
  (`new_with_refreshed_list`, exactly what `snapshots.rs::free_bytes` and
  `commands::volumes` already do) filtered to the mount; `total − available`. Unprivileged,
  portable, ~ms.
- **`apfs_container_report(mount)`** (macOS; other platforms return `None`): runs
  `diskutil apfs list -plist` with a **fixed argv, no interpolation** (the C27 tmutil
  pattern — no disk-derived string ever reaches a command line; plist volume names are
  data, parsed, and displayed only in this user's own panel). Parses the plist
  (`plist` crate or serde-plist — app-only dependency) into: container identity of the
  scan volume, per-volume `CapacityConsumed`, boot-group membership (System/Data roles).
  A parse failure or non-APFS root yields `None` — decomposition absent, H still shown,
  panel says "not decomposed on this volume".
- **`purgeable_bytes(mount) -> Option<u64>`** (macOS): `important` minus `avail` from
  `NSURL` resource values — requires Foundation bindings (`objc2-foundation`, app-only,
  macOS-gated; the app already links far heavier machinery). `None` when negative or
  unavailable; never a made-up figure.
- **Test seams**: all three are injectable function slots in the `snapshots.rs::Runner`
  style; the IPC tests feed canned plist/number fixtures.

**Volume-scan detection:** the `scan` command compares the requested path against the
mount points of the `sysinfo` list it already builds for the overview — if the root is a
mount point, this is a volume scan: `same_device_only = true`, plus on macOS the two
**fixed system prefixes** `/System/Volumes/Data` and `/Volumes/com.apple.TimeMachine.localsnapshots`
(exclusions supplied by the app, not the engine; both are constants in `volume.rs`, and
neither is disk-derived). The first prevents double-walking the Data volume that the
merged namespace already presents via firmlinks; the second prevents walking transient
snapshot mounts (same st_dev — §0 — so the device bound alone cannot stop them; their
content is Data's own past, and their bytes belong in the unsized snapshot category).
Folder scans change nothing: no anchor, no boundary, no hidden node.

**Anchor policy:** `U` is re-measured on **every** projection (scan response, `view`,
`revalidate`, `delete_collected`, the admin graft report) — one statfs each, so H always
reflects the current tree against the current volume. Decomposition components
(`diskutil`, purgeable) are measured once at scan completion and cached on the `Scan`,
with a "measured at scan time" line in the panel (they move slowly; `diskutil` per
navigation would be a subprocess per click). Negative H beyond 0.5 % of U hides the petal
and surfaces "the volume changed more than the scan window — rescan for exact figures"
as visible text; a small negative within tolerance is shown as 0 with the same note.

**New command** `hidden_space(scan_id)` (nothing else — the webview cannot name a path):
returns `{ used_bytes, tree_bytes, hidden_bytes, other_volumes: Option<Vec<{name, bytes}>>,
snapshot_count, snapshot_note, restricted_roots: Vec<{path, note}>, purgeable_bytes:
Option<u64>, purgeable_note, still_hidden_bytes, measured_at }`. `ScanResponse` and the
other view-returning responses gain `volume: Option<VolumeSummary>` (additive;
`hidden_bytes` + `used_bytes` so the header can render without the panel call).

### 3.3 Frontend

- `types.ts`: the kind union gains `'hidden'`; new payload types.
- `layout.ts`: a `case 'hidden'` in `colorFor` — a distinct low-saturation purple-grey,
  visually kin to `restricted` (the same "the engine cannot show you this" family).
- `main.ts`: when a scan response carries `volume`, the header total is `used_bytes`; the
  hidden petal's click opens the **hidden-space panel** instead of navigating (its
  `node_id` is null, so today it would do nothing — one branch keyed on `kind === 'hidden'`).
- New `hidden.ts` pure model + `bun test`s, the `snapshots.ts`/`duplicates.ts` precedent:
  no component renders without its "up to"/"not reported" wording; the model cannot
  render a size for snapshots; a failed decomposition never renders as "0 other
  volumes"; the negative-H note is passed through, never swallowed.
- **FDA hint, closing C23's recorded deferral** (the R3 design §1 deferred the Full Disk
  Access hint to R5): when the tree still has restricted roots after any admin pass, the
  panel's protected-areas section carries one line — "Some of this is protected by macOS
  privacy settings; grant Full Disk Access to measure it" — with a button opening the
  pane via the existing `open` adapter (`platform.rs` shape) with a **fixed URL**
  (`x-apple.systempreferences:com.apple.preference.security?Privacy_AllFiles`). No
  detection of whether FDA is granted (no clean API; the hint is unconditional while
  restricted roots remain).

### 3.4 Cross-platform

Linux and Windows get the anchor and the hidden petal (U from statvfs/
`GetDiskFreeSpaceExW`-backed `sysinfo`; T from the same walker; the st_dev bound works
on both — Windows drive roots are mount points with nothing nested). No decomposition:
the panel shows H with "this volume's breakdown (other volumes, purgeable) is
macOS-only for now". The engine additions are platform-neutral; every macOS-specific
probe lives in the app crate behind `cfg`.

## 4. Minimal first slice, and the DEFERS list

**Slice:**

1. Core: `same_device_only`, `exclude_prefixes`, `Tree::skipped_mounts`,
   `VolumeAnchor` + `build_volume_view` + tests (§7 AC1).
2. App: `volume.rs` (three probes + parsers + seams), volume-scan detection in `scan`,
   macOS exclusion constants, `hidden_space` command, `VolumeSummary` on responses,
   registration in `lib.rs`.
3. Frontend: types/colour/header/panel click, `hidden.ts` model + tests, the FDA line.
4. Docs: README "Hidden space" section — what each component means, why snapshots carry
   no size, why purgeable says "up to".
5. Verification: full five-gate battery; the real-machine run of §7; screenshots of the
   petal and the panel (pixels inspected), plus one driven click (panel opens on petal
   click, navigation unchanged elsewhere).

**DEFERS (explicitly not in this increment):**

- **R10's purgeable-in-the-gauge** — the overview bar keeps showing statfs free; folding
  purgeable in is R10's own row.
- **Sizing snapshots** — stays a count (C27 doctrine); reopen only if Apple exposes an
  occupancy API.
- **Clone interplay (R6)** — APFS clones counted as separate allocations inflate T and
  so deflate H by an unknown amount; quantified when R6 lands, noted here so nobody
  "fixes" H for it later.
- **Linux/Windows decomposition** (btrfs subvolumes, LVM, VSS) — anchor-only now.
- **One row per physical disk on the overview** (merging the APFS boot-group rows
  sysinfo lists) — a C19 UX change with its own acceptance; this design only makes the
  *scan* honest.
- **Live re-measurement of decomposition components** (diskutil/purgeable refresh button).
- **Any affordance that frees hidden space beyond what exists** — C27's snapshot purge
  and C21's trash-empty are already built and stay the only ones.

## 5. Claim and anti-claim impact

- **A1 (deletes only what was collected): unaffected.** No delete call site moves; the
  hidden node cannot enter the collector (no `node_id`; frontend guards at `main.ts:567`).
- **A2 (allocation, never logical): extended, not violated.** H is an exact residual
  between two measured quantities (statfs used; the tree's `st_blocks`-accounted total);
  U's inputs are OS-measured; the purgeable figure is a difference of two OS-measured
  capacities, displayed with "up to". No logical length enters any of it.
- **A3 (unmeasured reported, never dropped): strengthened twice** — the volume-level gap
  becomes a reported node with named components (from invisible today, §1), and skipped
  mounts are listed (`skipped_mounts`) rather than silently absent from a volume total.
- **A4 (UI never blocks): preserved** — probes are ms-class; `diskutil` runs on
  `spawn_blocking` once per scan; every command stays async where it already is.
- **A5 (core loop never elevated): preserved** — statfs, `diskutil`, `NSURL`, `tmutil`
  are all unprivileged; nothing new runs elevated.
- **A6 (arcs proportional): holds** — the hidden petal's width is H bytes over the ring
  total U, the same proportionality as every other petal; H ≤ 0 means no petal plus text.
- **C5 (tree arithmetic): untouched** — H never enters the `Tree`.
- **C14: unaffected** — its measurements are folder scans; volume scans gain a boundary
  (a *reduction* of work, if anything).
- **C27: referenced, not changed** — snapshot count reused; the no-size doctrine extended
  into the panel.
- **C23: its deferred FDA hint lands here** (§3.3).

**New claim, exact text proposed for ISA.md § Claims:**

> ### C29 — Hidden space is accounted, not implied
> A scan whose root is a volume reports the volume's own used space, and the difference
> between it and the measured tree is one "Hidden space" node whose width is that
> residual — decomposed into other volumes of the same APFS container (measured), local
> snapshots (counted, never sized), protected areas (the tree's restricted roots, named),
> and a "still hidden" remainder that is named rather than distributed. macOS' purgeable
> figure is shown beside the breakdown as "up to N", never as a slice of the residual,
> because the system may free space the tree already counts. A volume scan walks only the
> scanned volume's device (the boot volume group counts as one), never other disks or
> snapshot mounts, and the mounts it excluded are listed. A negative residual is
> reported in words with the petal omitted, never shown as zero silently.
> **Probe:** `cargo test -p diskguardian-core` — `build_volume_view` places the residual
> exactly (`anchor = tree ± K` ⇒ hidden = K; `anchor: None` ⇒ no hidden node); boundary
> tests — an excluded prefix and a foreign `st_dev` subtree leave the tree, are listed in
> `skipped_mounts`, and set nothing to zero. `cargo test -p diskguardian-app` — canned
> `diskutil` plist parses (well-formed, other-container, malformed → no decomposition,
> never zeros); injected capacity probes (purgeable negative or absent → not reported);
> `hidden_space` IPC on the mock runtime (unknown scan typed error; folder scan returns
> none; panel payload carries the "up to" and no-snapshot-size wording). `bun test` on
> `hidden.ts` — the model cannot render a snapshot size, a missing decomposition as
> "none", or a swallowed negative-residual note.
> **Falsifier:** a hidden petal whose bytes are not `volume used − tree total` at the
> moment of projection; a purgeable figure presented as freed or as part of the residual;
> a volume scan that enters another disk or a snapshot mount; a skipped mount absent
> from the report; a per-snapshot size shown anywhere.

**New § Decisions entries proposed:**

1. *A volume scan is bounded by device* (2026-11-02) — root `st_dev` defines the scan;
   the APFS boot group shares one id (measured, §0), which is exactly the disk a user
   means by the startup volume. Two fixed macOS prefixes (`/System/Volumes/Data`, the
   localsnapshots mount root) are excluded to prevent double-walking the merged
   namespace; the inode ledger alone was measured insufficient (directory metadata
   double-counts, and snapshot mounts share the device id).
2. *Hidden space is computed at view time, never stored* — the residual must track
   grafts, deletes and revalidates for free; the tree stays purely measured nodes.
3. *Purgeable is an annotation, not a slice* — it overlaps visible files; slicing it
   into the residual would double-count (measured: purgeable can exceed the whole
   residual on a real disk).
4. *Every OS figure carries its qualifier as visible text* — "up to" for purgeable,
   "macOS does not report per-snapshot size", "measured at scan time" for the cached
   decomposition.

## 6. Surfaces that must move

| Surface | Change |
|---|---|
| Core public API (`crates/core/src/lib.rs`) | re-export `VolumeAnchor`, `build_volume_view`; `ScanOptions::{same_device_only, exclude_prefixes}`; `Tree::skipped_mounts` (`#[serde(default)]` — admin job files) |
| Serialized shape across IPC | `ViewNode` unchanged (kind gains the value `"hidden"`); `ScanResponse`/view-returning responses gain `volume: Option<VolumeSummary>` (additive); `Tree` gains `skipped_mounts` (additive, defaulted — crosses only the admin helper boundary, not the webview) |
| Command surface | new `volume.rs`, new `hidden_space(scan_id)`, `lib.rs` registration; `scan` sets boundary options for volume scans |
| Dependencies (app crate only) | `objc2-foundation` (macOS, `NSURL` capacity keys), a plist parser |
| TypeScript | `types.ts` (kind union + payloads), `layout.ts` (one colour case), `main.ts` (header total, petal click → panel), new `hidden.ts` + tests, `bridge.ts` demo mode |
| Docs | README "Hidden space" section |
| ISA | C29 + four § Decisions entries; R4/R5 cross-references (maintainer lands these) |

## 7. Pre-committed acceptance rule

**Correctness half (tests, all synthetic or injected — nothing here can be fluff):**

1. Core identity: `build_volume_view` with `anchor.used = tree.total + K` yields a
   hidden node of exactly K; root node bytes equal the anchor; with `anchor.used <
   tree.total` there is no hidden node and the negative residual is surfaced in the
   return; with no anchor there is no hidden node (**RED control** for the mechanism).
2. Boundary: a fixture tree with an `exclude_prefixes` match and (Unix-gated) a foreign
   `st_dev` subtree — both absent from the tree, present in `skipped_mounts`, tree
   arithmetic intact, nothing zeroed (A3 shape).
3. App parsers on canned fixtures: a well-formed container plist; a volume in a
   *different* container (must not be summed); a malformed plist (decomposition absent,
   never zeros); injected capacity probes where `important < avail` (purgeable not
   reported, never negative).
4. IPC on the mock runtime: `hidden_space` for an unknown scan is a typed error; a
   folder scan reports no anchor; the payload's wording fields are non-empty where the
   claim requires them ("up to", the snapshot note).
5. `bun test`: the model refuses every dishonest render named in C29's falsifier.

**Real-machine half (the plausibility arm, cross-checked against three independent OS
sources so a plausible-looking figure cannot pass alone):** on the boot container,
immediately after a `/` volume scan: the app's U must agree with `df -k`-derived used
and with `diskutil apfs list`'s container-used **within 1 %** (two oracles external to
our code); the purgeable line must agree with a fresh `swift`/Foundation one-liner
within rounding; and the residual H = U − T must land in the band
**[0.9 × other_volumes, 0.25 × U]** — the floor is an arithmetic necessity (the other
volumes are by construction unwalked, so a residual below them means the boundary is
leaking or the anchor undercounts), and the ceiling catches any container/volume
confusion or double-count. Sample size: one boot volume, plus the mounted external
volume as the tie-breaker if needed.

**Both kill directions, written before any scan-side number is seen:** **ship** if AC1–5
pass, the oracles agree within 1 %, and H lands inside the band; **kill** (fix before
merge, or defer R5 to behind R6 if the cause is clone inflation) if H < 0.9 ×
other_volumes — the boundary or anchor is wrong and every downstream number inherits
the lie — or if any oracle disagrees by > 1 %; **underpowered, not conclusive** if H
lands above the ceiling or below the floor with the panel unable to name why — measure
the second volume before any verdict, and treat a still-hidden share > 80 % of H as the
same signal. No threshold moves after data is seen.

## 8. Top risks, each with the probe that would falsify the design

1. **Time skew: the volume moves between walk and anchor** (this machine was mid-backup
   during measurement, with snapshot mounts appearing live). H is re-measured per
   projection, which narrows but cannot remove it. Falsified by the acceptance run
   recording H twice, 60 s apart: a swing large enough to flip the band verdict kills
   the per-projection policy in favour of anchor-at-scan-end, and a persistent negative
   beyond 0.5 % kills the volume-scan anchor outright (report only).
2. **The st_dev model of the boot group is a macOS implementation detail** (firmlink
   presentation could change; encrypted groups, Fusion, external-boot configurations
   untested). Falsified by the band check and the oracle agreement on the real boot
   volume; degraded mode if it ever breaks elsewhere: other-volumes absent, H still
   reported with "not decomposed" — honest, not broken.
3. **`diskutil apfs list` output drift across macOS versions** (and the `Update` volume
   it already does not list, which silently lands in still-hidden). Falsified by the
   canned-plist parse tests including a hostile/malformed fixture; in production a parse
   failure yields no decomposition, never zeros.
4. *(Smaller, named)* **Old frontend + new tree**: verified graceful (§3.1) — an unknown
   kind falls to `colorFor`'s default arm and every interaction guards on `node_id`.

## 9. Precedent

- **The view-pruning merge** (`view.rs:119-137`): synthetic presentation nodes with
  `node_id: None` and a `kind` string; the hidden node is its sibling, and C16's
  "a synthetic child never resolves" protects it from navigation for free.
- **`snapshots.rs`** (runner seam, fixed argv, refusal-as-a-result, no-size doctrine):
  `volume.rs` copies all four, and C27's snapshot count is reused rather than re-measured.
- **`ScanOptions` budgets** (`max_entries`, `deadline`): `same_device_only` and
  `exclude_prefixes` are the same options-object pattern with the same
  reported-not-dropped consequence.
- **C23/C25**: shape-only boundaries and reported-refusals; the FDA hint closes R3's
  recorded deferral to R5.
- **The mock-runtime IPC harness** (`src-tauri/tests/ipc.rs`): every §7-AC4 test extends
  it, injected probes in the `snapshots.rs` seam style.

## 10. What I would refuse to build

- **Any "clean purgeable space" affordance.** Purgeable is a system promise about future
  freeing, not a file set we can name or delete; the only built purges stay C27's
  (snapshots, as the user) and C21's (trash). An elevated `deleteSnapshot` path is
  refused for the same reason R4 refused it.
- **Per-snapshot sizes**, by any means including walking snapshot mounts (they are
  transient, share blocks with the live volume, and "measured once by accident" is not a
  measurement).
- **A breakdown invented where the OS offers none** — Windows/Linux get the residual
  and a sentence, not fabricated categories; non-APFS macOS volumes get no purgeable
  line rather than a zero.
- **`df` parsing** for the anchor (locale, headers, and a shell where none is needed —
  `sysinfo` already reads the same statfs).

## 11. Open questions for the maintainer

1. **The purgeable fork**: this design makes purgeable an annotation (§2) because it
   overlaps the tree. DaisyDisk's panel slices it into hidden space. If you want their
   literal shape, the claim text must change and the double-count caveat becomes the
   load-bearing sentence — I recommend against.
2. **`objc2-foundation`** is a new (app-only, macOS-gated) dependency for two integers.
   The alternative is dropping the purgeable line to "not measured on this system" until
   a cheaper route exists. Your call on whether the figure is worth the dependency.
3. **The exclusion list is two hardcoded macOS prefixes** (§3.2). A mount-table-driven
   rule (skip same-dev mounts already covered by the merged namespace) is more
   principled and more work; I took the constants because they are Apple's own fixed
   paths, but a future macOS relocation would need a one-line fix.
4. **C29's wording** is drafted verbatim in §5; the `skipped_mounts` field on `Tree`
   changes the admin job/results serialization (additive, defaulted) — flag if you want
   that versioned instead.
