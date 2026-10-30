# DiskGuardian

**See what is actually filling your disk, and get the space back.**

DiskGuardian draws a volume as an interactive sunburst. Every folder is a coloured petal,
and the petal's width is the space that folder really occupies on disk. You drill into the
big ones, collect what you do not need, and delete it in one action.

Open source, written in Rust, macOS-first.

## The idea

Finder tells you a category called "System Data". DiskGuardian shows you the folder, at its
actual size, in one picture. There are no presets and no automatic cleanup: you look, you
decide, you delete.

## The pieces

| Piece | What it does |
|---|---|
| Sunburst | Petal width = space. Coloured petals are folders, grey are files, purple is space it could not read. |
| Drill-down | Click a petal to enter it, click the centre to leave. |
| Collector | Drag petals in, watch the total climb, then delete once. |
| Guard | Refuses system locations, and asks before anything large. |

## Start here

- [How it works](how-it-works.md) — the size accounting, the sunburst, and why both are
  built the way they are.
- [Building](building.md) — prerequisites and the two commands that produce an app.
