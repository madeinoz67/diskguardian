# Multi-platform release artifacts — design

Date: 2026-11-03 · Author: increment-designer · Status: proposed, **amended once** (maintainer rulings, same day; no code was built against the original text — the builder was stopped before merging)

**Amendment 1 — what changed vs what stood.** The maintainer ruled:

1. **Windows ships the app binary ONLY.** No `diskguardian-admin.exe` in the NSIS
   installer. The original design's "the helper ships in every artifact" premise is dead
   for Windows; the `resolve_helper()` `.exe` defect found by the original design is
   therefore unreachable in shipped builds (no sidecar to resolve) and its 3-line fix is
   withdrawn — superseded, not landed.
2. **Windows elevation is IN this slice, as self-elevation.** The app re-execs itself
   elevated via the `runas` verb; the elevated instance serves the existing job/results
   protocol. No sidecar, no `externalBin` on Windows.

Standing unchanged: version 0.1.1, NSIS only (no msi), macOS lanes byte-for-byte as they
are today, Linux ships the helper with elevation still deferred, no frontend gating of
the Measure button, C30 is a claim only, and the whole verification/release-lane
apparatus of the original design (hash-based verify, triple-named assets, exact-count
assertion) stands — with verification now **per-platform expected sets** (§3.3).

**High-risk areas, named in the first paragraph as required.** This increment touches
**none of the three engine high-risk areas** — physical-size accounting, restricted-space
reporting and guarded deletion are unchanged in behavior on every platform. It touches
the **distributed form of the restricted-space story** twice over: the admin helper ships
in macOS/Linux artifacts only, and on Windows the *privileged process becomes the app
binary itself* — a deliberately accepted widening of the privileged image (§3.7) that the
claim text states rather than hides. Deletion stays unprivileged everywhere; the
privileged path on every platform measures and never deletes (A1, A5 hold).

---

## 1. Premise verified — the symptom is real and its cause is in one job

Everything below was read this session, not assumed.

- `.github/workflows/release.yml`, `build` job: `runs-on: macos-latest` with a matrix of
  exactly `aarch64-apple-darwin` and `x86_64-apple-darwin`; build step
  `bunx tauri build --target <triple> --bundles app,dmg`; verify step shells `lipo` and
  `test -x` against `Contents/MacOS/{diskguardian-app,diskguardian-admin}`; upload takes
  `bundle/dmg/*.dmg` and `bundle/macos/*.app`. No Windows or Linux leg. That is the whole
  defect: the pipeline can only ever attach macOS assets.
- Published v0.1.0 (`gh release view`, this session): four assets, all macOS —
  `DiskGuardian-aarch64-apple-darwin.zip` (2,955,743 B), `DiskGuardian-x86_64-apple-darwin.zip`
  (3,149,897 B), `DiskGuardian_0.1.0_aarch64.dmg` (3,009,779 B), `DiskGuardian_0.1.0_x64.dmg`
  (3,200,566 B).
- `apps/desktop/src-tauri/tauri.conf.json` (line 33): `"targets": ["app", "dmg"]` — invalid
  bundle targets on Windows (`nsis`/`msi`) and `dmg` invalid on Linux, so a local
  `bun run tauri:build` outside macOS fails today. The config must move per-platform.
- `.github/workflows/ci.yml`: the `engine` job already runs `cargo test -p diskguardian-core`
  on `macos-latest`, `ubuntu-latest`, `windows-latest`, installs the Linux webview deps,
  and `cargo check -p diskguardian-app` with `-D warnings` on all three. **Unconditional
  compilation on Windows and Linux is proven in CI**; only bundling is new. (CI runs no
  app-crate *tests* today — §3.7.5 adds them where the mock runtime makes them headless.)
- `apps/desktop/scripts/stage-admin.ts`: already triple-portable, including the Windows
  `.exe`-after-triple rule for `externalBin`. Under Amendment 1 it is **skipped on the
  Windows lane** (`if: runner.os != 'Windows'`) and unchanged for macOS/Linux. Its known
  `../../target` hard-coding does not fire on CI runners (already in the ISA log).
- `crates/admin/src/main.rs`: portable; the only platform gate is `#[cfg(unix)]` around
  `world_readable` (a chmod) with a `#[cfg(not(unix))]` no-op. On Windows the elevated
  writer's files are read by the unelevated parent through the staging directory's own
  inherited ACLs (§8, risk 3).
- `apps/desktop/src-tauri/src/admin.rs`: the seam is exactly right and was built for this
  day — `pub type Launch = Arc<dyn Fn(&Path) -> io::Result<LaunchOutcome> + Send + Sync>`
  (~line 87), taking the session staging directory; `start_daemon_with_dialog` (osascript)
  fills it on macOS; the IPC tests inject a stand-in that "plays the daemon against the
  same files". `AdminSession` (staging dir, heartbeat thread, `job_seq`, `daemon_started`)
  is host-agnostic and is **not modified** by this design — only the launcher closure and
  a cfg-gate move. `refusal_message` already classifies any stderr containing "cancel"
  into "Administrator access was declined — restricted areas stay unmeasured", which the
  Windows decline path reuses verbatim (§3.7.2).
- `apps/desktop/src-tauri/src/main.rs` is four lines — `#![cfg_attr(not(debug_assertions),
  windows_subsystem = "windows")]` then `diskguardian_app::run()` — so the pre-Tauri
  serve branch (§3.7.3) slots in before any Tauri initialisation, and release builds have
  no console to flash.
- `crates/admin/Cargo.toml`: bin-only, `[lints.rust] unsafe_code = "forbid"`. The app
  crate carries no such lint today, but core and admin both forbid — the one unavoidable
  Win32 call therefore gets its own windows-gated mini-crate (§3.7.2) rather than being
  the app crate's first `unsafe`, and admin's forbid is untouched.

**Root cause, stated once:** the release pipeline's build matrix enumerates two Apple
triples on one macOS runner and its verify/upload/collect steps hard-code macOS bundle
shapes. Multi-platform shipping needs the matrix to select runner and bundle set per
target, verification stated per-platform ("the payload contains, byte-identical, exactly
the binaries this platform ships"), and — new in Amendment 1 — a Windows elevation
mechanism that fills the existing launcher seam without shipping a second binary.

## 2. What "honest status" means per platform, after the rulings

- **macOS — unchanged.** Sidecar helper via `externalBin`, osascript prompt-per-session,
  everything as shipped in C23.
- **Windows — app binary only, self-elevation in this slice.** The NSIS payload contains
  `DiskGuardian.exe` and nothing else. When the user consents via UAC, the elevated
  process is that same `DiskGuardian.exe` re-executed with a serve flag; it runs the
  identical job/results protocol (from the same code, §3.7.1) and exits with the session.
  The trade the maintainer accepted, stated plainly: **on macOS the privileged process is
  a capability-minimal binary whose entire code is measure-and-write; on Windows it is
  the whole app binary, whose serving path is selected by an argv branch taken before any
  window, webview or command surface can exist.** The unreachable code is still in the
  image. That is a wider privileged image than macOS's, and the claim (§6) says so.
- **Linux — helper ships, elevation deferred.** The `.deb` and `.AppImage` carry
  `diskguardian-admin` beside the app, exactly like macOS, and the Measure button reports
  "not available on Linux yet" in words (the existing `#[cfg(not(target_os = "macos"))]`
  stub narrows to Linux — §3.7.5). Why not self-elevation here: **`pkexec` is not
  self-elevation.** It is a polkit-daemon-mediated prompt that needs a `.policy` file
  defining the action, its allowed callers and its retain-timeout — a new mechanism with
  its own security review, not a re-exec behind the same seam. Linux keeps the helper in
  the bundle so that day is a launcher swap plus one policy file, with the bundle shape
  already stable.
- **Everywhere:** the core loop never needs elevation (A5); restricted petals stay purple
  and counted when elevation is declined or unavailable (A3); deletion never runs
  elevated on any platform (the serving path initialises no command surface at all).

## 3. The mechanism — what the pipeline and the app do differently

### 3.1 The matrix selects runner and bundle set per target

`release.yml`'s `build` matrix becomes an `include` list; each entry carries the target
triple, the runner, and the bundle targets:

| target | runner | `--bundles` | ships in payload |
|---|---|---|---|
| `aarch64-apple-darwin` | `macos-latest` | `app,dmg` (unchanged) | app + admin |
| `x86_64-apple-darwin` | `macos-latest` | `app,dmg` (unchanged) | app + admin |
| `x86_64-pc-windows-msvc` | `windows-latest` | `nsis` | **app only** |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `deb,appimage` | app + admin |

`runs-on: ${{ matrix.os }}`; `fail-fast: false` stays. Windows and Linux build their
host triple — no cross-compilation anywhere.

### 3.2 Linux leg system dependencies (exact set)

Same webview set CI already proves, plus nothing speculative:

```
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

The AppImage step gets `APPIMAGE_EXTRACT_AND_RUN: 1` so the bundler's `linuxdeploy`
(itself an AppImage) never needs FUSE on the runner. If the first run still fails with a
FUSE error on `ubuntu-24.04`, the named remediation is `sudo apt-get install -y
libfuse2t64`; if that does not resolve it, the pre-declared fallback in §5 fires.

### 3.3 Verification: hash-based, name-agnostic, per-platform expected sets

> **Amended by Amendment 2 (below):** the build passes `--no-binary-patching` (deb/NSIS
> payload binaries must hash-match cargo output exactly), and the Linux AppImage arm is
> a per-section ELF-content check rather than a file hash, because Tauri/linuxdeploy
> rewrite both binaries there by construction. This section's text stands as originally
> written for macOS and for the deb's payload binaries.

The rule each leg's verify step enforces, stated once:

> The packaged payload contains, **byte-identical by SHA-256 to the cargo build products
> under `target/<triple>/release/`**, every binary in that platform's expected set — and,
> on Windows, **no binary outside it**.

| leg | expected set (hash-matched) | negative assertion |
|---|---|---|
| macOS (both) | `diskguardian-app`, `diskguardian-admin` | — |
| Windows | `diskguardian-app.exe` **only** | **no file named `diskguardian-admin*` (case-insensitive) anywhere in the extracted payload** — pins the ruling: a config-merge regression or a stale staged sidecar cannot quietly re-ship the helper |
| Linux (both formats) | `diskguardian-app`, `diskguardian-admin` | — |

Mechanically, per leg: compute reference hashes from the cargo output directory
(`sha256sum` exists in all three runners' bash shells), extract the packaged payload,
enumerate its files, require each reference to appear among the extracted hashes, and on
Windows require the admin name to appear nowhere. Hash equality proves presence,
correctness **and** architecture in one check (a binary under `target/<triple>/` is that
triple by construction) and does not care what the bundler renames the main binary to
inside each format — the one fact this lane could not pin down from macOS. Extraction
per format: macOS keeps the existing `lipo`/`test -x` assertions and adds the hashes;
Windows `7z x` the `*-setup.exe` (fallback: silent-install `setup.exe /S /D=<scratch>` —
stronger, it proves installability, just slower); Linux `dpkg-deb -x` and
`./DiskGuardian_*.AppImage --appimage-extract` (type-2 extraction, no FUSE needed).

Every Windows step gets `shell: bash` explicitly (runner default is pwsh; the existing
steps' bash idioms would fail in it confusingly). Each verify step prints every hash it
compared to the job log — those lines are the evidence stage B checks against.

### 3.4 Upload stays one-artifact-per-triple

`upload-artifact` keeps the name `diskguardian-<triple>` (the suffix source the release
lane already trusts) with per-leg globs: `nsis/*-setup.exe` (Windows),
`{deb/*.deb,appimage/*.AppImage}` (Linux), macOS unchanged. `if-no-files-found: error`
stays.

### 3.5 The release lane collects, renames by triple, and asserts the exact set

Unchanged from the original design: the `release` job (still `macos-latest` — it needs
`ditto`) extends its collect step to the new single-file artifacts, renaming each with
its triple derived from the artifact directory (the pattern that fixed the
one-arch-overwrites-the-other bug):

- `DiskGuardian-x86_64-pc-windows-msvc-setup.exe`
- `DiskGuardian-x86_64-unknown-linux-gnu.deb`
- `DiskGuardian-x86_64-unknown-linux-gnu.AppImage`

Plus two assertions: the **exact matrix-derived asset count** (7 with the full matrix:
2 dmg + 2 zip + 1 exe + 1 deb + 1 AppImage) and **no duplicate asset names**
(`ls out | sort | uniq -d` empty).

### 3.6 The release body tells each platform what it will actually see

Per-platform blocks, same register as the existing Gatekeeper text:

- **macOS (existing, kept):** unsigned → quarantine; right-click Open or the
  `xattr -dr com.apple.quarantine` command.
- **Windows:** unsigned → SmartScreen "Windows protected your PC"; *More info → Run
  anyway*. Then, plainly: **"Measuring restricted areas asks Windows for administrator
  rights (one UAC prompt per app session) — the app re-launchs itself elevated to
  measure, and the elevated copy only measures; it never deletes. Decline it and
  everything else works, with those areas shown as restricted. This behaviour is new in
  0.1.1."**
- **Linux:** `.deb` is unsigned (`sudo apt install ./DiskGuardian…deb` or `dpkg -i`);
  AppImage needs FUSE, which some distros omit — `--appimage-extract-and-run` is the
  no-FUSE route. **"Measuring restricted areas is not available on Linux yet; those
  areas stay shown as restricted."**

### 3.7 Windows self-elevation — the four new problems, solved

#### 3.7.1 `crates/admin` becomes lib + bin, so the serve loop has one implementation

The serving loop (`serve(dir, idle_secs, stale_secs)`), the one-shot mode, job
validation, and the status/results writers move from `main.rs` into a new
`crates/admin/src/lib.rs` — **unchanged code, moved**; `main.rs` becomes a thin wrapper
calling the lib, and the bin stays the macOS/Linux sidecar with the same name, same
protocol, same tests. The app crate takes the lib as a **Windows-only target dependency**
(`[target.'cfg(windows)'.dependencies]`), so macOS and Linux app binaries do not even
link a serve path. `JOB_VERSION`, the wire shapes and every admin test move with the
code; nothing in the protocol changes, which is the whole point: an elevated Windows
instance serving `job-<n>.json → results-<n>.jsonl + status-<n>.json` is
indistinguishable, at the file level, from the macOS helper.

#### 3.7.2 The launcher: `ShellExecuteW` with the `runas` verb, isolated in a mini-crate

The `Launch` seam gets a Windows filler that does what osascript does on macOS: raise the
OS consent, start the serving process, and return once the user has answered — **without
holding a child handle**. The primitive with exactly those semantics is
`ShellExecuteW(null, "runas", <current_exe>, "--dg-admin-serve <dir>", null, SW_HIDE)`:
it shows the UAC prompt, and on approval spawns the elevated process and returns
(whether it returns before or after the prompt is answered is handled below by not
trusting it). The maintainer's "shouldn't windows just use application elevation" maps
onto this directly.

- **Why not the `runas` crate:** its `status()` waits for the child to **exit** — the
  child is a session-long server, so the first `scan_restricted` would park on the launch
  await until the whole session ends. We need prompt-then-return, not prompt-then-wait.
  If, at build time, a vetted crate with those semantics is preferred over ~60 lines of
  our own, the seam accepts it; the requirement is the semantic, not the supplier.
- **Where the one `unsafe` lives:** a new windows-gated mini-crate (`crates/win-elevate`,
  ~60 lines, no tauri, no core — one public function `elevate_serve(exe, args) ->
  io::Result<ElevateOutcome>` wrapping the single `ShellExecuteW` call). `crates/admin`
  keeps `unsafe_code = "forbid"` untouched; the app crate takes the mini-crate as a
  windows-only target dependency and stays `unsafe`-free. The mini-crate's entire source
  is the review surface for the only `unsafe` this increment adds anywhere.
- **Declined UAC maps onto the existing outcome for free.** `runas` refusal
  (user clicked No / closed the prompt) surfaces as a failed `ShellExecuteW` and is
  returned as `LaunchOutcome { success: false, stderr: "the user cancelled the elevation
  prompt" }` — and `refusal_message` already renders any stderr containing "cancel" as
  *"Administrator access was declined — restricted areas stay unmeasured"*. Zero changes
  to the refusal path; the sentence the macOS user sees is the sentence the Windows user
  sees.
- **UAC is once per session because the flag already exists.** The launcher is only
  invoked when `!session.daemon_started`, and `daemon_started` flips only after
  `daemon.ready` appears — the one-prompt-per-session cadence is `AdminSession`'s
  property, not the launcher's. A second pass in the same app run reuses the serving
  instance and asks nothing.
- **An abandoned prompt cannot be dismissed, and the design says so.** macOS force-kills
  `osascript` at the pass deadline; **UAC prompts cannot be programmatically dismissed —
  that is Windows' design, not ours.** An unanswered prompt therefore parks that one
  pass until the user answers it or quits the app. This is bounded and honest: the
  session lock is held across the launch on purpose, so no second prompt can stack
  behind an unanswered one; answering resumes normally; declining returns the refusal
  message with the tree untouched. Stated limit, recorded in §6.
- **Ready-wait uses the outer budget, not `READY_WAIT`.** Because `ShellExecuteW`'s
  return timing relative to the prompt answer is not something this design trusts, the
  Windows launcher polls `daemon.ready` with the user-paced outer bound
  (`DEADLINE_SECS + OUTER_GRACE_SECS`), not the 30-second `READY_WAIT` that suffices
  after a blocking osascript. Approval without a ready instance still fails with the
  existing "the administrator helper did not start after approval".

#### 3.7.3 Second-instance behaviour: the argv branch runs before Tauri exists

`main.rs` becomes:

```rust
fn main() {
    // Before ANY tauri code: an elevated re-exec serves the admin protocol and exits.
    // No Builder, no window, no webview, no command surface can come into existence.
    #[cfg(windows)]
    match diskguardian_app::admin_serve_mode(std::env::args_os()) {
        ServeMode::Normal => {}
        ServeMode::Serve(dir) => std::process::exit(diskguardian_admin::serve_standalone(dir)),
        ServeMode::BadUsage => std::process::exit(2),
    }
    diskguardian_app::run();
}
```

- **Flag:** `--dg-admin-serve <dir>` (two args; `BadUsage` covers the flag without an
  operand — an elevated child with broken argv exits 2 rather than opening an elevated
  window). `admin_serve_mode` is a pure, unit-tested classifier in the app lib; the
  dispatch in `main` is four lines that reference no Tauri type.
- **No second window, structurally.** The serving instance never constructs a
  `tauri::Builder`; it cannot open a window because the code that opens windows is never
  reached. Release builds already set `windows_subsystem = "windows"` (no console flash;
  debug builds may flash a console — cosmetic, debug-only, stated).
- **Two user-launched copies of the app** behave as they do on every platform today:
  two windows, two independent sessions, and if both press Measure, two UAC prompts —
  correct, because they are two apps, and the same as macOS raising two osascript
  dialogs for two app copies. Out of scope to change.
- **Abuse of the flag by hand** grants nothing: invoking `DiskGuardian.exe
  --dg-admin-serve <dir>` elevated already requires the consent of someone holding admin
  rights, and that someone can run anything elevated. The branch reads job files only
  from a directory the invoker controls, walks only the paths in them read-only, writes
  only into that directory, and exits when the heartbeat there goes stale — the same
  same-writer trust and zero-delete posture as the macOS helper, stated in §2.

#### 3.7.4 Liveness: the existing heartbeat, uniformly — not a Windows job object

The heartbeat (`session.alive`, touched by the unelevated parent every 5 s, staleness
budget 30 s) plus the staging-directory-disappearance and idle exits are **kept as the
liveness mechanism on Windows too**, because the serve loop is now the same lib code
(§3.7.1) and the parent-side `AdminSession` is untouched.

Why not a job object (`JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`), which a spawned parent could
use to tie the elevated child's death to its own: it would require `ShellExecuteExW` with
a retained process handle (more `unsafe` surface than the mini-crate's one call), a
post-spawn assignment race, and handle-lifetime reasoning across the whole app — to buy
exactly one thing: killing the elevated instance **immediately** on parent force-kill
instead of within the existing 30-second staleness budget. That budget is already the
accepted, field-observed macOS behaviour (C23's record: helper alive at +3 s after a hard
kill, gone by +8 s), and the lingering instance is inert without a job file and writes
nowhere outside the staging directory. **Decision: uniform heartbeat; job object
deferred** with reopen trigger: a Windows user or security scanner reports the ≤30 s
elevated tail as a problem. Then it lands behind the same `Launch` seam as everything
else.

Force-kill answer, stated: kill the parent and the elevated instance measures nothing
further (no job file can reach it), notices the stale heartbeat within its 30 s budget,
and exits. It cannot outlive the session by more than that.

#### 3.7.5 Config and CI changes the rulings make necessary

1. **Bundle config moves wholly into per-platform files**, so no merge semantics can
   produce a mixed result (arrays in Tauri's platform merge may replace or append —
   carrying each list in exactly one place is correct under either rule):
   `tauri.macos.conf.json`: `targets ["app","dmg"]`, `externalBin
   ["binaries/diskguardian-admin"]`; `tauri.linux.conf.json`: `targets ["deb","appimage"]`,
   same `externalBin`; `tauri.windows.conf.json`: `targets ["nsis"]`, **no `externalBin`
   at all**. The base `tauri.conf.json` drops both keys. Local `bun run tauri:build`
   then works on every platform, and the Windows leg cannot validate a sidecar it must
   not ship.
2. **`stage:admin` gating:** release.yml stages on macOS/Linux only; ci.yml's stage step
   gains `if: runner.os != 'Windows'` (with `externalBin` absent on Windows there is
   nothing to validate).
3. **ci.yml runs the app-crate tests** (`cargo test -p diskguardian-app --tests`) on the
   ubuntu engine lane (webkit deps + staging already present; the mock runtime makes the
   IPC suite headless). Today CI only `cargo check`s the app crate — the injected-launch
   tests this slice adds would otherwise never run in CI anywhere.
4. **The non-macOS launcher stub narrows to Linux:** `#[cfg(not(target_os = "macos"))]`
   becomes `#[cfg(target_os = "linux")]` with the message "administrator measurement is
   not available on Linux yet" (Windows now has a real launcher).
5. **New tests riding the existing seams** (all CI-runnable, none needing elevation):
   - classifier: `admin_serve_mode` returns `Serve(dir)` for the flag pair, `Normal` for
     ordinary argv, `BadUsage` for the bare flag;
   - in-process serve: the lib's `serve_standalone` runs against a temp staging dir with
     a crafted job and produces the results/status files (the protocol tests that exist
     today, now pointed at the lib entry the Windows branch calls);
   - injected-launch IPC: a Windows-shaped decline returns the exact refusal sentence
     with `daemon_started` still false and the tree untouched; a successful injected
     launch plus a ready file flips `daemon_started`, and the injected launcher is
     **not called again** for a second pass in the same session (the once-per-session
     property, pinned);
   - a source-order guard in the style the repo has used before (C28's source-level
     markup guard): a test asserting `main`'s serve dispatch textually precedes any
     `Builder` reference — crude, and exactly the point: the elevated path must not be
     one refactor away from initialising Tauri.
   - Optional, pre-declared droppable if flaky: a harness-less Windows integration test
     that owns its `main`, re-execs **itself** through `win_elevate` in serve mode
     against a temp dir, and asserts the round-trip. On GitHub's already-elevated
     Windows runners no prompt fires, so this actually runs the real `ShellExecuteW`
     path headlessly; it proves the wrapper and the dispatch, **not** consent behaviour.

### 3.8 What explicitly does NOT move

- `crates/core` — untouched. No scan, model, view, delete or duplicates change.
- The serialized shape of `Tree`/`Node`/`ViewNode` across the Tauri IPC boundary, and
  the admin wire protocol (`JOB_VERSION = 1`) — unchanged. The elevated Windows instance
  is a different *host* for the same protocol, not a protocol change.
- The Tauri command surface — no command added, removed or re-typed. `AdminSession`,
  `scan_restricted`'s flow, the graft verification and `admin_authorised` are unchanged;
  only the cfg-gate on the launcher and the new Windows filler move.
- Frontend TypeScript (`apps/desktop/src/`) — untouched (Measure button stays ungated,
  per the ruling).
- The deletion guard, the core loop's privilege profile (A5), and the no-delete-call-
  sites property of the serving code — untouched. On Windows the *binary* containing
  deletion code runs elevated, but the branch that does so initialises no command
  surface, so no deletion code path is reachable elevated — stated in §2 and §6.

## 4. Precedent

- **The `Launch` seam** (`admin.rs` ~line 87) was built so "a future SMAppService
  launcher replaces one module and no protocol" (R3 design §10, ISA decision). The
  Windows self-elevation launcher is that prophecy arriving early: one module, same
  protocol, same `AdminSession`.
- **The human-test precedent is C23's AC4**, where a real consent dialog could not be
  automated ("automating that is not something this project will ever do, which is why
  the claim needed a human") and the claim waited for the principal's hands. The Windows
  UAC arms of this design follow the same rule (§7 Stage H).
- **The heartbeat's three exits** are field-proven on real hardware (C23 record) and
  unit-tested; reusing them on Windows extends that mechanism rather than inventing a
  parallel one.
- **The hash-verify + artifact-dir-suffix patterns** stand as in the original design
  (§3.3–§3.5), extending the pipeline's own "build asserts what ships; nothing rebuilds
  in the release lane" shape and the matrix-with-runner dimension `ci.yml` models.
- **New architecture, and why precedent does not fit it:** the `win-elevate` mini-crate
  and the lib+bin split of `crates/admin` have no in-tree precedent. The alternative —
  putting `ShellExecuteW` in the app crate, or duplicating the serve loop in the app —
  was rejected: the first puts the repo's only `unsafe` in its largest crate with no
  forbid lint, the second forks the single most security-audited loop in the codebase.

## 5. Minimal first slice — and the pre-declared fallbacks

**One slice attempts all four legs.** Windows/Linux are independent under
`fail-fast: false`; each leg's verify is self-contained; the release lane's expected
count derives from the matrix.

**Ship gate for the Windows asset includes Stage H (the hand test, §7).** The maintainer
put elevation in the slice; the repo's own rule ("No 'should work'") and the C23 AC4
precedent say a consent behaviour ships only after a human has seen it. If H cannot run
before the 0.1.1 tag, the macOS+Linux assets ship and the Windows asset waits for H on
the next tag — a pre-declared partial-ship path, not a failure. macOS and Linux assets
ship on CI evidence alone, exactly as v0.1.0 did.

**Pre-declared fallback:** if the Linux AppImage cannot be produced hash-verified within
the iteration cap (§7), that leg drops `appimage` and ships deb-only. Windows has no
fallback below NSIS (msi deliberately deferred) — a Windows tooling failure within the
cap kills the slice rather than degrading it.

**Explicitly deferred, each with its reopen trigger:**

| Deferral | Why | Reopen trigger |
|---|---|---|
| `msi` on Windows | NSIS is Tauri's default; msi adds the WiX download failure surface and doubles the verify set for zero new capability | Enterprise/GPO deployment request, or machine-wide install defaults |
| Linux elevation (`pkexec` + `.policy` file) | Polkit is daemon-mediated, not self-elevation; its policy file is a new security-reviewed mechanism. Helper already ships there, so it is a launcher swap + policy file | User demand, or the signing-identity work |
| Job-object liveness on Windows | ≤30 s inert tail after force-kill matches accepted macOS behaviour; a job object adds handle-lifetime `unsafe` for that one improvement (§3.7.4) | A user or scanner reports the elevated tail |
| Hiding vs erroring the Measure button on Linux | Untested UI change; an honest typed error is better than an unobserved branch | Bundled with the pkexec launcher day |
| arm64 Linux | x86_64 is the CI-proven story | A user on ARM Linux asks |
| macOS `universal2` single dmg | Two dmgs work today | User-confusion report |
| Signing / notarization / SmartScreen reputation | No identity exists (recorded R3 decision); warnings are stated honestly instead | Developer ID acquired |
| `stage-admin.ts` hard-coded `../../target` | Does not fire on CI runners; in the ISA log | The workflow ever sets `CARGO_TARGET_DIR` or builds in a worktree |
| Published `sha256sums.txt` beside assets | Stage-B already re-hashes downloads | First corrupted-download report |
| Per-bundle-type updater support (what `--no-binary-patching` forgoes) | Amendment 2: the flag makes C30's byte-identity sentence true; the app ships no updater, so the forgone capability is unused | An updater is ever added — at which point the flag, the verify steps and C30's wording must be revisited together |

## 6. Claim and anti-claim impact — exact ISA amendments

The maintainer lands these; the text is written to be pasted.

**1. Out of Scope (v0.1), remove the bullet:**

> - Windows and Linux builds. The code is written portable and cross-platform, but only macOS is tested and shipped at v0.1.

**2. New claim (written so its CI-provable and hand-test-only arms are visibly
different, and the hand-test arms are open until the record exists):**

> ### C30 — Every supported platform ships an installable, structurally verified artifact; Windows measures restricted space by self-elevation  `[ ]`
> A tagged release attaches installable artifacts for macOS arm64 + x86_64 (`.dmg` + `.app`
> `.zip`), Windows x86_64 (NSIS `.exe`), and Linux x86_64 (`.deb` + `.AppImage`) — every
> artifact named with its target triple; each verified before upload to contain,
> byte-identical by SHA-256, exactly the binaries that platform ships (macOS/Linux: app +
> helper; Windows: app only, with no `diskguardian-admin*` file anywhere in the payload);
> and the release body naming what each platform's OS will warn about (Gatekeeper,
> SmartScreen, unsigned deb / AppImage FUSE).
>
> On Windows, measuring restricted areas re-executes the app elevated (UAC `runas`), the
> elevated instance serves the unchanged job/results protocol and exits with the session,
> and — unlike macOS, whose privileged process is a measurement-only helper — the
> privileged process is the app binary itself, gated by a pre-Tauri argv branch that
> initialises no window, webview or command surface. That difference is stated, not
> hidden. A UAC prompt cannot be dismissed programmatically; an abandoned prompt parks
> that one pass until answered or the app exits, and no second prompt can stack.
>
> **Probe (CI arms, close on workflow evidence):** the release workflow's per-leg Verify
> steps — expected-set hash equality plus the Windows negative assertion, hashes printed
> in the job log — the release lane's exact-count and duplicate-name assertions, and the
> post-tag download re-hash; the injected-launch IPC tests (decline maps to the exact
> refusal sentence with the tree untouched; one launch per app session; the classifier),
> run headless in CI.
> **Probe (consent arms, open until the Stage H record lands):** one real UAC prompt per
> app session — not one per scan — seen and photographed by a human on real Windows;
> decline shows the refusal banner and changes nothing; the elevated instance grafts
> measured petals; it is gone within its 30-second heartbeat budget after the parent is
> force-killed. GitHub's Windows runners are already elevated, so the consent prompt
> never fires there; these arms cannot close on CI evidence and do not pretend to.
> **Falsifier:** any shipped platform artifact whose payload lacks a binary in its
> expected set (or, on Windows, contains one outside it) by hash; any asset named without
> its triple; a tag whose draft release carries fewer assets than the matrix produces; a
> release body omitting a platform's warning; a second UAC prompt for a second pass in
> one app session; an elevated instance still alive past its budgets after the parent dies.
>
> **Status:** opens `[ ]`. The CI arms close when the 0.1.1 tag is audited; the consent
> arms stay open until the Stage H hand-test record exists, even if the artifact has
> shipped in the meantime.

**3. C23 gains one status line (mechanism untouched):**

> **Platform note (2026-11-03):** the helper and its protocol ship unchanged in the macOS
> and Linux artifacts. Windows ships no helper: the app re-execs itself elevated (UAC)
> and the elevated instance serves the same protocol from the same code (`crates/admin`
> lib). On Linux the Measure button reports "not available on Linux yet" and restricted
> petals stay restricted (A3, A5 hold). Per-platform launchers for Linux are deferred
> with reopen triggers in the multi-platform release design.

**4. § Decisions gains two entries:**

> - **Windows elevation is self-elevation through the existing launcher seam** (2026-11-03,
>   maintainer ruling). The app re-execs itself with `runas`; the elevated instance takes a
>   pre-Tauri argv branch and serves the unchanged job/results protocol. Accepted trade,
>   stated in C30: the Windows privileged image is the whole app binary, not a
>   measurement-only helper; the argv branch that selects serving constructs no window,
>   webview or command surface, so no deletion code path is reachable elevated. The single
>   `ShellExecuteW` lives in a ~60-line windows-gated crate (`crates/win-elevate`) so
>   `crates/admin` keeps `unsafe_code = "forbid"` and the app crate gains no `unsafe`.
> - **Release artifacts are hash-verified against the binaries cargo built for the
>   triple, against per-platform expected sets** (2026-11-03). Presence-by-name is what a
>   bundler promises; content-identity is what a user installs. The hash check also
>   proves architecture for free and survives bundler renaming; the per-platform set plus
>   the Windows negative assertion make "what ships" a decision the config makes and the
>   pipeline enforces.

**Anti-claims: none move.** A1 — no delete path is reachable elevated on any platform
(the serving branch initialises no command surface; the helper keeps zero delete call
sites). A2/A6 — engine untouched. A3 — restricted petals behave identically everywhere,
including under decline. A4 — no scan changes. A5 — the core loop still never needs
elevation; decline leaves everything but the restricted petals working. No existing
claim becomes harder to falsify; C12 stays macOS-worded; C23's mechanism is untouched
(only its platform availability widens).

**README (user docs), same slice:** Install section gains the per-platform artifacts and
local-build notes; Status drops "Windows/Linux builds" from Not-yet; the Administrator
measurement section gains the Windows paragraph (UAC once per session, elevated copy
only measures, macOS+Windows only, Linux pending).

## 7. Measurable proof — pre-committed acceptance rule (amended)

Written before any CI run of the new pipeline and before the hand test. Deterministic
checks throughout; n = 1 build per leg per run suffices. The one numeric threshold is
derived from prior data: known-good v0.1.0 assets are 2.96–3.20 MB, so the plausibility
floor is **1,000,000 bytes** (3× margin under the smallest known-good; a 3 MB floor was
considered and rejected — it would fail the known-good 2.96 MB zip).

**Stage A — prove without publishing** (`workflow_dispatch` on the branch):

- **A1** All four legs complete; every per-leg Verify step exits 0, including the
  Windows negative assertion.
- **A2** Every Verify log prints, for its leg, the SHA-256 of each reference binary in
  that platform's expected set and of the same content found inside the produced
  package — equal — and (Windows) the enumeration showing no `diskguardian-admin*`.
- **A3** The injected-launch/classifier/serve-lib tests run green in CI (ubuntu lane;
  the windows-gated ones compile-and-run where applicable).

**RED controls (scratch branch, before A1–A3 are trusted):**

- **R-phantom** — each leg's verify step requires a third, nonexistent reference hash
  (`diskguardian-cli`); **every leg must fail**. Green means decorative verification.
- **R-strip** — macOS leg: `rm "$APP/Contents/MacOS/diskguardian-admin"` after build;
  the verify step **must fail** naming the helper. (On Windows/Linux the absence arm is
  exercised by R-phantom — their package formats cannot be tampered post-build without
  rebuilding.)
- **R-mutate (new)** — on every leg, overwrite a *reference* binary in
  `target/<triple>/release/` with different bytes after the build step; the verify step
  **must fail** on the hash mismatch. This is the content arm, and it works uniformly
  because references are read at verify time.
  **Tightened by Amendment 2 for the AppImage arm:** the mutation is two named sites —
  one byte flipped inside the reference's `.text`, one inside its `.rodata` — and both
  must fail the per-section check (a build-id survives post-link mutation, so flipping
  bytes is the only arm that proves the section hashes are load-bearing).
- **R-neg (Windows)** — scratch branch, deliberately stage a sidecar and add the old
  `externalBin` to the Windows config; the Windows verify **must fail** on the negative
  assertion (proves the ruling is enforced, not just preferred).

**Stage B — publish** (tag `v0.1.1`):

- **B1** Draft release carries exactly the matrix-derived 7 assets (2 dmg + 2
  triple-suffixed zip + 1 `…x86_64-pc-windows-msvc-setup.exe` + 1 `…x86_64-unknown-linux-gnu.deb`
  + 1 `…x86_64-unknown-linux-gnu.AppImage`).
- **B2** Every asset ≥ 1,000,000 bytes.
- **B3** `gh release download` of the draft; payload binaries extracted as in §3.3
  re-hash equal to the Stage-A log; single-file bundles compared byte-wise to the
  artifact copies after the triple rename.
- **B4** Collect step's duplicate-name assertion passes (in the job log).
- **B5** The draft release body contains, mechanically checked: the SmartScreen wording,
  the Windows UAC/self-elevation paragraph, the Linux unsigned-deb and AppImage-FUSE
  wording, and the "not available on Linux yet" sentence.

**Stage H — the Windows hand test (consent arms; the ship gate for the Windows asset).**
Runs on real Windows 10/11 with a standard unelevated interactive login and an admin
credential available for UAC. **Synthetic fixture, per the repo's privacy rule** — no
real user paths in any screenshot: before the test, create
`C:\dg-fixture\locked\<files>` and deny the current user with
`icacls C:\dg-fixture\locked /deny <user>:(OI)(CI)F`; the scan target is `C:\dg-fixture`,
so every path in the evidence is synthetic. The operator then:

1. **H1 — approve and graft.** Install the exact NSIS asset from the draft release (the
   one B3 hash-verified). Scan `C:\dg-fixture`. Expect the `locked` folder drawn purple
   as restricted. Press *Measure restricted areas…*. Expect **one** UAC prompt (naming
   DiskGuardian, publisher Unknown — the unsigned wart, same class as macOS's
   osascript-attributed dialog). Approve. Expect the restricted petals to resolve and
   graft in the same view.
2. **H2 — once per session.** In the same app run, scan `C:\dg-fixture` again and press
   Measure again. Expect **no second UAC prompt** and a served pass (the reused-session
   property).
3. **H3 — decline.** Quit, relaunch, scan, press Measure, **decline** the prompt.
   Expect the banner "Administrator access was declined — restricted areas stay
   unmeasured", the tree unchanged, no crash; press Measure once more and the prompt is
   offered again (a refusal does not spend the session's authorisation).
4. **H4 — liveness.** With a session serving (H1 state), force-kill DiskGuardian from
   Task Manager. Expect the elevated DiskGuardian process present immediately after and
   gone within 60 s (budget 30 s + observation margin), with nothing else changed.

**The record that closes the consent arms:** screenshots of (a) the purple restricted
petals pre-measurement, (b) the UAC prompt, (c) the grafted view post-measurement with
the admin report in the status line, (d) the declined banner from H3 — all showing only
synthetic `C:\dg-fixture` paths — plus written confirmation of H2's no-second-prompt,
H3's re-offer, and H4's timing observation (two Task Manager snapshots suffice). This is
the same class of record C23's AC4 set: a human, a real dialog, a written trace.

**SHIP iff** the RED controls fail as required, **A1–A3 hold**, and **B1–B5 hold** —
with the platform split pre-declared: macOS and Linux assets **ship on that evidence**;
the **Windows asset ships only if H1–H4's record also exists** (if H cannot run before
the tag, macOS+Linux ship and Windows follows on the next tag after H — pre-declared
partial ship, §5).

**KILL / degrade:** any platform failing hash-verification within **5 CI iterations**
fires its fallback (Linux → deb-only; Windows → slice killed, matrix reverts to
macOS-only pending revision). Both failing kills the increment; C30 does not open. A
hand test that contradicts an H arm (a second prompt appears; decline crashes or
corrupts state; the elevated instance outlives its budget walking or writing) **kills
the Windows elevation mechanism for that tag** — the artifact may still ship with the
Measure button erroring honestly? **No** — pre-committed: an H failure removes the
Windows asset from that tag entirely; an elevation mechanism that misbehaved under real
consent does not ship in any form that release. Fix, re-run H, next tag.

**UNDERPOWERED (decides nothing):** dispatch cannot run on the branch, draft assets
cannot be listed/downloaded, or no Windows machine is available for H — infrastructure
or logistics, not product. No verdict recorded; the attempt does not count against the
iteration cap. (H-unavailability routes to the partial-ship path above rather than to a
kill.)

**Merge-gate evidence shape:** PR rollup carries the Stage-A run URL, the four RED run
URLs, the Stage-B asset listing (names + sizes) and body check, and — before the
Windows asset ships — the H record. Complete rollup, terminal states, per
`scripts/merge-verified.sh`.

## 8. Top risks, ranked (1 = most likely to burn CI rounds or reviews)

1. **Windows elevation semantics** (`ShellExecuteW` return timing vs the prompt; the
   wrapper's argv quoting; debug-build console flash). *Mitigation:* ready-wait uses the
   outer budget, not `READY_WAIT` (§3.7.2); the mini-crate is the only `unsafe` and is
   ~60 reviewable lines; the optional harness-less re-exec smoke runs the real path on
   the elevated CI runner. **Falsifier:** H fails on prompt/launch behaviour — the
   Windows asset does not ship that tag (pre-committed).
2. **Linux AppImage tooling on `ubuntu-24.04`** (linuxdeploy is an AppImage; FUSE may be
   required). *Mitigation:* `APPIMAGE_EXTRACT_AND_RUN=1`; named remediation package
   (`libfuse2t64`); pre-declared deb-only fallback. **Falsifier:** AppImage fails the
   5-iteration cap even with the fallback — fallback ships, C30 amended.
3. **ACL inheritance on the staging directory** (elevated writer, unelevated reader):
   the user-owned temp dir's inherited ACEs should let the parent read the results; if a
   hardened machine breaks inheritance, the graft reads nothing. *Mitigation:* H1's graft
   step is exactly this check on a real machine; the failure mode is loud (results never
   appear) and safe (nothing corrupts). **Falsifier:** H1 grafts nothing despite an
   approved session — the elevated writer must set explicit ACLs (its own small
   increment) before the Windows asset ships.
4. **Privileged-image honesty** (the whole app binary runs elevated on Windows). Not a
   defect — a stated trade the maintainer ruled for — but the review lanes should attack
   the argv branch: is anything initialised before it? Can the webview be reached?
   *Mitigation:* the source-order guard test (§3.7.5) and the no-command-surface
   property stated in C30. **Falsifier:** any path from the serve branch to a Tauri
   type.
5. **Icon set** (NSIS wants `.ico`; deb wants sized PNGs). *Mitigation:* regenerate the
   full set with `bunx tauri icon` in the same PR. **Falsifier:** bundler rejects the
   generated set — fix the source image; cosmetic.
6. **Windows step shells** (pwsh default vs bash idioms). *Mitigation:* `shell: bash` on
   every Windows step. Immediate, visible, one-line fix.
7. **7z cannot extract the NSIS payload** (handler drift). *Mitigation:* silent-install
   fallback (`/S /D=`), which is stronger evidence anyway.
8. **Asset-name collisions** in the release lane (the historical bug class).
   *Mitigation:* matrix-derived exact count + duplicate-name guard (B4).
9. **`crates/admin` lib+bin churn** — tests move with the code; a mechanical refactor
   that changes the protocol would be caught by the moved tests themselves. Low.

## 9. BUILD — recommended, amended

The premise survived verification; the rulings removed the one piece the original design
treated as fixed (helper-everywhere) and replaced it with a mechanism the codebase was
visibly prepared for (`Launch` seam, launcher-agnostic protocol, host-agnostic
`AdminSession`, four-line `main`). The serve loop stays single-sourced; the only new
`unsafe` is one Win32 call in a cage; the consent behaviour gets the C23-AC4 treatment
(human, real dialog, written record) instead of a CI fiction. Verification is
per-platform expected sets with a negative assertion that makes the ruling itself
machine-enforced. **BUILD**, one slice, all four legs, Stage H as the Windows ship gate,
with the §5 fallbacks and §7 acceptance rule committed with this document before any
workflow run or hand test.

## 10. Open decisions for the maintainer

1. **Who runs Stage H and when** — it needs a real Windows machine and a human; it gates
   the Windows asset for 0.1.1 (partial-ship path if unavailable).
2. **The optional re-exec smoke test** (§3.7.5, droppable-if-flaky) — keep or cut; it is
   the only CI exercise of the real `ShellExecuteW` wrapper.
3. **`win-elevate` as a crate vs a vetted third-party wrapper** — the design prefers the
   mini-crate for review surface; if you accept a dependency instead, the semantic
   requirement is prompt-then-return (not wait-for-exit), stated in §3.7.2.
4. **Next tag** confirmed as **0.1.1** (asset names and the release body embed it).
5. **Roadmap placement of C30** — a claim, not a DaisyDisk-parity row; number it only if
   you want it on the roadmap, outside parity ordering.

---

# Amendment 2 — binary patching is Tauri's, not a defect; the flag, not the exception

Date: 2026-11-03, after Stage A attempts 1–3 (runs 41632923688, 41635246338, 41637620161).
Maintainer ruling; builder diagnosis accepted as stated.

## What the runs proved

- The deb's `diskguardian-admin` hash-matches cargo output; the deb's
  `diskguardian-app` is the same size and BuildID with exactly three differing bytes,
  inside `__TAURI_BUNDLE_TYPE_VAR_UNK` → `__TAURI_BUNDLE_TYPE_VAR_DEB`. The mechanism is
  tauri's own (`tauri-utils-2.10.2/src/platform.rs:357`: "modified by binary patching
  during build"; macOS returns `App` from `cfg!` unpatched, which is why the macOS legs'
  byte-identity checks pass as designed). The original patchelf hypothesis is refuted
  for the deb: neither reference nor payload carries any rpath entry.
- The NSIS payload's app binary differs the same way (`_NSS`).
- The AppImage rewrites BOTH binaries (RUNPATH `$ORIGIN/../lib` injected, sizes grown):
  byte-identity is unattainable there by construction, for the sidecar too.

## The ruling

1. **`--no-binary-patching` on the shared build step** (deb, NSIS; a documented no-op
   for macOS-native bundles). A bounded 3-byte exception would make C30's sentence
   false by construction and license deviation from it; the flag makes the sentence
   TRUE. The flag's only documented cost is per-bundle-type updater support, and this
   app ships no updater (no updater plugin, no `createUpdaterArtifacts`, no source
   reads a bundle-type token). Attempt 4 must prove it: with the flag on, the deb and
   NSIS main binaries hash-match cargo output exactly, alongside the sidecar that
   already does. If the flag does not eliminate the difference, the bounded exception
   returns as the fallback — not before.
2. **The deb arm is unchanged**: plain SHA-256 equality, now expected to pass.
3. **The AppImage arm is a section-wise ELF-content check, for BOTH binaries** (the
   sidecar is rewritten there too). Per binary: ELF machine equal (and asserted to be
   the leg's architecture, since hash equality no longer carries it there); GNU
   build-id equal; program interpreter equal; `DT_NEEDED` list equal; RUNPATH exactly
   `$ORIGIN/../lib`; section-NAME set equal; and every section's content hash equal
   **except the bundler-owned `.dynstr` and `.dynamic`**. NOBITS sections (`.bss`,
   `.tbss`) have no file bytes and are skipped as a mechanical fact, not a policy
   exception. Every compared section hash is printed to the job log. No size bound
   anywhere — a growth bound is a looser check wearing a stricter coat and cannot fail
   R-mutate.
   - Why build-id alone is not enough: a build-id survives all post-link mutation
     (attempt 3's deb and AppImage binaries share `06c35623` while differing from the
     reference and from each other), so it is necessary, not sufficient. Why not
     `.text`-only: the bundle-type token lives in `.rodata`. The full-section compare
     minus the two named sections is the strongest check that can still pass.
4. **R-mutate tightens for the AppImage arm** to two named mutations: one byte flipped
   inside the reference's `.text`, one inside its `.rodata`. Both must fail the step.
   **Stated residual blind spot:** bytes outside any section (gaps between sections in
   the file image) are undetectable by any content check we run.
5. **Asset-level SHA-256s for the deb and the AppImage are printed in the Linux leg's
   log**: Stage B runs on macOS and cannot `dpkg-deb -x` or run linuxdeploy tooling —
   the printed asset hashes are what its download re-hash compares against.
6. **Iteration accounting** (maintainer): attempts 1 and 2 spent before the ruling, the
   post-diagnosis dispatch counts as the third; the flag-proving run is the fourth; cap
   stays five.
7. Attempt-2's instrumentation (per-file hashes, rpath dumps, byte-context `cmp`)
   **stays in the file** — it settled this amendment and it is the evidence Stage B
   reads.

Governance findings folded in the same window (runs crossed mid-flight): C30's evidence
lines now lead with the exact `**Probe:**` prefix the claim checker accepts, and the
triple-naming claim names the single-file artifacts as the triple-named set with the
two dmg names as non-colliding bundler names — in ISA and the README. The 1,000,000-byte
per-asset floor (§7 B2) moves from prose to an executable assertion in the release
lane's collect step, at the pre-committed value.
