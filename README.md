# DiskGuardian

**See what is actually filling your disk, and get the space back.**

DiskGuardian draws a volume as an interactive sunburst: every folder a coloured petal,
sized by the space it really occupies on disk. Drill into the big ones, drag what you do
not need into the collector, and delete it. Open source, written in Rust, macOS-first and
built to run anywhere the scanning engine runs.

<p align="center">
  <img src="docs/screenshot.png" width="880"
       alt="DiskGuardian showing a 1.6 TB startup volume as a sunburst: a large Users
            petal at 1.2 TB, a purple Hidden space petal at 210 GB, and Applications at
            75.5 GB, with the same figures listed in the contents column.">
</p>

<p align="center">
  <em>One glance: 1.2 TB is in Users, and 210 GB is space no scanner could see. One drag:
  it is in the collector.</em>
</p>

## Why another disk cleaner

Finder's storage view tells you a category ("System Data"). DiskGuardian shows you *the
actual folder*, at its actual size, in one picture. There are no presets and no automatic
deletion: you look, you decide, you delete.

## What it does

- **A sunburst, not a list.** Petal width is space. Coloured petals are folders, grey are
  files, purple is space it could not read, struck-through items vanished since the scan.
- **Drill in and out.** Click a petal to enter it, click the centre to leave.
- **Hover for detail.** The right-hand column shows the current level, biggest first.
- **Collect before you delete.** Drag petals to the collector, watch the total climb, then
  delete once. A guard refuses system locations and asks before anything big.
- **Honest numbers.** Sizes are physical allocation (`st_blocks * 512`), not file length.
  Hard links are counted once. Space it cannot read is shown as restricted, never as zero.
- **Quick Look.** Press Space on any item for the system's own preview.

## Install

Releases carry per-platform artifacts. The single-file downloads are named with their
full target triples, so a download cannot be mistaken for another architecture's; the
two macOS dmgs keep the bundler's per-architecture names (`aarch64` / `x64`), which
cannot collide with any other asset:

| Platform | Artifacts |
|---|---|
| macOS (Apple silicon) | `DiskGuardian_0.1.4_aarch64.dmg`, `DiskGuardian-aarch64-apple-darwin.zip` |
| macOS (Intel) | `DiskGuardian_0.1.4_x64.dmg`, `DiskGuardian-x86_64-apple-darwin.zip` |
| Windows (x86_64) | `DiskGuardian-x86_64-pc-windows-msvc-setup.exe` (NSIS installer) |
| Linux (x86_64) | `DiskGuardian-x86_64-unknown-linux-gnu.deb`, `DiskGuardian-x86_64-unknown-linux-gnu.AppImage` |

On macOS the app is also in a Homebrew tap:

```bash
brew install --cask madeinoz67/tap/diskguardian
```

The cask is regenerated on every tag from the dmg that release published, so its
checksum and the file you download cannot drift apart. It is unsigned like the direct
download, so the Gatekeeper note below applies to it as well.

Nothing is signed (no signing identity yet). macOS quarantines the download —
right-click the app and choose Open, or run
`xattr -dr com.apple.quarantine /Applications/DiskGuardian.app`. Windows SmartScreen
shows "Windows protected your PC"; choose *More info*, then *Run anyway*. The Linux
`.deb` installs with `sudo apt install ./DiskGuardian-x86_64-unknown-linux-gnu.deb`;
the AppImage needs FUSE, which some distributions omit —
`--appimage-extract-and-run` is the no-FUSE route.

Building from source works the same way on every platform — the bundle targets live in
per-platform Tauri configs, so one command produces the right shape for the host:

```bash
bun install --cwd apps/desktop
bun run --cwd apps/desktop tauri:build
# macOS   → target/release/bundle/{macos/DiskGuardian.app, dmg/*.dmg}
# Windows → target/release/bundle/nsis/*-setup.exe
# Linux   → target/release/bundle/{deb/*.deb, appimage/*.AppImage}
```

On macOS and Linux, `tauri:build` first stages the `diskguardian-admin` measurement
helper for the bundler (Tauri's `externalBin` wants it prebuilt, target-triple-suffixed,
under `src-tauri/binaries/`); if you only want `cargo build`/`test`/`clippy` on the whole
workspace, run `bun run --cwd apps/desktop stage:admin` once first, because the app
crate's build script validates the staged helper. Windows ships no helper, so there is
nothing to stage there.

Requires Rust (stable) and Bun, plus the Xcode command line tools on macOS and the
usual webview libraries on Linux.

## Development

```bash
cargo test -p diskguardian-core     # scan engine: sizes, links, permissions, guards
cd apps/desktop && bun test          # sunburst layout maths
cd apps/desktop && bun run tauri:dev # the app, hot-reloading
```

The frontend also runs in a plain browser (`bun run dev`) against a synthetic tree, which
is useful for layout work.

## Architecture

```
crates/core/          pure Rust, no UI: the scanning engine
  scan.rs             parallel walk, physical-size accounting, hard-link dedupe
  model.rs            the tree: own_bytes / total_bytes and the invariant between them
  view.rs             pruning for display: big children kept, long tail merged
  collect.rs          the collector, with overlap counted once
  delete.rs           guarded deletion; Trash by default, permanent on request
crates/admin/         diskguardian-admin: one-shot read-only helper behind the
                     administrator dialog (job file in, results file out; measures,
                     never deletes)
apps/desktop/
  src-tauri/          the Tauri shell: IPC commands, Finder reveal, Quick Look
  src/                the sunburst: layout maths, canvas renderer, interaction
```

The engine is deliberately UI-free so a CLI, a Windows build, or a test harness can reuse
it unchanged.

## Safety

Deletion is the only destructive thing here, so it is the most constrained:

- The backend deletes only paths that came from a scan it holds in memory.
- System trees (`/System`, `/usr/bin`, `/bin`, `/sbin`, `/private/etc`, …) are refused.
- The scanned root itself is refused.
- The default is the macOS Trash: you can change your mind until it is emptied.
- Deleting more than 5 GB requires explicit confirmation.

## Administrator measurement

Some areas (other users' homes, most of `/Library`, parts of `/private/var`) cannot be
read with your own permissions, and DiskGuardian draws them as purple "restricted"
petals rather than pretending they are empty. After such a scan, the status bar offers
**Measure restricted areas…** — the first pass raises one system password dialog, and
those petals are re-measured and redrawn at their real sizes.

What that pass is, plainly:

- **The dialog belongs to macOS.** The app never sees, holds, or caches your password.
  There is **one prompt per app session**: approving it starts a small measurement
  helper that serves every later pass without asking again. The helper lives exactly
  as long as the app does — it exits when the app exits (its working directory goes
  away), when the app's heartbeat stops (a crash), or after 15 idle minutes — and no
  privileged process exists between app runs.
- **The privileged side measures; it never deletes.** The helper walks the restricted
  paths read-only and writes the numbers back. It contains no deletion code at all,
  and deletion continues to run unprivileged, behind the same guard as always. Seeing
  a protected folder is never the same as being able to delete it.
- **You may still have purple petals afterwards.** Root does not bypass macOS privacy
  controls (TCC), so folders like `~/Library/Mail` may stay restricted until DiskGuardian
  is granted Full Disk Access. Whatever could not be measured is reported with a count,
  never silently dropped.

**On Windows**, the same feature works by self-elevation: measuring restricted areas
asks Windows for administrator rights — one UAC prompt per app session, raised only
when you press *Measure restricted areas…*, never at launch. The app re-launches itself
elevated to measure, and the elevated copy only measures; it never deletes. Decline the
prompt and everything else works, with those areas shown as restricted. This is new in
0.1.1.

**On Linux**, restricted-area measurement is not available yet; those areas stay shown
as restricted.

## Hidden space

A scan of a **volume** (its root is a mount point — the startup disk, an external disk)
does not stop at the flower. The volume's own used space is measured fresh at every
view, and the gap between that figure and everything the scan could see is drawn as one
**Hidden space** petal — its width is always exactly `volume used − scanned tree` at
the moment you are looking at it, so deleting files grows it in the same frame, with no
rescan.

Click the petal and the panel says what the figure is, and what it is not:

- **The hidden figure is measured.** It is the volume's used space (statfs — the figure
  `df` prints) minus the scanned tree. If the tree ever totals *more* than the volume
  reports used — the disk changed during or after the scan — the petal is omitted and
  the panel says so in words, never a silent zero.
- **The breakdown is held.** The gap could in principle be split into other volumes of
  the same APFS container, local snapshots, protected areas, and a still-hidden
  remainder. That split is **not shown**: the acceptance rule that was written before
  any measurement could not confirm it on a second volume, so no category is presented
  at all — an unshown category can never be mistaken for an empty one. The measuring
  code is kept and tested in the app's volume module, waiting on that cross-check.
- **Purgeable space is an annotation, not a slice.** macOS also reports how much it
  *may* free on its own (snapshots, caches, offloaded content). That pool overlaps
  space the scan already counts, so it is shown beside the figure as "up to N" — never
  as part of the hidden figure, and nothing in this app offers to purge it. The only
  purges are the ones you ask for yourself: your snapshots (as you, not elevated) and
  your Trash.

A volume scan also stays on its own disk: it walks only the scanned volume's device
(the startup volume's System and Data volumes count as one, because macOS presents them
as one), never enters other disks or transient snapshot mounts, and lists the mounts it
excluded.

## Favourite folders

Any folder you keep coming back to can be starred, and the favourites then sit at the
top of the "Disks and folders" page — above home, Trash, snapshots and every volume —
each with a Scan button and a star to remove it again.

- **Star from the scan screen.** The contents list on the right carries a star beside
  the `+` on every folder row, and the list's heading has one for the folder you are
  currently inside. Starring never navigates.
- **Unstar from the front page.** The ★ beside a favourite removes it. Unstarring
  removes a list entry, never a file.
- **Where the list lives.** A `favourites.json` file in the app's own data directory
  (`~/Library/Application Support/com.madeinoz.diskguardian/` on macOS), written
  atomically by the app itself. The webview never writes favourite state, so a webview
  bug cannot corrupt the list; and if the file ever is unreadable, it is kept beside a
  fresh one and the page says so rather than reading as "no favourites".
- **"Not on this disk right now."** A favourite whose folder has been deleted, or whose
  volume is not mounted, stays on the page with that sentence as its detail line and
  its Scan disabled — reported, never silently dropped. It re-enables itself the moment
  the folder exists again; until then, unstarring is the way to clear it.
- **One folder, one row.** Starring a folder that is already a favourite — including
  through the other spelling of a symlinked path — is a no-op. Favourite rows show no
  size, because a size would mean walking every favourite on every page load.

## Status

Early. The core loop works end to end on macOS: scan, draw, drill, collect, delete, and
administrator measurement of restricted areas.

Not yet: cloud accounts (Dropbox, Drive, OneDrive, Box), APFS local
snapshots, disk images, network volumes, and restricted-area measurement on Linux.

## Licence

MIT.
