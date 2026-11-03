# Multi-platform release artifacts — design

Date: 2026-11-03 · Author: increment-designer · Status: proposed (not implemented)

**High-risk areas, named in the first paragraph as required.** This increment touches
**none of the three engine high-risk areas** — physical-size accounting, restricted-space
reporting and guarded deletion are unchanged in behavior on every platform. But it does
touch the **distributed form of the restricted-space story**: the `diskguardian-admin`
helper ships inside every new artifact, and the elevated-measurement *launcher* stays
macOS-only (`apps/desktop/src-tauri/src/admin.rs`, `#[cfg(not(target_os = "macos"))] fn
start_daemon_with_dialog`). The design's honesty obligation is therefore about
**packaging and labelling**, not arithmetic: no artifact may silently lack the helper, no
release body may imply a platform can do what it cannot, and the degraded Windows/Linux
state (button reports "administrator measurement is macOS-only for now") must be stated,
not discovered.

---

## 1. Premise verified — the symptom is real and its cause is in one job

Everything below was read this session, not assumed.

- `.github/workflows/release.yml`, `build` job: `runs-on: macos-latest` with a matrix of
  exactly `aarch64-apple-darwin` and `x86_64-apple-darwin`; build step
  `bunx tauri build --target <triple> --bundles app,dmg`; verify step shells `lipo` and
  `test -x` against `Contents/MacOS/{diskguardian-app,diskguardian-admin}`; upload takes
  `bundle/dmg/*.dmg` and `bundle/macos/*.app`. There is no Windows or Linux leg. That is
  the whole defect: the pipeline can only ever attach macOS assets.
- Published v0.1.0 (`gh release view`, this session): four assets, all macOS —
  `DiskGuardian-aarch64-apple-darwin.zip` (2,955,743 B), `DiskGuardian-x86_64-apple-darwin.zip`
  (3,149,897 B), `DiskGuardian_0.1.0_aarch64.dmg` (3,009,779 B), `DiskGuardian_0.1.0_x64.dmg`
  (3,200,566 B). The reported symptom matches the workflow exactly.
- `apps/desktop/src-tauri/tauri.conf.json` (line 33): `"targets": ["app", "dmg"]` — both are
  invalid bundle targets on Windows (`nsis`/`msi`) and `dmg` is invalid on Linux, so a local
  `bun run tauri:build` on those platforms would fail even though the workflow will override
  with `--bundles`. The config has to move too, or the README's build-from-source story is a
  lie outside macOS.
- `.github/workflows/ci.yml`: the `engine` job already runs `cargo test -p diskguardian-core`
  on `macos-latest`, `ubuntu-latest`, `windows-latest`, installs the Linux webview deps
  (`libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf`), and
  `cargo check -p diskguardian-app` with `-D warnings` on all three. **Unconditional
  compilation of the app crate on Windows and Linux is proven in CI**; only bundling is new.
- `apps/desktop/scripts/stage-admin.ts`: already triple-portable. It parses `--target`,
  resolves the host via `rustc -vV`, and — decisive for Windows — appends `.exe` after the
  triple ("Windows cargo emits diskguardian-admin.exe, and tauri's externalBin looks for the
  target-suffixed name WITH the extension intact"). It stages
  `src-tauri/binaries/diskguardian-admin-<triple>[.exe]`. **No change needed** for
  `x86_64-pc-windows-msvc` or `x86_64-unknown-linux-gnu` staging. Its known trap — copying
  from a hard-coded `../../target` — does not fire on CI runners (no `CARGO_TARGET_DIR`,
  fresh checkout), and is already logged in the ISA (2026-11-02). Out of scope here.
- `crates/admin/src/main.rs`: portable Rust. The only platform gate is `#[cfg(unix)]` around
  `world_readable` (a chmod), with a `#[cfg(not(unix))]` no-op. The helper knows nothing
  about `osascript`; elevation lives in the app's launcher. It links `diskguardian-core` and
  `serde`/`serde_json` only, so it cross-builds anywhere the engine does.
- `apps/desktop/src-tauri/src/admin.rs`: the non-macOS launcher stub already returns
  `io::ErrorKind::Unsupported` — "administrator measurement is macOS-only for now". The
  job-file/results-file protocol and the `AdminScanReport` IPC shape are platform-neutral.
  **Design finding (defect, folded into this slice):** `fn resolve_helper()` does
  `current_exe().parent().join(HELPER_NAME)` where `HELPER_NAME = "diskguardian-admin"`.
  Tauri's bundler renames the sidecar to `diskguardian-admin.exe` in a Windows install, and
  `Path::exists()` on the extension-less name is false there — so on Windows the user would
  see *"could not find the diskguardian-admin helper"* when the truth is "it shipped; the
  launcher is macOS-only". Wrong-honesty class, three-line fix (§3.7), and it is the reason
  the verify step must confirm the helper by **hash**, not by name.

**Root cause of the symptom, stated once:** the release pipeline's build matrix enumerates
two Apple triples on one macOS runner and its verify/upload/collect steps hard-code macOS
bundle shapes. Multi-platform shipping needs the matrix to select the runner per target,
the bundle set per platform, and the verification to be stated in a platform-neutral way
("the packaged payload contains, byte-identical, both binaries cargo built for this
triple") instead of the macOS-specific way it is stated today (`lipo`, `.app` paths).

## 2. What "honest status on Windows/Linux" means, and what drives it

The helper's *measurement* engine is portable; only its *elevation* is macOS-specific
(`do shell script … with administrator privileges`). Therefore:

- **Ship the helper in every artifact.** It is small, portable, has no delete call sites,
  and the wire protocol is identical — a future `pkexec` (Linux) or UAC (Windows) launcher
  slots into the existing `Launch` seam (`admin.rs`, `pub type Launch`) without a protocol
  change. Dropping it from Windows/Linux bundles would fork the bundle shape and make that
  day harder.
- **Do not ship an elevated-measurement story on Windows/Linux in this slice.** No `pkexec`,
  no `runas`. A scan on those platforms behaves exactly like a refused dialog on macOS:
  restricted petals stay purple and counted (A3 holds), the core loop never needs elevation
  (A5 holds), and pressing "Measure restricted areas…" returns the launcher stub's own
  words. That is a degraded, honest state — the release body says so in plain text.
- The button is **not** gated per-platform in this slice. Gating it would be a frontend
  change with no way to observe it here (no Windows/Linux desktop available to this lane),
  and an honest typed error is better than an untested UI branch. Deferred with a reopen
  trigger (§4).

## 3. The mechanism — what the pipeline does differently

### 3.1 The matrix selects runner and bundle set per target

`release.yml`'s `build` matrix becomes an `include` list; each entry carries the target
triple, the runner, and the bundle targets:

| target | runner | `--bundles` |
|---|---|---|
| `aarch64-apple-darwin` | `macos-latest` | `app,dmg` (unchanged) |
| `x86_64-apple-darwin` | `macos-latest` | `app,dmg` (unchanged) |
| `x86_64-pc-windows-msvc` | `windows-latest` | `nsis` |
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | `deb,appimage` |

`runs-on: ${{ matrix.os }}`; `fail-fast: false` stays (a Linux tooling failure must not
mask a Windows ship, and vice versa). Windows and Linux build their **host** triple — no
cross-compilation anywhere, which is what keeps `stage:admin --target <triple>` on the
host-binary fast path.

### 3.2 Linux leg system dependencies (exact set)

Same webview set CI already proves, plus nothing speculative:

```
sudo apt-get install -y libwebkit2gtk-4.1-dev libappindicator3-dev librsvg2-dev patchelf
```

The AppImage step gets `APPIMAGE_EXTRACT_AND_RUN: 1` in its env so the bundler's
`linuxdeploy` (itself an AppImage) never needs FUSE on the runner. If the first run still
fails with a FUSE error on `ubuntu-24.04`, the named remediation is
`sudo apt-get install -y libfuse2t64` — and if that does not resolve it, the pre-declared
fallback in §5 fires. This is ranked risk #1 precisely because it cannot be verified from
a macOS design lane.

### 3.3 Verification is hash-based and name-agnostic (the heart of the slice)

Today's verify step asks macOS questions (`lipo -archs`, `Contents/MacOS`). The new step
asks one platform-neutral question on every leg:

> Does the packaged payload contain, **byte-identical by SHA-256**, both
> `target/<triple>/release/diskguardian-app[.exe]` and
> `target/<triple>/release/diskguardian-admin[.exe]`?

Mechanically, per leg: compute the two reference hashes from the cargo output directory
for that triple (`sha256sum`; present in the bash shells of all three runners), extract
the packaged payload, enumerate its files, and require **exactly** the two references to
appear among the extracted files' hashes. Fail with the missing binary's name otherwise.
Hash equality proves presence *and* correctness *and* architecture in one check — the
binary under `target/<triple>/` is that triple by construction, so no `dumpbin` or
`file`-parsing is needed. It also sidesteps the one fact this lane could not pin down:
what Tauri names the main binary inside each package format (`DiskGuardian.exe` vs
`diskguardian-app.exe`, `/usr/bin/DiskGuardian` vs `/usr/bin/diskguardian-app`). The check
does not care.

Payload extraction per platform:

- **macOS (kept, plus hashes):** the existing `lipo`/`test -x` assertions stay verbatim;
  add hash comparison of `Contents/MacOS/{diskguardian-app,diskguardian-admin}` against
  the two references.
- **Windows:** `7z x` the produced `*-setup.exe` into a scratch dir (windows runners ship
  7-Zip; NSIS payloads are 7z-readable), enumerate, hash-match. Fallback if 7z cannot
  read the payload: silent-install it (`setup.exe /S /D=<scratch>` — Tauri's NSIS default
  install mode is per-user, no elevation on a throwaway runner) and hash the installed
  files. That fallback is *stronger* (it proves installability), it is just slower.
- **Linux:** `dpkg-deb -x *.deb pkg/` and `./DiskGuardian_*.AppImage --appimage-extract`
  (type-2 runtime extraction; no FUSE needed to extract), enumerate both, hash-match
  against the same two references.

Every Windows step gets `shell: bash` explicitly — the windows runner default is pwsh,
and the existing steps' bash idioms (`set -euo pipefail`, `$(ls -d …)`) would fail in it
in confusing ways. Git bash ships `sha256sum`, so all four legs run near-identical verify
scripts.

The step also prints every hash it compared to the job log — those lines are the evidence
the acceptance rule's stage B checks against (§7).

### 3.4 Upload stays one-artifact-per-triple

`upload-artifact` keeps the name `diskguardian-<triple>` (the suffix source the release
lane already trusts — that trust was earned by the one-arch-overwrites-the-other bug) and
gains per-leg path globs: `target/<triple>/release/bundle/nsis/*-setup.exe` (Windows),
`target/<triple>/release/bundle/{deb/*.deb,appimage/*.AppImage}` (Linux). `if-no-files-found:
error` stays.

### 3.5 The release lane collects, renames by triple, and asserts the exact set

The `release` job (still `macos-latest` — it needs `ditto` for the `.app` zips) extends
its collect step: copy `*.exe`, `*.deb`, `*.AppImage` from the artifact tree, renaming
each to carry its triple, derived from the artifact directory exactly as the `.app` loop
already does:

- `DiskGuardian-x86_64-pc-windows-msvc-setup.exe`
- `DiskGuardian-x86_64-unknown-linux-gnu.deb`
- `DiskGuardian-x86_64-unknown-linux-gnu.AppImage`

Two new assertions replace today's `[ "$count" -ge 2 ]`:

1. **Exact expected count, derived from the same matrix** (7 with the full matrix: 2 dmg +
   2 zip + 1 exe + 1 deb + 1 AppImage). The count is not hard-coded; it is computed from
   the matrix definition the lane imports, so the pre-declared fallback in §5 (drop
   AppImage) changes the expectation honestly rather than breaking it.
2. **No duplicate asset names**: `ls out | sort | uniq -d` must be empty — the mechanical
   guard for the class of bug that once shipped one arch as both.

### 3.6 The release body tells each platform what it will actually see

The notes gain a per-platform block, in the same register as the existing Gatekeeper text:

- **macOS (existing, kept):** unsigned → quarantine, right-click Open or the
  `xattr -dr com.apple.quarantine` command.
- **Windows:** unsigned → SmartScreen "Windows protected your PC" on first run;
  *More info → Run anyway* is the route. Plus one honest sentence: **"Measuring
  restricted areas with administrator rights is macOS-only in this version; on Windows
  those areas stay shown as restricted."**
- **Linux:** the `.deb` is unsigned (install with `sudo apt install ./DiskGuardian…deb`
  or `dpkg -i`); the AppImage needs FUSE, which some distros omit —
  `./DiskGuardian…AppImage --appimage-extract-and-run` is the no-FUSE route. Same
  macOS-only measurement sentence.

### 3.7 Supporting changes that the above makes necessary

1. **`resolve_helper()` `.exe` fix** (`apps/desktop/src-tauri/src/admin.rs`): on Windows,
   look for `diskguardian-admin.exe` beside the app binary (a `cfg!(windows)` name
   selection, ~3 lines), so the honest stub message is the one a Windows user sees.
   Compile-proven by CI's existing `cargo check -p diskguardian-app` on Windows;
   **runtime-unobserved** (no Windows desktop in this lane) and stated as such.
2. **Per-platform bundle targets** — new `apps/desktop/src-tauri/tauri.windows.conf.json`
   (`"bundle": {"targets": ["nsis"]}`) and `tauri.linux.conf.json`
   (`"targets": ["deb", "appimage"]`), so a local `bun run tauri:build` outside macOS
   works and the workflow's `--bundles` flag agrees with config rather than silently
   overriding it. The macOS config is untouched.
3. **Full icon set** — regenerate `src-tauri/icons/` with `bunx tauri icon` from the
   existing square source PNG. Today's `icon: ["icons/icon.png"]` is sufficient for
   `.app`/`.dmg` but NSIS wants an `.ico` and `.deb` wants sized PNGs; rather than bet on
   bundler auto-conversion, ship the generated set and list it in `tauri.conf.json`.
   Regenerated from the same source, the macOS icon appearance does not change.

### 3.8 What explicitly does NOT move

- `crates/core` — untouched. No scan, model, view, delete, or duplicates change.
- The serialized shape of `Tree`/`Node`/`ViewNode` across the Tauri IPC boundary —
  unchanged. The admin wire protocol (`ResultLine`, `JOB_VERSION = 1`) — unchanged; the
  helper binary is *byte-different per platform* but speaks the same schema, which is
  precisely why it can ship in every artifact without a version bump.
- The Tauri command surface (`apps/desktop/src-tauri/src/`) — no command added, removed,
  or re-typed. `resolve_helper` is private plumbing behind an unchanged command.
- Frontend TypeScript (`apps/desktop/src/`) — untouched.
- The deletion guard, the core loop's privilege profile (A5), and the helper's
  no-delete-call-sites property — untouched.

## 4. Precedent

This extends two proven in-tree mechanisms rather than inventing one:

1. **The release pipeline's own two-lane, verify-before-upload shape** (`release.yml`
   header comment: "build … asserts they contain what they must … Nothing here rebuilds,
   so what ships is what was verified"). The hash-verify step generalizes the existing
   macOS "Verify the bundle contains what it must" step from arch-inspection (`lipo`) to
   content-identity, applied to more package formats.
2. **The artifact-dir-derived triple suffix** in the release lane's collect step — the
   exact pattern introduced to fix the silent one-arch-overwrites-the-other asset bug.
   Windows/Linux renames ride the same derivation, and the new duplicate-name assertion
   mechanically pins the bug class shut.

The matrix-with-runner dimension is the same shape `ci.yml` already models (`os:` matrix
over three runners). Nothing here is new architecture; the one genuinely new tool
interaction is the payload extraction per format (§3.3), and each has a named fallback.

## 5. Minimal first slice — and the pre-declared fallback

**One slice attempts all four legs.** Windows and Linux are independent under
`fail-fast: false`, each leg's verify is self-contained, and the acceptance rule derives
its expected set from the matrix — so a partial landing is representable by editing the
matrix, not by weakening the rule.

**Pre-declared fallback (decided now, before any CI data):** if the Linux leg's AppImage
cannot be produced hash-verified within the iteration cap (§7), the fallback removes
`appimage` from that leg's `--bundles` and ships **deb only** — the deb's verification
chain (`dpkg-deb`) has no third-party tooling dependency. The fallback is *not* "drop
Linux"; it is "drop the one bundle format whose tooling failed". Windows has no
equivalent fallback below NSIS (msi is deliberately deferred, §6), so a Windows failure
within the cap kills the slice rather than degrading it.

**Explicitly deferred, each with its reopen trigger:**

| Deferral | Why now | Reopen trigger |
|---|---|---|
| `msi` on Windows | NSIS is Tauri's default; msi adds the WiX download failure surface and doubles the verify set for zero new capability; SmartScreen treats both identically | An enterprise/GPO deployment request, or a need for machine-wide install defaults |
| Windows/Linux elevation launchers (`pkexec`, UAC `runas`) + per-platform gating of the Measure button | New privileged code paths on platforms this lane cannot run; an honest typed error beats an untested UI branch | User demand, or the signing-identity work that R3's design already anticipates — they slot into the existing `Launch` seam |
| arm64 Linux (`aarch64-unknown-linux-gnu`) | x86_64 is the CI-proven runner story; ARM Linux needs a cross or ARM runner decision | A user on ARM Linux asks |
| macOS `universal2` single dmg | Two dmgs work today; unification is cosmetic | A user-confusion report about picking a dmg |
| Signing / notarization / SmartScreen reputation | No identity exists (recorded R3 decision); warnings are stated honestly instead | Developer ID acquired |
| `stage-admin.ts` hard-coded `../../target` | Does not fire on CI runners; already in the ISA log | The release workflow ever sets `CARGO_TARGET_DIR` or builds in a worktree |
| A published `sha256sums.txt` beside assets | Stage-B already hash-checks downloads; a checksums file is user-facing redundancy | First corrupted-download report |

## 6. Claim and anti-claim impact — exact ISA amendments

The maintainer lands these; the text is written to be pasted.

**1. Out of Scope (v0.1), remove the bullet:**

> - Windows and Linux builds. The code is written portable and cross-platform, but only macOS is tested and shipped at v0.1.

*(Replaced by C30 landing; nothing else in that list moves.)*

**2. New claim:**

> ### C30 — Every supported platform ships an installable, structurally verified artifact  `[ ]`
> A tagged release attaches installable artifacts for macOS arm64 + x86_64 (`.dmg` + `.app`
> `.zip`), Windows x86_64 (NSIS `.exe`), and Linux x86_64 (`.deb` + `.AppImage`) — every
> artifact named with its target triple, every artifact verified before upload to contain,
> byte-identical by SHA-256, both `diskguardian-app` and `diskguardian-admin` as built for
> that triple, and the release body naming what each platform's OS will warn about
> (Gatekeeper, SmartScreen, unsigned deb / AppImage FUSE). Administrator measurement stays
> macOS-only and is stated as such; the helper ships everywhere and the job/results
> protocol is unchanged.
> **Probe:** the release workflow's per-leg "Verify" steps (hash equality against
> `target/<triple>/release/`, printed in the job log) plus the release lane's exact-count
> and duplicate-name assertions; audited after the tag by downloading the draft release's
> assets and re-hashing them against the same log.
> **Falsifier:** any shipped platform artifact whose payload lacks either binary by hash;
> any asset named without its triple; a tag whose draft release carries fewer assets than
> the build matrix produces; a release body that omits a platform's warning.

*(The claim-check gate requires the `**Probe:**` line — included. It is ticked only when a
tag has actually been run and audited, which is why it opens `[ ]`.)*

**3. C23 gains one status line (mechanism untouched):**

> **Platform note (2026-11-03):** the helper and its protocol ship in the Windows and
> Linux artifacts unchanged, but the elevated launcher is macOS-only; on those platforms
> the Measure button reports that in words and restricted petals stay restricted (A3, A5
> hold). Per-platform launchers are deferred with a reopen trigger in the multi-platform
> release design.

**4. § Decisions gains one entry:**

> - **Release artifacts are hash-verified against the binaries cargo built for the
>   triple** (2026-11-03). Presence-by-name is what a bundler promises; content-identity
>   is what a user installs. The hash check also proves architecture for free (a binary
>   under `target/<triple>/` is that triple) and survives whatever the bundler renames
>   the main binary to inside each package format.

**Anti-claims: none move.** A1 (deletion) — no delete path touched; the helper keeps zero
delete call sites. A2/A6 (sizes, arcs) — engine untouched. A3 — restricted petals behave
identically on the new platforms, including under the declined measurement. A4 — no scan
changes. A5 — the core loop still never needs elevation; the new platforms ship *without*
any elevation path at all. Nothing in this increment makes any C-claim harder to falsify;
C12 remains macOS-worded by design ("`bun run tauri:build` produces `DiskGuardian.app`")
and C30 is the cross-platform sibling, not a rewrite of it.

**README (user docs) must move in the same slice:** the Install section gains the
per-platform artifact names and the local-build note (per-platform bundle targets), and
the Status section's "Not yet: … Windows/Linux builds" line is deleted. The macOS-only
"Administrator measurement" section gains one sentence naming the platforms where the
button reports macOS-only.

## 7. Measurable proof — pre-committed acceptance rule

Written before any CI run of the new pipeline. Sample size is **n = 1 build per leg per
run** and that is sufficient: every check below is deterministic (hashes, counts, names),
not statistical; there is no effect size to estimate because the expected set *is* the
specification. The one numeric threshold is derived from **prior** data, not data this
run produces: known-good v0.1.0 assets are 2.96–3.20 MB, so the plausibility floor is
**1,000,000 bytes** — a 3× margin under the smallest known-good bundle and orders of
magnitude above any broken or empty package (a floor of 3 MB was considered and rejected:
it would fail the known-good 2.96 MB zip).

**Stage A — prove without publishing** (`workflow_dispatch` on the branch):

- **A1** All four legs complete; every per-leg Verify step exits 0.
- **A2** Every Verify log prints, for its leg, the SHA-256 of both reference binaries and
  of the same content found inside the produced package, and each pair is equal.

**RED controls (run once, on a scratch branch, before A1/A2 are trusted):**

- **R-phantom** — edit each leg's verify step to also require a third reference hash that
  cannot exist (a nonexistent `diskguardian-cli` build product). Every leg **must fail**.
  A leg that stays green has decorative verification and the slice is killed until fixed.
- **R-strip (macOS leg only, the one runnable end-to-end from this repo's lane)** — after
  the build step, `rm "$APP/Contents/MacOS/diskguardian-admin"`, then run the verify step:
  it **must fail** naming the admin helper. This pins the missing-file arm specifically,
  on the leg whose mechanics are byte-identical to the other three.

**Stage B — publish** (tag `vX`):

- **B1** The draft release carries **exactly** the matrix-derived set (7 assets with the
  full matrix): the two existing `.dmg` names, two `DiskGuardian-<apple-triple>.zip`,
  `DiskGuardian-x86_64-pc-windows-msvc-setup.exe`,
  `DiskGuardian-x86_64-unknown-linux-gnu.deb`,
  `DiskGuardian-x86_64-unknown-linux-gnu.AppImage`.
- **B2** Every asset ≥ 1,000,000 bytes (floor justified above).
- **B3** `gh release download` of the draft, then SHA-256 of each downloaded asset's
  *inner payload binaries* (extracted as in §3.3) equals the Stage-A log hashes — with
  one exception stated in advance: dmg/zip/exe/deb/AppImage are containers, so the
  downloaded-file hash is checked for the three single-file bundles renamed in §3.5 by
  confirming the rename step's `cp` preserved content (`cmp` against the artifact copy),
  and the payload-binary hashes are re-extracted and re-matched.
- **B4** The collect step's duplicate-name assertion (`ls out | sort | uniq -d` empty)
  passes — recorded in the job log.

**SHIP iff** R-phantom and R-strip fail as required, and A1, A2, B1–B4 all hold.

**KILL / degrade (both directions pre-committed):**

- If any platform cannot produce a hash-verified artifact within **5 CI iterations**, the
  pre-declared fallback fires for that platform: Linux → deb only (§5); Windows → the
  slice is killed (no fallback below NSIS) and the matrix reverts to macOS-only pending a
  design revision. Both platforms failing the cap kills the increment; the honest state
  remains "macOS only" and C30 does not open.
- A partial ship (Windows + deb-only Linux) is a **valid ship** of an amended C30 that
  names the omitted format — the amendment is mechanical (the matrix is the spec), not a
  threshold moved after seeing data.

**UNDERPOWERED (decides nothing):** if `workflow_dispatch` cannot run on the branch, or
the draft release's assets cannot be listed/downloaded (Actions permissions, `gh` auth),
the attempt produces no verdict. Fix the infrastructure and re-run; no ship/kill is
recorded from an attempt that could not have measured anything. An iteration spent this
way does not count against the cap of 5.

**Merge-gate evidence shape:** the PR's rollup should carry the Stage-A run URL, the two
RED-control run URLs, and the Stage-B draft-release asset listing (names + sizes) — the
same terminal-state, complete-rollup standard `scripts/merge-verified.sh` enforces.

## 8. Top risks, ranked (1 = most likely to burn CI rounds)

1. **Linux AppImage tooling on `ubuntu-24.04`.** The bundler's linuxdeploy is itself an
   AppImage; if `APPIMAGE_EXTRACT_AND_RUN=1` is not honored end-to-end, FUSE is needed and
   24.04 does not ship libfuse2 (the package is `libfuse2t64`). Cannot be verified from a
   macOS lane. *Mitigation:* env var set in §3.2; named remediation package; pre-declared
   deb-only fallback. **Falsifier of the design:** AppImage fails the 5-iteration cap even
   with the fallback tooling — then the fallback ships and C30 is amended, not abandoned.
2. **Icon set.** NSIS may refuse `icon.png`-only config (wants `.ico`); deb wants sized
   PNGs. One wasted round if guessed wrong. *Mitigation:* regenerate the full set with
   `bunx tauri icon` in the same PR (§3.7.3). **Falsifier:** bundler rejects the generated
   set (wrong source aspect/size) — fix the source image; cosmetic only.
3. **Windows step shells.** Runner default is pwsh; existing bash idioms fail confusingly
   there. *Mitigation:* `shell: bash` on every Windows step (§3.3). **Falsifier:** a step
   that still runs under pwsh — visible immediately in the log, one-line fix.
4. **7z cannot extract the NSIS payload** (handler/version drift). *Mitigation:* named
   fallback to silent-install verification (`/S /D=`), which is stronger evidence anyway
   (§3.3). **Falsifier:** both routes fail — the Windows leg dies, and with it the slice
   (no Windows fallback), honestly.
5. **Asset-name collisions in the release lane** (the historical one-arch-overwrites-
   another bug class). *Mitigation:* exact-count assertion derived from the matrix + the
   duplicate-name guard (§3.5, B4). **Falsifier:** a duplicate name reaching the upload.
6. **`resolve_helper` `.exe` fix is compile-verified only.** No Windows desktop exists in
   this lane; the runtime message path is unobserved. *Stated, not claimed:* C30 claims
   the binary **ships**, not that the button was pressed on Windows. **Falsifier:** a
   Windows user reporting "helper not found" — the three-line fix is then wrong, not the
   claim.

## 9. BUILD — recommended

The premise survived every check: the symptom's cause is one workflow's matrix (§1);
compilation on all three OSes is already CI-proven, so only bundling is new; the staging
script is already Windows/Linux-triple-aware; the runtime degraded state on the new
platforms is already implemented and honest; no ISA decision blocks it — the Out-of-Scope
line this removes is exactly the line an increment like this exists to remove, and the
project's own goal statement says "cross-platform … macOS-first". The verification
mechanism is hash-based, so it cannot be fooled by bundler naming drift, and every risky
unknown has either a named fallback or a pre-declared degrade path. The value is directly
measurable (assets exist, are triple-named, are hash-verified), and the RED controls make
the proof falsifiable rather than narrative.

**Recommendation: BUILD**, one slice, all four legs, with the §5 fallback and §7
acceptance rule committed with this document before any workflow run.

## 10. Open decisions for the maintainer

1. **msi** — deferred (§5). Confirm or override; overriding adds the WiX lane to the
   matrix and one asset to B1.
2. **Next tag** — the pipeline change should prove itself via `workflow_dispatch` on the
   PR branch (Stage A) and land before the next tag; confirm the next version is 0.2.0
   (README/asset names embed the version).
3. **Roadmap placement** — C30 is a claim, not a DaisyDisk-parity row; if you want it
   numbered on the roadmap anyway, it slots as a new R row of its own, not inside parity
   ordering.
4. **The Measure button on Windows/Linux** — currently untested UI surface showing an
   honest error; if you prefer it hidden until a launcher exists, that is a small frontend
   gate better landed with §3.7.1 than never.
