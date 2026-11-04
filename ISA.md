---
title: diskguardian
principal_stated_goal: "I want to setup a rust project called diskguardian, will be an open source disk space visualizer and free up space, cross platform. ... will be a graphical ui, primary for macos. ... research disk daisy, I want it to be identical"
phase: climbing
progress: 27/30
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

### C8 — Navigation works both directions  `[x]`
Clicking a petal re-roots the view to that node; clicking the centre returns to the previous root, restoring it exactly.
**Falsifier:** driving the app and observing an unchanged or wrong view.
**Verified 2026-11-02 on the built app.** Clicking the `beta` petal re-rooted to `beta` at 8 MB with its two files and the breadcrumb following; clicking the centre restored `dg-fixture` at 12 MB with both petals, both totals and the breadcrumb as before. Frames read at `/tmp/dgv-4a.png` (inside `beta`) and `/tmp/dgv-4b.png` (restored).
**Record corrected 2026-11-03:** the commit that closed this (`f26e37c`) said "C8 and C13 closed" while only C13's box was actually ticked and this section carried no evidence. The verification happened; the record did not follow it. Same class as the C27 stale-status note: a commit message asserting a state the file does not hold.

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

### C13 — Scanning does not freeze the window  `[x]`
While a scan runs, the window still paints, the UI reports progress, and clicks land. A scan of
another path *may* be started; the newest scan owns the screen, and the older one is stopped
rather than left to finish unseen.
**Falsifier:** window beachballs or progress stops updating during the scan.
**CORRECTED 2026-11-02 after review — the mechanism sentence above was not yet true.** The first
version of this claim said the older scan "finishes unseen", which the scan token does not
deliver: `scan-progress` events carry no scan identity, so the *new* scan's listener accepts the
old scan's branch reports and applies them by name. Two trees sharing a child name (`src`,
`target`, `node_modules`) mark the new scan's petal done at the old scan's width. The token stops
the stale listener from painting; it never stopped the fresh one from listening. A review lane
found this by reading the event plumbing after the fix had been verified by screenshot — the
screenshots were of a tree with no shared child names. The fix is a per-scan nonce carried on
every progress event and on the progress emitter, gated so a cancelled walk emits nothing; it is
folded with the cancellation branch below.
**Verified 2026-11-02 on the built app**, twice (a 17-second scan of a million-file tree and a
volume scan): the progress counter climbed (`Measuring… 9 of 21 done`), petals grew while it
ran, and clicks were accepted mid-scan — including the breadcrumb that starts a second scan.
**Narrowed, and the narrowing is the finding.** This claim used to assert that "a second scan of
another path can be started concurrently", as though that were a feature. It is not: measured
against a running volume scan, a 12 MB scan started behind it had still not completed 75 seconds
later, because both walks compete for one rayon pool and there is no cancellation path through
the IPC. The scan token stops the two from corrupting each other's view, which is what the
screenshot before the fix showed — one scan's breadcrumb over another's totals and a white petal,
wedged for ever. Serving two scans fairly is a different piece of work, on the roadmap as **R14** rather than
implied by this claim. (This paragraph previously said R13, which is the duplicate finder.)

### C14 — A million-node tree scans near the filesystem's own floor  `[x]`
A scan of a ≥1M-node developer tree completes in **no more than 1.15× the wall clock of
`du -sk` on the same tree**, and does not exceed **1 GB** of peak resident memory.
**The original bar was wrong, and that is the finding.** C14 used to demand "under 10 seconds",
which no implementation can meet: `du` itself takes 44–80 s on the trees this product is for.
A target nobody measured is not a criterion, it is a wish. The bar is now the filesystem's own
floor plus a stated tolerance.
**Measured 2026-11-02**, independent lane, pre-committed protocol, correctness proven first
(exact agreement with a raw `st_blocks` oracle and `du` on fixtures covering sparse files, hard
links, symlink loops and a permission wall):
- reference tree: **1,056,336 nodes / 900,254 leaves / 344.3 GiB** → **21.46 s** median wall
  (18.07–25.99, n=5, 10 threads, load ≈8), **388.8 MB** peak RSS, 41.9k leaves/s.
- `du` on the same tree: **44.05 s**. The walker is **1.7–2.4× faster** than the floor — the
  ordering this ISA used to record was the other way round, from an older build.
- second, independent tree: 1,001,301 files / 52.00 GB → **10.7 s**.
**Falsifier:** a scan of a ≥1M-node tree slower than 1.15× `du`, or peak RSS over 1 GB.
**How to check it in one command:** `cargo run --release -p diskguardian-core --example scan
-- <tree>` for wall/files/restricted, with `/usr/bin/time -l` around it for peak RSS.
**What wall time does and does not say.** It is contention-bound, not engine-bound: the same
tree took 130.5 s while machine load rose to 51–75, while CPU per scan stayed ~38 s (97%
kernel) throughout. Quote CPU and the load window with any wall number, or the number is
meeting-noise, not the engine.
**Why the old diagnosis was wrong.** The per-level subtree merge was blamed and was never
dominant: it was retired by `857d662` (2026-10-30) at 105.9 s → 87.9 s on the then-reference
tree, and a counted cost model puts it at 0.3–1.5 s of that run. The walk is syscall-bound, and
the remaining distance to `du` is bookkeeping, not copying. "Under 10 seconds" is retired to
the incremental-rescan item under *Not yet specified*, which is the only honest path to it: a
warm rescan re-walks changed paths instead of the whole volume.
**Memory arm.** 6.60 MB floor + ~362 B/node (`size_of::<Node>()` = 136 B, so the arena is ~36%
of peak). The contingent node-record shrink is therefore **not activated** — it was predicated
on breaching 1 GB. Directory width costs ~40% more RSS at 32k entries in one directory, stated
as a shape effect rather than a scaling law.

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
> `delete_strikes_the_node_through_and_shrinks_the_totals` in
> `apps/desktop/src-tauri/tests/ipc.rs`, which drives the real command on a mock runtime.
> *(Corrected 2026-11-03: this note named `delete_removes_the_node_and_returns_a_smaller_view`,
> a test that has never existed in the file. The evidence line pointed at nothing, which is the
> class of defect the claim checker cannot catch — it verifies that evidence is named, never that
> the name resolves.)*
After a delete the backend prunes the deleted nodes from the tree, recomputes every
ancestor's totals and returns a fresh view, so the petal disappears and the totals shrink
immediately. The default action moves items to the macOS Trash; holding Option on the Delete
button removes them permanently, which always asks first.
**Probe:** `cargo test` — `Tree::forget` (deleted node leaves its parent, every ancestor
loses exactly the deleted bytes, tree arithmetic still holds, the scan root cannot be
forgotten), the Trash-recovery test, and the end-to-end
`delete_strikes_the_node_through_and_shrinks_the_totals` in
`apps/desktop/src-tauri/tests/ipc.rs`. That last one takes its expected drop from the
pre-delete scan rather than a hardcoded figure, so it holds on any filesystem — the first
two alone are exactly what closed this claim wrongly before the 2026-11-02 correction.
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
separators); the volume list comes from `sysinfo`, so it is per-platform rather than `df`;
and `home_dir` resolves each platform's own home (`USERPROFILE`, then `HOMEDRIVE`+`HOMEPATH`
on Windows; `HOME` elsewhere), covered by `commands::tests` for both platforms and by
`pathPlaceholder` in `platform.test.ts` for the hand-typed hint.
**Falsifier:** an app that can only scan one hard-coded root — including a home row that
offers the filesystem root because the variable it read does not exist on that platform.
**Note (2026-11-04, found in the Windows VM while staging Stage H):** the home row rendered
`/` on Windows, because `home_dir` read only `HOME`, which Windows does not set. The typed-path
hint was POSIX-only for the same reason (`/tmp`, `~/Documents`), and Windows does not expand
`~`. This claim was ticked on a macOS-only run whose probe never covered home resolution
elsewhere, which is the signature of a platform-gated bug a single-OS gate cannot see.

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
`delete_strikes_the_node_through_and_shrinks_the_totals` (the drop equals the subtree total
the pre-delete scan measured — 65536 on APFS, 69632 on ext4, where the directory inode
occupies a 4096 block — not a hardcoded constant, and not the directory's metadata size;
the ext4 leg of this probe is CI's to run). The view gives a missing node a floor width of
parent/200 so it stays visible while counting as zero.
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
**Cadence gap — CORRECTED 2026-11-02: it is on `main`, not on a branch.** This paragraph used to
say `r3-session-scanning` was unmerged, and that merging it "would ship a privileged behaviour
that has not yet run against a real dialog". Git says otherwise: `fd853b1` has been an ancestor
of `main` since merge `b794c4d` ("merge: one administrator prompt per app session"), and it has
been pushed. The prose was written before that merge and never corrected, so for a while the
state of record claimed a safety position the tree did not hold. It is kept visible rather than
quietly rewritten, because a document asserting restraint that the code does not have is the
failure worth remembering. What remains true: AC4 — a manual run against a real dialog on a real
machine — is still unverified **on `main`**, and the ship/kill sizing rule still has no
real-machine number. The one-prompt-per-session behaviour is shipped, and its liveness
mechanisms are pinned only by tests.
**AC4 — CLOSED 2026-11-02, against a real dialog on this machine.** The principal supplied the
credential to a genuine macOS consent prompt ("osascript wants to make changes"); automating
that is not something this project will ever do, which is why the claim needed a human. What
followed: `diskguardian-admin --serve` answered `{"completed": 452, "exit": 0,
"deadline_hit": false}` — every restricted root measured, none abandoned — and the app grafted
the results into the tree before the user's eyes. `/private/var` went from **7.4 GB to 11.2 GB**
in the same session, with `root` (645 MB), `dirs_cleaner` (2.2 GB), `vm` (2 GB) and `networkd`
appearing where unreadable petals had been. `protected` (20.9 MB) stayed restricted, exactly as
this claim's honest limit predicts: root does not bypass TCC.
**The ship/kill sizing rule now has its number.** Recovered space on a real volume, one pass, one
human approval: **+3.8 GB**, against a ≥1 GB ship threshold. Ship.
**AC2's elevated arm is still open** — the 1%-of-`sudo du -sk` corroboration needs a `sudo` run
this session did not take, and the RED control (the same pipeline unprivileged recovering exactly
zero) is covered by the IPC suite rather than by a live run.
**Anomaly — NOT REPRODUCIBLE, closed as a one-off rather than a defect.** The first pass began
without the "Measure restricted areas…" button being pressed; the only click was Scan. Two
hypotheses were tested and both are dead. *Persisted authorisation:* refuted by reading the code
— `admin_authorised()` consults `state.admin_session` in memory, `None` on a fresh launch.
*Code path:* refuted by a clean re-run on a fresh instance, which scanned the same root to the
same 7.4 GB with "some items restricted", **offered the enabled Measure button, raised no dialog,
started no helper and wrote no log entry**. Identical input, correct behaviour. The likely
explanation is a mis-delivered click from the automation harness rather than the app: the same
coordinates that press Scan in the overview also sit inside the stage once the view has changed.
Left in the record as a one-off observation, because "we could not reproduce it" is a weaker
statement than "it cannot happen", and a root-consent dialog appearing unasked is worth
remembering even at one sighting.
**Liveness confirmed on real hardware, for free.** Killing the app left the helper alive at +3 s
and gone by +8 s, so the heartbeat mechanism does its job without a test harness. The staging
directory survived the SIGTERM (a `TempDir` drop does not run on an abrupt kill), so on this path
the heartbeat is the mechanism that actually ends the session — worth knowing, because the design
lists three and only one of them fired.
**Platform note (2026-11-03):** the helper and its protocol ship unchanged in the macOS
and Linux artifacts. Windows ships no helper: the app re-execs itself elevated (UAC)
and the elevated instance serves the same protocol from the same code (`crates/admin`
lib). On Linux the Measure button reports "not available on Linux yet" and restricted
petals stay restricted (A3, A5 hold). Per-platform launchers for Linux are deferred
with reopen triggers in the multi-platform release design.

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

### C27 — Local snapshots are visible, and the purge tells the truth about privilege  `[x]`
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
**Appearance verified 2026-11-02, on the second attempt.** The first launch locked its screen
mid-verification and the claim stayed open rather than closing on tests alone; the retry did the
work. The overview carried a "Local snapshots" row reading `21 local snapshots · macOS does not
report how much each holds` with a Review action; the panel listed all 21 newest-last as
`1 Nov 2026, 13:01` and so on, showed the raw `com.apple.TimeMachine.…local` name on hover, and
offered `Delete snapshots…` — with no size beside a single row, which is the property the claim
turns on. Evidence: `/tmp/dg-snap-open.png` and `/tmp/dg-snap-panel.png`, pixels inspected.
**Not observed, stated rather than glossed:** the purge has never been executed against real
snapshots. This machine holds 24 of them and deleting them was not authorised, so the command
path is proven through the injected runner and the real refusal branch is inferred from
tmutil's own `requires root privileges` string. The first real purge is the observation that
closes that gap.

### C28 — Favourite folders persist, and a favourite that has gone says so  `[x]`
Any folder visible in a scan can be starred from the contents list, and the current folder from
the list's heading, without a dialog; whether a node is a favourite is resolved from its path on
the Rust side, never guessed in the UI. Favourites persist in the app's own data directory as
JSON and appear on the "Disks and folders" page above every other row, each with a Scan button
and an unstar. Starring a folder that is already a favourite — including the other spelling of a
symlinked path — is a no-op that never produces a second row. A favourite whose folder no longer
exists stays on the page, marked unavailable in visible text with its Scan disabled, until it is
unstarred.
**Probe:** `cargo test -p diskguardian-app --test ipc favourites` — persistence across a relaunch
on one store file; star-twice is a no-op; `/tmp` and `/private/tmp` spellings collapse; a deleted
favourite is returned with `exists: false`; files and missing nodes are refused; relative paths
are refused; a corrupt store is reported and reset, not silently emptied. `bun test` on
`favourites.ts` — the model never filters missing favourites and never swallows the recovery
notice.
**Falsifier:** a duplicate favourites row; a missing favourite silently dropped; a favourite lost
across relaunch; favourite state written anywhere by the webview rather than through the command
surface.
**Design:** `.claude/designs/2026-11-02-favourite-folders-design.md`. Two forks defaulted rather
than asked (2026-11-02): favourites sit above **every** row, home and Trash included, and a
case-only path difference is an accepted limit — it can produce a second row, visibly, and never
loses data.

**Closed 2026-11-02, on four refutation passes and a rebuilt bundle.** Evidence: `cargo test -p
diskguardian-app --test ipc favourites` — 18 tests, including persistence over one store file,
star-twice as a no-op, `/tmp` and `/private/tmp` collapsing, a vanished favourite returned with
`exists: false`, the legend's unstar emptying the list *after the folder is gone*, and unstar-then-
re-star leaving one row; `bun test` on the model and on a source-level markup guard. Three
discriminations RED-checked by hand (duplicate refusal, canonicalisation, the gone-folder ancestor
walk), each failing when its mechanism was neutralised. On the built app: the favourites block above
every row with Scan and unstar; the star click not moving the view; a favourite surviving a real quit
and relaunch as `/private/tmp/...` from a `/tmp` scan; and a vanished favourite rendering "not on this
disk right now" with its Scan disabled. Screenshots `/tmp/dg-r8-overview.png`, `/tmp/dg-r8-legend.png`,
`/tmp/dg-r8-missing2.png`, `/tmp/dg-r8-final.png`.
**What the passes cost, and why it was worth it.** The first build passed every gate and all three
pre-committed discriminations, and still shipped a legend unstar that silently did nothing. The
second pass found it; the third found that the repair worked only while the folder existed — the
increment's own headline scenario — and that re-starring then produced the duplicate row C28 names as
its falsifier. Two regression tests were green with the defect live because their fixtures kept the
folder alive. The fix that ended it is a single canonicalisation chokepoint the writer, the
comparator, the remover and the load-time reconcile all call, so partial application is not
expressible.
**Stated limits:** a case-only path difference can still produce a second row (visible, never data
lost), and `tauri.conf.json` still ships `csp: null` — the hover-label injection is fixed, but CSP
hardening is its own decision and was deliberately not folded in.

### C29 — The hidden residual is measured, and what cannot be measured is named  `[x]`
A scan whose root is a volume reports one "Hidden space" node whose width is that volume's used space
minus the measured tree, computed at view time from a fresh anchor so it self-corrects after deletes,
grafts and revalidates without ever entering the `Tree`. The walk stays on the scanned volume's device
— the boot volume group counts as one — never entering another disk or a snapshot mount, and the
mounts it excluded are listed. macOS' purgeable figure appears beside the residual as "up to N", never
as a slice of it and never as free space, because it overlaps bytes the tree already counts. A
negative residual is reported in words with the petal omitted, never silently zeroed; no anchor means
no node.
**Probe:** `cargo test -p diskguardian-core --test volume` — the residual is placed exactly (anchor =
tree ± K gives hidden = K; no anchor gives no node), a foreign `st_dev` subtree and an excluded prefix
leave the tree and are listed in `skipped_mounts`, and prefix matching is by component so excluding
`…/skip` does not swallow `…/skipme`. `cargo test -p diskguardian-app --test ipc hidden_space` — the
payload carries the anchor, the purgeable wording, and a held note in the backend's own words.
`bun test` on the model — it cannot render a snapshot size, cannot show an absent decomposition as
"none", and cannot swallow a negative-residual note.
**Falsifier:** a hidden petal whose bytes are not `volume used − tree total` at the moment of
projection; a purgeable figure presented as freed or as part of the residual; a volume scan that
enters another disk or a snapshot mount; a skipped mount absent from the report; a per-snapshot size.
**Narrowed 2026-11-02, by the rule's own verdict.** The design committed an acceptance rule before any
measurement: ship iff the identity tests pass, the reported used space agrees with `df` and `diskutil`
within 1%, and the residual lands in a pre-committed band. The run passed every numeric arm — oracles
0.00013% and 0.017%, purgeable matching Foundation exactly, skew 12 MB over 67 s — **but still-hidden
was 92.3% of the total, past the rule's 80% line, and the external-volume tie-breaker could not run
because that disk is the live Time Machine destination.** The rule said underpowered decides nothing,
so the decomposition (other container volumes, snapshot count, named restricted roots, the still-hidden
remainder) is **held**, and a backend-owned note says so on the panel rather than showing empty
categories. R15 carries its reopen trigger. No threshold was moved after the data was seen.
**Also fixed here, pre-existing:** the `/` walk entered an external backup volume and live snapshot
mounts, so the "Macintosh HD" figure could include another disk's bytes.
**Appearance observed 2026-11-03 on the built app.** A boot-volume scan drew `/` at 1.6 TB with a
purple **Hidden space** petal at 210 GB between Users and Applications, and the same figure in the
contents column; clicking it opened a panel reading "Hidden space · 210 GB of 1.6 TB used", the held
breakdown note in the backend's own words ("no category of the hidden figure is shown — not as a
figure, and not as zero"), purgeable as "macOS can free up to 196 GB ... Not part of the hidden
figure — much of it is space the scan already counts", and the nine excluded mounts listed by path.
Evidence: `docs/screenshot.png` (committed, and now the README's front-page image),
`/tmp/dg-shot-panel.png` for the panel.

### C30 — Every supported platform ships an installable, structurally verified artifact; Windows measures restricted space by self-elevation  `[ ]`
A tagged release attaches installable artifacts for macOS arm64 + x86_64 (`.dmg` + `.app`
`.zip`), Windows x86_64 (NSIS `.exe`), and Linux x86_64 (`.deb` + `.AppImage`) — the
single-file artifacts named with their full target triples (the `.app` zips, the
Windows setup, the Linux deb and AppImage), while the two macOS dmgs keep the bundler's
per-architecture names (`aarch64` / `x64`), which cannot collide with any other asset;
each verified before upload to contain exactly the binaries that platform ships
(macOS/Linux: app + helper; Windows: app only, with no `diskguardian-admin*` file
anywhere in the payload) — byte-identical by SHA-256 on macOS, Windows and the Linux
`.deb` (the build passes `--no-binary-patching`, so the bundler's in-place bundle-type
rewrite never runs; macOS needs no flag, the bundler already skips macOS-native
bundles) and, on the Linux `.AppImage` only, equal at ELF-content level: same machine,
GNU build-id, interpreter and `DT_NEEDED` list, RUNPATH exactly `$ORIGIN/../lib`, and
every section's content hash equal to the cargo build product except the
bundler-owned `.dynstr`, `.dynamic` and `.shstrtab` (linuxdeploy's RUNPATH
injection, its library deployment and its rewrite of the section-name table are
intrinsic to the format — a build-id survives all post-link mutation, so no
single-file hash there would carry meaning; the section-NAME set is still
asserted equal and that is what carries the semantics, since a renamed, added or
dropped section always fails it); and the release body
naming what each platform's OS will warn about (Gatekeeper, SmartScreen, unsigned deb /
AppImage FUSE).

On Windows, measuring restricted areas re-executes the app elevated (UAC `runas`), the
elevated instance serves the unchanged job/results protocol and exits with the session,
and — unlike macOS, whose privileged process is a measurement-only helper — the
privileged process is the app binary itself, gated by a pre-Tauri argv branch that
initialises no window, webview or command surface. That difference is stated, not
hidden. A UAC prompt cannot be dismissed programmatically; an abandoned prompt parks
that one pass until answered or the app exits, and no second prompt can stack.

**Probe:** (CI arms, close on workflow evidence) the release workflow's per-leg Verify
steps — expected-set hash equality plus the Windows negative assertion, hashes printed
in the job log; on Linux the AppImage arm's per-section sha256 manifest, build-id pair,
RUNPATH and `NEEDED` comparison — the release lane's exact-count and duplicate-name
assertions, and the
post-tag download re-hash; the injected-launch IPC tests (decline maps to the exact
refusal sentence with the tree untouched; one launch per app session; the classifier),
run headless in CI.
**Probe:** (consent arms, open until the Stage H record lands) one real UAC prompt per
app session — not one per scan — seen and photographed by a human on real Windows;
decline shows the refusal banner and changes nothing; the elevated instance grafts
measured petals; it is gone within its 30-second heartbeat budget after the parent is
force-killed. GitHub's Windows runners are already elevated, so the consent prompt
never fires there; these arms cannot close on CI evidence and do not pretend to.
**Falsifier:** any shipped platform artifact whose payload lacks a binary in its
expected set (or, on Windows, contains one outside it) by hash; any single-file asset
named without its full triple, or a dmg name colliding with another asset; a tag whose
draft release carries fewer assets than the matrix produces; a release body omitting a
platform's warning; an AppImage payload whose build-id, interpreter, `DT_NEEDED` list or
any non-exempt section hash differs from the cargo build product, or whose RUNPATH is
anything but exactly `$ORIGIN/../lib`; a `.deb` or NSIS payload that is not
byte-identical (patching is off, so any difference names an unknown mutator and stops
the release); a second UAC prompt for a second pass in one app session; an
elevated instance still alive past its budgets after the parent dies.

**Status:** opens `[ ]`. The CI arms close when the 0.1.1 tag is audited; the consent
arms stay open until the Stage H hand-test record exists, even if the artifact has
shipped in the meantime.

## Test Strategy

| Layer | Probe |
|---|---|
| Scan engine arithmetic | `cargo test -p diskguardian-core` on synthetically built fixtures |
| Layout arithmetic | frontend unit tests (`bun test`) on the pure layout + hit-test functions |
| Integration | `cargo test -p diskguardian-core` — `crates/core/tests/scan.rs` builds real fixtures on disk |
| Appearance | launch the built app, `screencapture`, view the pixels |
| Behaviour | drive the running app (hover, drill, collect, delete) and re-read state |

## Decisions

- **A tag publishes the release; there is no publish step** (2026-11-05, principal directive). `release.yml` created every release with `--draft`, and GitHub never promotes a draft, so `v0.1.1` through `v0.1.4` left the page reading **Latest: v0.1.0** while `Cargo.toml`, `tauri.conf.json`, `package.json`, the tag and every asset filename already read 0.1.4. Four releases were invisible behind one manual flip that never happened. The asset set is verified before the release step runs, so the draft bought nothing; it is now created published, and publication is proved from the API afterwards (asset count equals 7, nothing below the 1,000,000-byte floor, `isDraft == false`) so a partial upload fails the job instead of shipping a hole. Re-running a tag re-uploads with `--clobber` and re-asserts `--draft=false`. `v0.1.4` was published by hand once to correct the standing state; `v0.1.1`–`v0.1.3` stay drafts as superseded builds.
- **Structure is never pruned, only files are** (2026-11-04, principal elected after seeing it live). The view folded any child past 8 kept siblings once 99.5% of the parent was covered, which on a real Windows profile meant `AppData` filled the quota and the thirteen empty folders beside it — Documents, Downloads, Desktop, Pictures, Videos and the rest — all vanished into `smaller objects`. The disk was reporting its space correctly and still lying about what was there. Size pruning now folds files only; folders are kept to the `max_children` cap, which is still the hard bound. Two passes rather than one, because a big file otherwise satisfies the coverage rule and breaks the loop before reaching folders that sort after it on zero bytes.
- **Tauri 2 + Rust core over egui or SwiftUI** (2026-10-30, principal elected). Rust owns scanning, tree building, deletion and macOS integration; the system webview owns the canvas. Chosen because visual fidelity to DaisyDisk is a rendering problem the web canvas solves directly, while the hard cross-platform part (physical-size accounting, traversal) stays portable Rust.
- **Workspace, not one crate.** `crates/core` has no UI dependencies so it is unit-testable and reusable by a future CLI.
- **Physical sizes via `st_blocks`.** `len()` would over-report sparse files and under-report block rounding.
- **Trash by default, permanent as an option.** DaisyDisk deletes permanently; an open-source tool should not, by default.
- **Layout computed in the frontend.** Rust ships the tree and sizes; angles are cheap and belong next to the renderer that draws them.
- **No `PathBuf` per node** (2026-10-30, measured). The first build stored an absolute path on every node; a scan of the home directory reached 2.7 GB of RSS and still had not finished. Paths are now reconstructed from the name chain by `Tree::path_of`, which is called a handful of times per user action instead of millions of times per scan. Same scan: 417 MB.
- **Progressive paint, not lazy loading** (2026-10-30, principal asked whether to scan on demand). Lazy expansion cannot draw this product's first frame: a petal's width IS its subtree's total, so from one `readdir` every folder would render at near zero and a 50 GB folder would look identical to an empty one. Lazy also re-walks a subtree on every re-entry unless cached, which is a full scan with extra steps. The scan now emits each root child the moment its subtree finishes and the UI draws equal-width "measuring" petals that grow into place, so the total cost is unchanged and the perceived wait collapses to about a second.
- **Walker benchmarked against `du`, not a target** (2026-10-30, `du -sh ~/Library` = 79.6 s vs our 87.9 s on 974,623 files). The walk is within 10% of the filesystem floor for that tree, so the original "10 seconds" C14 target was invented rather than measured. **Correction (2026-11-02):** the earlier note that our kernel time is "double `du`'s, from a redundant per-directory stat" does not survive scrutiny. `sys` is summed across all threads, and this walker is parallel while `du` is single-threaded, so the comparison says nothing about extra syscalls. There is one `stat` per directory and there is no free way to remove it (the parent's directory entry does not carry the child directory's block count). R2 is therefore closed as not-a-defect rather than fixed.
- **Delete prunes, it does not rescan** (2026-10-30). The first working version re-scanned the whole volume after every delete, which is minutes on a real disk and made the feature feel broken. `Tree::forget` now removes the deleted nodes, recomputes ancestors and re-projects the view, so the flower updates in the same frame.
- **A volume scan is bounded by device** (2026-11-02, C29). The root's `st_dev` defines what the walk
  enters; the APFS boot group presents System and Data as one id, which is exactly the disk a user
  means by "the startup volume". The inode ledger alone was measured insufficient — directory
  metadata double-counts and snapshot mounts share the device id — so two fixed macOS prefixes are
  excluded and any directory excluded that way is listed rather than silently skipped.
- **Purgeable space is an annotation, never a slice** (2026-11-02, C29). Measured at 164 GB on a 2 TB
  boot container while it overlaps bytes the tree already counts; presenting it as reclaimable space,
  or as part of the hidden residual, would double-count. It is reported as "up to N" and nothing
  offers to purge it — the only purges in this product are the user's own snapshots (C27) and the
  Trash (C21).
- **Trash by default, Option for permanent** (2026-10-30). The button moves to the Trash; holding Option deletes permanently and always confirms. Trash runs over 5 GB also confirm.
- **Favourites are Rust-owned data, not webview state** (2026-11-02, C28). A JSON store in the app
  data directory, reached only through commands. `localStorage` is for UI preferences that degrade
  to a default — the legend width — not for user data that must survive a relaunch, be testable
  without a DOM, and stay out of reach of webview store corruption. Only Rust can turn a node id
  into a path anyway.
- **The canonical path is the favourites key** (2026-11-02). Writes canonicalise best-effort;
  comparisons canonicalise both sides, because `path_of` returns literal walked paths while
  everything filesystem-side canonicalises. That is the same mismatch that once made C17's delete
  prune nothing, and here it would surface as duplicate rows. Case-folding is a stated limit: a
  case-only variant can produce a second row, visibly, and never loses data.
- **The star lives in the legend, not the top bar** (2026-11-02). A star on each directory row and
  one on the heading for the current folder — which is also what makes the scan root starrable. No
  modal, and no new row in a bar that is already dense.
- **Admin scanning is prompt-per-session, not a signed helper** (2026-11-02; cadence set by the
  principal: "one per privileged session"). Merged to `main` as `b794c4d` from branch
  `r3-session-scanning` (`fd853b1`) — corrected 2026-11-02, because this entry claimed "not yet
  merged" while the merge had been in history for some time. See the C23 status above. The original design's §10 said a resident privileged
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
- **Windows elevation is self-elevation through the existing launcher seam** (2026-11-03,
  maintainer ruling). The app re-execs itself with `runas`; the elevated instance takes a
  pre-Tauri argv branch and serves the unchanged job/results protocol. Accepted trade,
  stated in C30: the Windows privileged image is the whole app binary, not a
  measurement-only helper; the argv branch that selects serving constructs no window,
  webview or command surface, so no deletion code path is reachable elevated. The single
  `ShellExecuteW` lives in a ~60-line windows-gated crate (`crates/win-elevate`) so
  `crates/admin` keeps `unsafe_code = "forbid"` and the app crate gains no `unsafe`.
- **Release artifacts are hash-verified against the binaries cargo built for the
  triple, against per-platform expected sets** (2026-11-03). Presence-by-name is what a
  bundler promises; content-identity is what a user installs. The hash check also
  proves architecture for free and survives bundler renaming; the per-platform set plus
  the Windows negative assertion make "what ships" a decision the config makes and the
  pipeline enforces.
- **Non-macOS payloads were never byte-identical; the fix removes the rewrite rather than
  loosening the check** (2026-11-03, after run 41635246338). Tauri's bundler overwrites a
  27-byte link-time placeholder (`__TAURI_BUNDLE_TYPE_VAR_UNK` → `_DEB`/`_APP`/`_NSS`) in
  place at the cargo output path before each non-macOS bundle and restores it afterwards, so
  the packaged main binary differed from the reference by exactly three bytes (measured:
  offsets 733413–733415, `UNK`→`DEB`, no rpath entry either side). macOS passed only because
  the bundler skips macOS-native bundles, and the sidecar matched everywhere because sidecars
  are copied, never patched. linuxdeploy separately rewrites **both** AppImage binaries with
  `RUNPATH $ORIGIN/../lib`, growing them. The rejected alternative was a bounded carve-out
  permitting the three-byte slot; a permitted deviation is a claim that has to be explained at
  every review. The accepted fix is `--no-binary-patching`, which makes the `.deb` and NSIS
  genuinely byte-identical and costs only per-bundle-type updater support — a feature this
  project does not ship, reopen if an updater is ever adopted. The AppImage keeps an
  ELF-content check because its rewrite is intrinsic to the format. A GNU build-id survives all
  post-link mutation, so it is a necessary condition and never an integrity check.

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
- **R15 — The hidden-space decomposition** (opened 2026-11-02, C29's held half). Other container
  volumes, the snapshot count, the named restricted roots and the still-hidden remainder. Its
  machinery is written and tested (`parse_containers`, `decompose_other_volumes`, `container_report`,
  `examples/hidden_measure`), and nothing on the shipped path calls it. **Reopen trigger:** run
  `cargo run --release -p diskguardian-app --example hidden_measure -- /Volumes/<external>` against a
  second APFS volume while it is idle, and land the decomposition only if the cross-check brings the
  still-hidden share under the rule's 80% line. Measured on the boot volume alone it was 92.3%, which
  is the reason it is not shipped.
- **R14 — Concurrent scans are served fairly, or refused** (opened 2026-11-02 from C13's
  narrowing; **half landed 2026-11-02**). A superseded scan is now asked to stop and, if it
  observes the request, returns `ScanError::Cancelled` with no tree — checked before the walk,
  per directory, per listing entry, per measurement entry, and before the directory dispatch.
  What remains open, named rather than implied: the flag is **one slot per app**, so the
  operation it can express is "cancel the most recently registered walk", not "cancel exactly
  the scan I superseded" — a canccel arriving during the superseded scan's `peekRoot` finds an
  empty slot and lets that walk run. Making it per-generation needs the request to carry an id;
  that and fair scheduling are the rest of this row. There is also no Cancel button yet, so
  every canceller is a superseding scan and the "cancelled reads as failed" path is unmasked
  but untested in anger. Today a scan started behind a large one does not complete until the large one
  does: both walks submit to one rayon pool and nothing cancels the superseded one. Two honest
  shapes, and the cheaper one is probably right — either refuse to start a scan while one is in
  flight and say so on the button, or give each scan its own pool and add a cancel path through
  the IPC so a superseded walk stops instead of finishing invisibly. Until one lands, the UI
  should not offer a control whose outcome is "nothing happens yet".

## Not yet specified

In scope eventually, too dim to state as claims today:

- Streaming traversal, so a scan of a multi-million-file volume does not hold every node in
  memory at once. *(The flat-arena half landed in `857d662`; only streaming remains, and C14's
  measured 362 B/node means it is a ceiling-raising exercise rather than a current defect.)*
- APFS clone detection (`ATTR_CMNEXT_CLONEPARENT`), the largest known accuracy gap.
- Incremental rescan: reusing a previous tree and re-walking only paths whose mtime changed.
- Admin/root scanning and the privileged helper that would carry it.
- Animation on drill-down, which DaisyDisk has and v0.1 does not.

## Log

- 2026-11-04 — Multi-platform release, Stage C **landed**: PR #3 merged through
  `scripts/merge-verified.sh` as merge commit `f892fda`. The gate polled the checks to terminal
  state, printed the complete rollup — four checks, **0 fail, 0 pending** — and only then merged. The
  draft flag was the real blocker all along, and it was cleared before the attempt rather than
  discovered mid-merge. `v0.1.1` is now an ancestor of `main`, so the tag and the code it names are
  reachable from the default branch, which the earlier record could not yet claim. What remains open
  is unchanged and deliberate: **C30 stays `[ ]`** because its consent arms need a human at a real
  Windows machine (Stage H), and the draft release stays unpublished until the principal says
  otherwise. The increment is landed but not closed.

- 2026-11-04 — Multi-platform release, Stage C **in flight, not landed**: PR #3 was still a
  **draft**, which is what the GitHub `BLOCKED` state actually meant, so it was marked ready before
  any merge attempt. `scripts/merge-verified.sh 3` is now polling the check rollup to terminal state
  and has **not** merged. The record is being written while it runs rather than after, because the
  outcome belongs to the tool: the gate prints the complete rollup, refuses on any fail, pending or
  empty result, merges with a merge commit, and leaves the remote branch in place since remote
  deletions take the principal's word. Stage B is already complete at `3544a6e` (tag `v0.1.1`, draft
  release, seven assets, two of them re-hashed on download), so `v0.1.1` becomes an ancestor of
  `main` only once this merge lands; that ordering is a consequence of the merge, not a claim about
  it. C30 stays open on its consent arms regardless of the merge, because Stage H still needs a
  human at a Windows machine.

- 2026-11-04 — Multi-platform release, Stage B: tag `v0.1.1` cut at `3544a6e` — the head that went
  green, not `main`, which PR #3 has not reached, so the tag is not an ancestor of `main` until that
  merge lands. The tag-triggered run (41745364730) went green on all four legs **including Attach to
  the draft release**, and the draft "DiskGuardian v0.1.1" carries exactly the seven expected assets,
  every one non-zero: two `.zip`, two `.dmg`, the Windows `-setup.exe`, the `.deb` and the
  `.AppImage`. The single-file artifacts are triple-named and the two dmgs keep the bundler's
  architecture names, which is what C30 now says. **Post-tag download re-hash:** the `.deb`
  (`4bc1fc4e…`) and the `.AppImage` (`15223b1b…`) came back byte-identical to the digests the verify
  step printed, so the attach step did not alter them. The other five carry no printed digest to
  compare against, and that limit is stated rather than glossed. **C30 stays open:** this closes its
  CI arms, but the consent arms wait on Stage H and no human has yet seen a UAC prompt from this
  build. The draft is deliberately left unpublished.

- 2026-11-04 — Multi-platform release, Stage A green and the RED controls run: **five of five pass
  as designed, after one partial found a defect in our own instrumentation rather than in the
  artifacts.**
  Stage A's honest green is iteration 7 (run 41736259266, all four legs, **zero MISS lines**, the
  only skips being the three named exemptions `.dynstr`, `.dynamic`, `.shstrtab`), with `.text`
  matching genuinely on both binaries and the deb byte-identical. PR #3's four CI lanes are green
  alongside it. Each control ran on its own scratch branch off `8af7d19`, one sabotage each —
  stacking was rejected up front because R-neg's assertion sits after the phantom check and a
  stacked run would have failed for the wrong reason. Four failed as designed: **R-phantom**
  (41740327048) through the payload lookup, with the macOS leg honestly noted as failing earlier,
  at the payload hash, because the phantom's payload path is absent from the bundle; **R-strip**
  (41741129856) at the presence check; **R-neg** (41741242464), where a planted mixed-case
  `DiskGuardian-Admin.exe` was enumerated, counted at 1, named, and refused — the control that
  pins the app-only ruling; **R-mutate m2** (41741474090) on **both** arms, with the deb diagnostic
  reporting "total differing bytes vs reference: 1" against the correctly chosen candidate and the
  AppImage arm reporting `.rodata` differs. **R-mutate m1** (41741360938) is a **partial**: the deb
  arm fired correctly, then the step died in the iteration-2 diagnostic block before the AppImage
  arm ran, for two reasons of different size — the candidate find used `-name "$bin*"` and
  nominated `diskguardian-app.png` from the deb's icon tree, dumping 21,603 lines of cmp noise, and
  `printf '%s\n' "$DIFFS" | head -1` took SIGPIPE when head closed, which `pipefail` turned into
  exit 141 and `set -e` turned into a dead step. Measured locally: the pipe form returns 141, the
  herestring form returns 0. Both are fixed in `bbd9e94`, and **m1 then re-ran and passed on both
  arms** (run 41743288948, branch `red-mutate-text` at `91b422f`, one sabotage commit on
  `cfe3ac9`): the diagnostic named the right candidate, `debx/usr/bin/diskguardian-app`, reported
  "total differing bytes vs reference: 1", printed both 128-byte context dumps from offset
  1577417, and the step **survived its own miss path** to reach the AppImage arm, which reported
  `.text` reference `04a70e15…` against payload `5697f08f…`. That payload value is precisely what
  the green runs print for the genuine `.text`, so the reference sits exactly one byte away from
  what shipped — the control's whole claim, corroborated by the numbers rather than asserted. The
  admin sidecar's sections then compared clean on the same run, so the step completed a full pass
  with only its two designed failures. **The controls are five of five**: phantom (all four legs),
  strip, neg, m1 and m2; the acceptance rule's RED precondition is satisfied on evidence.

- 2026-11-03 — Multi-platform release, amendment 2, consuming no Stage A iteration: the Linux
  verification failure is root-caused and the premise corrected instead of weakened. The mutator
  is Tauri's bundler itself — `patch_binary` in `crates/tauri-bundler/src/bundle.rs` at
  `tauri-cli-v2.12.4` rewrites the 27-byte `__TAURI_BUNDLE_TYPE_VAR_UNK` placeholder in place at
  the cargo output path before each non-macOS bundle and restores the original afterwards. The
  run's own `cmp -l` was complete and is the whole proof: exactly three bytes, 733413–733415,
  `125 116 113` → `104 105 102`, `UNK`→`DEB`, with the sidecar matching by hash on the very next
  line because sidecars are copied rather than patched. linuxdeploy then rewrites both AppImage
  binaries with `RUNPATH $ORIGIN/../lib`, growing the app by 33,825 B and the sidecar by 6,169 B.
  **A GNU build-id survives all post-link mutation** — the deb and AppImage copies share
  `06c35623…` while both differ from the reference and from each other — so it can never be an
  integrity check, only a necessary condition. Correction: `--no-binary-patching` (its only cost
  is per-bundle-type updater support, and this project ships no updater), which makes the `.deb`
  and NSIS byte-identical exactly as C30 originally said, leaving the AppImage on an ELF-content
  check — machine, build-id, interpreter, `DT_NEEDED`, RUNPATH asserted exactly
  `$ORIGIN/../lib`, section-name set, and every section hash equal except the bundler-owned
  `.dynstr`/`.dynamic`. `R-mutate` tightens to two named mutations, one byte inside `.text` and
  one inside `.rodata`; the residual blind spot, bytes outside every section, is stated rather
  than hidden. Iteration cap: 2 spent, the amendment spends none.

- 2026-11-03 — Multi-platform release, Stage A attempt 2 of a pre-committed 5: **the same two
  legs failed, for two new reasons**, and the adversary lane returned a HIGH that outranks both.
  Windows: `E0308` twice at `admin.rs:637` and `:650` — the launcher returned `ElevateOutcome`
  where the seam wants `LaunchOutcome`, two distinct structs with identical fields, so the mapping
  the crate's own doc comment promised had never been written. Fixed by mapping once at the seam.
  It cannot be proven on this machine (it is `cfg(windows)` code) and is not claimed to be; the
  Windows CI leg is the proof, which is the same standing this increment already gave the Windows
  compile. **Linux: the hypothesis from attempt 1 is confirmed.** The instrumentation printed the
  reference `diskguardian-app` with no RUNPATH, the AppImage payload carrying
  `RUNPATH: [$ORIGIN/../lib]`, and `diskguardian-admin` matching its reference **byte-for-byte** in
  the deb. Byte-identity is therefore unachievable for the app binary on Linux, so per the
  pre-committed rule the correction goes to the design and C30 rather than to a looser check.
  One thing is *not* explained and is recorded as unknown rather than smoothed over: the **deb**
  payload differs at an identical size (7292936) with no RUNPATH either, so the rpath story covers
  the AppImage and does not yet cover the deb. Designer lane opened for that; C30's Linux clause
  still says "byte-identical by SHA-256" and is known false until it lands.
  **Adversary REFUTE (537b01e..2178102): the A1 privileged-image claim survives**, after 23
  classifier shapes executed against a vendored copy, an executed empty-argv `execve`, and the
  elevation cage read against the windows-sys source. One HIGH and three LOWs came back.
  **F1 HIGH, folded.** The Windows elevation path was **unreachable from the shipped UI**:
  `main.ts:363` gated the affordance on `isMacPlatform(navigator.userAgent)` — `/Mac/i`, written in
  the macOS-only era — and this branch added the Windows runner in Rust while touching **zero**
  frontend files. On the 0.1.1 Windows artifact a partial scan never rendered Measure, so
  `scan_restricted` was never invoked, no UAC prompt could fire, and the classifier, the pre-Tauri
  branch, the win-elevate cage and the NSIS negative assertion were all dead code in the product,
  while the README and C30 described that behaviour as shipping. No CI arm could see it: the ipc
  tests drive the Rust command directly and no Windows UI test exists. Fixed by replacing the
  OS-name question with the runner question, `hasAdminRunner` (macOS or Windows; Linux still has no
  runner and must not offer the button). RED, quoted: `SyntaxError: Export named 'hasAdminRunner'
  not found in module .../src/admin.ts` (0 pass, 1 fail). GREEN: 65 pass, 0 fail, tsc exit 0.
  **F2 LOW, folded.** `tests/serve_order.rs` located the dispatch with
  `source.find("admin_serve_mode")` — first occurrence, comments included — so a comment above
  `run()` that merely named the function satisfied the ordering assertion while the dispatch sat
  below the launch, and an elevated copy would have opened the full Tauri app with the guard green.
  The lookup is now comment-blind and requires a call; a second test pins the doctored shape and
  asserts the naive search *is* fooled, so the fix is proven to discriminate rather than asserted
  to. GREEN: 26 lib, 47 ipc, 2 serve_order, 1 crate_names, clippy clean.
  **Row-call, not folded:** F3 the admin audit log hardcodes `$HOME/Library/Logs/DiskGuardian` on
  every platform (on Windows `HOME` is usually unset, so that record silently does not exist);
  F4 `ShellExecuteW` returns after consent, so a child that dies before readiness makes a user who
  just approved UAC wait the full 1020 s bound, and the staging path is quoted rather than escaped;
  F5 C30's liveness wording says "0700 staging directory", which is the macOS mechanism, while
  Windows exclusivity is `%TEMP%`'s inherited profile ACLs.
  Worth keeping: this PR is the **first Windows gate this branch ever had** — pushes without a PR
  trigger no workflow — and the lane that would have caught F1's compile class is the PR CI lane.

- 2026-11-03 — Multi-platform release, Stage A attempt 1 of a pre-committed 5: **two of four legs
  failed**. macOS arm64 and x86_64 passed, including the new hash verification. Windows failed to
  build — `E0433: cannot find module or crate win_elevate` at `admin.rs:633`, because the package is
  named `diskguardian-win-elevate` and exposes `diskguardian_win_elevate`; the call sits behind
  `#[cfg(target_os = "windows")]`, so every macOS gate is genuinely green and blind to it. Linux
  built its deb, extracted it, and then matched no payload file to the reference hash; the cause is
  **not yet known** and the leading hypothesis (the bundler's `patchelf` pass rewriting the ELF) would
  make the design's byte-identity premise wrong for Linux rather than the workflow wrong. Instrumented
  before the rule is touched, per the pre-committed rule.
  Two findings that outlive this attempt. **`ci.yml` triggers on `push` to `main` and on
  `pull_request` only**, so a branch push with no PR runs no gate at all — that is how a Windows
  compile break reached the release lane unseen, and an early draft PR is now part of the loop.
  **The Gortex daemon served the primary checkout's bytes for this worktree** (it returned an `ISA.md`
  at `progress: 24/29` with no C30 while this file says 27/30), so any read through Gortex on a
  non-primary worktree is `main`'s content; use `git` and native reads there. *(The counter was
  separately repaired from a stale 24/29 to 27/30 with no claim ticked in that range: the base was
  already three ahead of its stated count, which is why the stale read was plausible rather than
  obviously wrong.)*
  Review lanes on the built branch: governance returned FIX FIRST — C30's evidence lines begin
  `**Probe (…):**` and `check-isa-claims.ts` accepts only `**Probe:**`, so ticking C30 at the 0.1.1
  audit would fail the gate deterministically; and "every artifact named with its target triple" is
  false of the two dmgs, which keep the bundler's architecture names, so C30's own falsifier catches
  its own release. Code review found one blocking defect (the same Windows break) and re-proved the
  macOS hash check byte-identical against a real bundle locally. Adversary still out. C30 stays open,
  progress unchanged at 27/30.

- 2026-11-02 — Favourite folders shipped (C28), and the first increment run end to end under the
  new `increment` skill. What landed: a Rust-owned JSON store in the app data directory, star from a
  directory row or the current folder's heading, favourites on the front page above every other row
  with Scan and unstar, and a vanished favourite that stays visible saying so instead of disappearing.
  Verified on the built app — block above every row, star click not moving the view, survival across a
  real quit and relaunch as `/private/tmp/...` from a `/tmp` scan, and a hidden folder rendering "not
  on this disk right now" with Scan disabled (`/tmp/dg-r8-overview.png`, `/tmp/dg-r8-legend.png`,
  `/tmp/dg-r8-missing2.png`, `/tmp/dg-r8-final.png`); plus 18 favourites IPC tests, three
  discriminations RED-checked by hand, and 105 Rust / 56 frontend tests green.
  **What it cost, recorded because it is the useful part.** Four review passes. The first build
  passed every gate and all three pre-committed discriminations and still shipped a legend unstar
  that silently did nothing. The second pass found it; the third found the repair held only while the
  folder existed — this increment's own headline case — and that re-starring then produced the
  duplicate row C28 lists as its falsifier. Two regression tests were green with the defect live
  because their fixtures kept the folder alive. The fix that ended it is one canonicalisation
  chokepoint that the writer, comparator, remover and load-time reconcile all call.
  **Left open by decision:** `tauri.conf.json` still ships `csp: null` (the hover-label injection is
  fixed; CSP hardening is its own call), and a case-only path difference can still yield a second row.

- 2026-11-02 — Cancellation verified end to end on the merged bundle. A volume scan running,
  then the fixture selected behind it: the fixture completed in **0.1 s** and owned the screen.
  The same sequence before this work left a 12 MB scan still measuring at 75 seconds. Also
  recorded from the same run: scanning `/` raises a macOS **network-volume consent** dialog,
  because the walk enters mounted shares. Denying it leaves the app working and reports the
  share's contents as unavailable — which is the right behaviour, but it means a whole-volume
  scan interrupts the user with a permission prompt the product never explains. Not a defect
  in this diff; noted because it is the first thing a new user will hit.

- 2026-11-02 — Review found what the gates could not. Three lanes (code review, refutation,
  mechanism critique) ran against the day's fixes and produced sixteen findings, six of them
  MEDIUM or blocking. The pattern worth keeping: **every one that mattered was found by reading
  the plumbing, not by running the thing.** The scan-token fix stopped a stale listener painting
  but never stopped a fresh listener listening, because `scan-progress` carries no scan identity
  — the screenshots were of a fixture whose trees shared no child names, which is exactly the
  case that hides it. The cancellation flag was ignored entirely for a file root. The
  measurement loop was unpolled, so cancelling a wide directory still stat'ed everything already
  collected (3.9 µs/entry measured). And a privileged pass could overwrite a newer scan's view,
  because `AdminScanReport` named no scan. All folded: progress events are stamped with a
  per-start nonce and filtered on it, a cancelled walk emits nothing, both root shapes honour the
  request, the measurement loop polls, the admin report carries its `scan_id` and the frontend
  drops a report for a scan it is no longer showing, `beginScan` stops the old animation, and the
  stage is cleared before the one await in `scanPath` rather than after it. Two of my own
  claims were wrong and are corrected above rather than edited away.

- 2026-11-02 — C13 closed and narrowed; C8 closed. Driving the built app verified both
  directions of navigation (petal re-roots, centre restores exactly) and showed that a scan
  never freezes the window — progress climbed, petals grew, clicks landed mid-scan. What it
  also showed is that a second scan started behind a large one does not get served: 75 seconds
  in, a 12 MB scan was still measuring while a volume scan ran. So the claim no longer asserts
  concurrent scans as a feature; the token keeps them from corrupting each other's view, and
  fair scheduling became R14.

- 2026-11-02 — Fixed: two scans merged into one view. Starting a second scan while one ran
  produced one scan's breadcrumb over another's totals, a white petal, and a permanent wedge at
  a progress count that never advanced. Every scan start now takes a token: a stale-token scan
  drops its branch reports and its result without touching the stage, and skips `stopPartial` so
  it cannot clear the newer scan's measuring petals. Verified by repeating the sequence on the
  merged bundle — the mixed state is gone; the older walk still finishes unseen, which is R14.

- 2026-11-02 — Fixed: a new scan showed the previous directory's contents. Reported by the
  principal. Reproduced on the built app — selecting a new folder left the contents list reading
  the old tree's rows and totals and the breadcrumb reading the old path for the whole of the
  next scan, and for ever if that scan failed before its first paint. The canvas was being
  replaced by the progressive-paint path but `renderLegend()` and `renderCrumbs()` were not, and
  both read `state.stack`, which still held the previous scan. `beginScan()` now clears the
  stack, the collector and the breadcrumb before the first `await`, and the breadcrumb keeps its
  "Disks and folders" escape when there is no trail so a long scan cannot strand the user.
  Verified by driving the built app: scanning `/Users/seaton/Documents/src` (51.7 GB, 999,187
  files, 17.3 s) and then switching volume showed an empty contents list and the new path,
  where the same action previously showed the old directory's 20 rows. **No unit test**: the
  state lives in `main.ts`, which touches the document at import, so there is no seam — the
  behaviour is closed by observation, as C8, C13 and C15 were, and that gap is stated rather
  than papered over. Also banked: `stage-admin.ts` copies the helper from a hard-coded
  `../../target`, so building a worktree with `CARGO_TARGET_DIR` set fails at the copy step.

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
