# How it works

## Sizes are physical, not logical

A file's length (`st_size`) is not the space it takes. A sparse file can be 64 MB long and
occupy 8 KB. A 1-byte file still occupies a whole block.

DiskGuardian reports `st_blocks * 512`, which is what the filesystem actually committed.

Two consequences follow, and both are load-bearing:

- **Hard links are counted once.** Four names pointing at one inode hold one file's bytes.
  The tree marks the duplicates `link_skipped` and gives them zero size, so the names stay
  visible without inflating the total.
- **Directories have size too.** A directory's own metadata blocks are real occupied space,
  so they count toward its own bytes rather than vanishing.

!!! note "Not yet done"
    APFS clone detection is not implemented, so two clones are currently counted twice. This
    is the largest known accuracy gap and it is recorded in `ISA.md`.

## Space we cannot read is still space

If a directory cannot be opened, it becomes a `Restricted` node carrying the reason, and
every ancestor is flagged `partial`. The alternative, reporting zero, is how a disk cleaner
ends up telling you that 40 GB does not exist.

Because restricted space is visible, so is the decision to close it: running the app with
full disk access removes most of it.

## The sunburst

One ring per depth. Within a folder, its children tile the folder's angular span in strict
proportion to their allocated bytes, largest first, clockwise from twelve o'clock.

Two rules keep it legible:

- **Big children only.** A view keeps children until it covers 99.5% of the parent or hits
  256 of them, whichever comes first. The remainder merges into one semi-transparent
  "smaller objects" petal, exactly as DaisyDisk does.
- **Five rings.** Beyond that the petals are thinner than the gaps between them.

The layout maths is pure and lives in `apps/desktop/src/layout.ts`, which is why it can be
tested without a browser: angles must sum to the circle, arcs must not overlap, and every
arc must be findable by a point inside it.

## Deletion

Deletion is the only destructive operation, so it has the tightest constraints:

1. The backend deletes only paths that came from a scan it is currently holding. The UI
   cannot hand it an arbitrary path.
2. System trees are refused: `/System`, `/usr/bin`, `/usr/lib`, `/bin`, `/sbin`,
   `/private/etc`, `/private/var/db`, `/Library/Apple`.
3. The scanned root itself is refused.
4. The default is the macOS Trash, so the decision is reversible until the Trash is emptied.
5. More than 5 GB requires explicit confirmation.

The guard is deliberately narrow. Protecting `/usr` and `/private` wholesale would refuse
the user's own temp directory and every Homebrew cache, which is where a lot of the space
usually is.
