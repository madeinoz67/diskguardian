# DiskGuardian

**See what is actually filling your disk, and get the space back.**

DiskGuardian draws a volume as an interactive sunburst: every folder a coloured petal,
sized by the space it really occupies on disk. Drill into the big ones, drag what you do
not need into the collector, and delete it. Open source, written in Rust, macOS-first and
built to run anywhere the scanning engine runs.

<p align="center">
  <em>One glance: the space is in Library and Archive. One drag: they are in the collector.</em>
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

Builds a macOS app bundle from source:

```bash
bun install --cwd apps/desktop
bun x --cwd apps/desktop tauri build
# → apps/desktop/src-tauri/target/release/bundle/macos/DiskGuardian.app
```

Requires Rust (stable), Bun, and the Xcode command line tools.

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

## Status

Early. The core loop works end to end on macOS: scan, draw, drill, collect, delete.

Not yet: admin/root scanning, cloud accounts (Dropbox, Drive, OneDrive, Box), APFS local
snapshots, disk images, network volumes, and Windows/Linux builds.

## Licence

MIT.
