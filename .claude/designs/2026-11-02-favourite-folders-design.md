# R8 — Favourite folders: design

Date: 2026-11-02 · Author: increment-designer · Status: proposed (not implemented)

**High-risk areas, named in the first paragraph as required:** this increment touches
**none of the three** — no change to physical-size accounting, no change to
restricted-space reporting, no change to the deletion guard or any delete call site. It
touches the surface C19 owns (the "Disks and folders" screen) and the scan screen's
contents list. `crates/core` is untouched: no new core API, no change to `Tree`, `Node`,
or the serialized `ViewNode` that crosses the IPC boundary. Every new payload is an
additive command response, which is deliberately outside the high-risk
old-frontend-new-tree class.

---

## 1. What exists today, and the one lesson that shapes this design

- The overview (`main.ts:showOverview`, `main.ts:840`) is rebuilt from scratch on every
  visit: home row, Trash row, snapshots row, then volumes from `sysinfo`. Rows come from
  `volumeRow` (`main.ts:954`), which already supports a custom action label, action tone
  and action handler — the Trash ("Empty") and snapshots ("Review") rows use exactly that.
  A favourites section is a new caller of an existing pattern, not a new row type.
- The **only** persistence in the app today is the legend width in `localStorage`
  (`main.ts:781-800`). That is a UI preference whose loss degrades gracefully to a
  default. Favourites are user data; the decision below treats that difference as the
  whole argument.
- The scan screen's contents list (`renderLegend`, `main.ts:454`) draws the **pruned
  view's** children. Real children carry their tree `node_id`; the synthetic "smaller
  objects" petal carries `node_id: None` (`view.rs:129`), so it already gets no collect
  button and will get no star. The collect button (`main.ts:527-539`) is the interaction
  template: a 20 px bordered glyph button, `stopPropagation` against the row's navigate
  click, `.on` state in amber.
- **The frontend has no paths.** Nodes are identified by `scan_id` + `node_id`; paths are
  reconstructed server-side by `Tree::path_of` (the 2026-10-30 no-`PathBuf`-per-node
  decision). `reveal` and `quicklook` already resolve paths through the `node_path` seam
  (`commands.rs:766`). Any starring from the scan screen must do the same.
- **The C17 lesson is the root cause this design must not re-import.** `guard`
  canonicalises (`delete.rs:128`, `fs::canonicalize`), while `path_of` returns the
  *literal walked path* — on macOS `/tmp/…` stays `/tmp/…` through the walk and comes
  back from `guard` as `/private/tmp/…`. That mismatch once made delete prune nothing
  (ISA C17 correction). Favourites stored as literal strings would show the same class as
  duplicate rows: starring `/private/tmp/x` from the picker and `/tmp/x` from the typed
  path field would be two entries for one folder, violating the roadmap's "never a
  duplicate row" requirement. Path identity is decided in §3.4.

## 2. The four decisions, reasoned

### 2.1 Where the list lives: Rust-owned JSON in the app data directory

**Decision: a `favourites.json` file in `app.path().app_data_dir()` (Tauri 2 core path
API, no plugin), owned by Rust, reached only through commands.** Not `localStorage`, not
a bare file the webview can name, not in-memory state.

- **Survival.** `localStorage` lives in the WKWebView data store, which the OS may evict
  and which is not contractual across reinstall; the app data directory survives bundle
  replacement and is where a desktop app's user data belongs.
- **Testability without a DOM.** `bun test` cannot reach `localStorage`; `main.ts`
  "touches the document at import, so there is no seam" (ISA log, 2026-11-02) — the
  exact reason legend width has no test. A Rust-owned store is testable with the existing
  mock-runtime harness (`src-tauri/tests/ipc.rs`) against a temp directory, including the
  property that matters most: **persistence across relaunch**, which a webview store
  cannot test at all.
- **The frontend cannot corrupt its own store.** A webview bug can write arbitrary
  `localStorage`. Here the webview can only reach favourites through four commands whose
  inputs are a `scan_id`+`node_id` (path resolved server-side) or a shape-validated
  path. Corruption is limited to torn writes, killed by the tmp+rename below.
- **Two windows, one store.** Single-window app today; when a second window exists, both
  share the process and the `Mutex` in `AppState`, so writes serialise. Two *processes*
  would be last-write-wins — out of scope, stated.
- **Why Rust must own it anyway:** the scan screen stars by node id. Only Rust can turn
  `node_id` into a path (§1). A `localStorage` list would force the frontend to
  reconstruct paths from the name chain — duplicating `path_of` in TypeScript, which the
  no-`PathBuf` decision exists to avoid.

### 2.2 What "missing" means and how it is shown

A favourite is **missing** when `Path::exists()` on the stored string is false — the
folder was deleted, or the volume it lived on is unmounted. `exists()` follows symlinks
(a favourite stored through `/tmp` answers for `/private/tmp`), and stat succeeds on
TCC-protected directories (the scan of such a favourite yields restricted petals —
C4/C25 behaviour, which is correct, not "missing").

A missing favourite:

- **stays on the page** (A3's spirit applied to lists: reported, never silently dropped),
- carries visible text — `not on this disk right now` — as its detail line (the C26/C27
  doctrine: a reason is visible text, not a tooltip),
- has its **Scan button disabled** with a title naming why, so it never offers a scan
  that would fail without saying so,
- keeps its **unstar** button enabled — unstarring is the way to clear it,
- re-enables itself the moment it exists again, because `showOverview` re-renders and
  re-stats on every visit.

### 2.3 How a folder is starred from the scan screen: a star in the legend

No modal, no new top-bar row. Two affordances, both inside the existing legend:

1. **A star button on each directory row**, beside the collect button — the same 20 px
   glyph-button pattern (`.collect`, `style.css:291`), amber when on, `stopPropagation`
   against the row's navigate click. Rendered only for `node_id !== null` and
   `kind === 'dir'` — never files, symlinks, restricted petals, aggregates or the
   "measuring" placeholders of the progressive paint.
2. **A star in the legend heading** (the `name · total bytes` block, `main.ts:462-465`)
   starring the **current folder** — the frame's node, `nodeId ?? null` mapping to the
   tree root exactly as `view` does (`commands.rs:266`). This is what makes the scan
   root itself starrable: scan home, star the heading, home is a favourite.

The heading block is currently built with `innerHTML` interpolating `root.name` — a
disk-derived string into HTML. Rebuilding it with `createElement`/`textContent` to host
the star fixes that incidentally; it is called out here rather than done silently.

### 2.4 Path identity: the canonical form is the store's key

- **At write**, the path is canonicalised best-effort: `fs::canonicalize(path)`
  falling back to the literal string when it fails (a folder that vanished between scan
  and star). While a folder exists, `/tmp/x` and `/private/tmp/x` both land as
  `/private/tmp/x` — one entry, satisfying "starring something already starred is a
  no-op".
- **At comparison** (the marks query, §4.3), *both* sides are canonicalised best-effort
  before matching — because `path_of` returns the literal walked path (§1) and the store
  holds the canonical one. Comparing literally here would show an empty star over a
  folder that is in fact starred: the C17 mismatch wearing a different hat.
- **At existence check** (the overview), the stored string is stat'ed as-is; no
  canonicalisation of a path that may be gone.
- **Removal** matches the exact stored string — safe because the UI passes stored
  strings back verbatim (the row carries the store's own path).
- **Stated limit, not hidden:** `fs::canonicalize` on macOS does not case-fold, so
  `/Users/NAME/x` and `/Users/name/x` remain two entries on a case-insensitive volume.
  This can only arise from typing a case variant of an existing favourite into the
  manual path field (the picker and the scan screen return the disk's own spelling). The
  failure mode is a visible duplicate row, never data loss. Case-folding per platform is
  deferred (§6).

## 3. The mechanism

### 3.1 Backend: `apps/desktop/src-tauri/src/favourites.rs`

New module, the shape of `snapshots.rs` (commands + pure helpers in one file, registered
in `lib.rs`).

**Store** (`Favourites`, held as `Mutex<Favourites>` in `AppState`):

```rust
pub struct Favourites {
    path: Option<PathBuf>, // resolved lazily on first use; None = not yet resolved
    paths: Vec<String>,    // canonical-at-write keys, insertion order
    loaded: bool,
    notice: Option<String>, // set when a corrupt store was recovered this load
}
```

- Lazily initialised on first command use, because `AppState::default()` runs in
  `configure` before an `AppHandle` exists (the same reason `admin_session` starts
  `None`, `state.rs:21`). Resolution: `app.path().app_data_dir()`; on error the commands
  return a typed error and the UI shows the banner — favourites degrade loudly, never
  silently.
- **Test seam:** a `set_path(PathBuf)` used by the mock-runtime tests to point the store
  at a `tempfile` directory before first load — the same seam philosophy as
  `admin_launch` (`state.rs:24`). Production never calls it.
- **File format**, schema-disciplined the way the admin job file is:

  ```json
  { "version": 1, "paths": ["/private/tmp/fixture"] }
  ```

  `deny_unknown_fields`; unknown `version` or malformed JSON takes the corrupt path:
  rename the file to `favourites.json.corrupt-<unix-ts>`, start empty, set the notice.
  A corrupt store is reported on the overview, not silently emptied (the C27 rule: a
  failed listing never reads as "none").
- **Writes are atomic:** serialise to `favourites.json.tmp` in the same directory, then
  `fs::rename` over the target. A torn write must never lose the list — unlike the
  legend width, this is user data.
- **Mutations:** `set(path, add)` — add canonicalises (§2.4), refuses exact duplicates
  (the no-op), appends at the end (insertion order; recency reordering deferred);
  remove drops the exact match. Every mutation persists before returning.

**Payloads** (additive; `Tree`/`Node`/`ViewNode` shapes untouched):

```rust
#[derive(Serialize)]
pub struct FavouriteEntry { pub path: String, pub exists: bool }

#[derive(Serialize)]
pub struct FavouritesReport {
    pub favourites: Vec<FavouriteEntry>,
    pub notice: Option<String>, // corrupt-store recovery, shown as visible text
}

#[derive(Serialize)]
pub struct FavouriteMarks { pub self_: bool, pub children: Vec<u64> }
```

**Four commands**, registered in `lib.rs` beside the rest:

| Command | Args | Returns | Used by |
|---|---|---|---|
| `favourites` | — | `FavouritesReport` | overview render |
| `favourite_marks` | `scanId`, `nodeId: Option<u64>` | `FavouriteMarks` | legend render (scan + navigate) |
| `favourite_node` | `scanId`, `nodeId: Option<u64>`, `add: bool` | `Vec<String>` | legend star / heading star |
| `set_favourite` | `path: String`, `add: bool` | `Vec<String>` | overview unstar |

- `favourite_marks` resolves `path_of` for the node's tree children and the node itself,
  canonicalises both sides (§2.4), and reports which children are favourites plus
  whether the node itself is. It is read-only over the scan tree plus one
  already-loaded store — the same cheapness `revalidate` already relies on
  (`commands.rs:611-617`). Children the legend prunes away are harmless: the frontend
  only consults marks for rows it draws.
- `favourite_node` resolves the path via the existing `node_path` seam and refuses: a
  file node ("only folders can be favourites"), a node flagged `missing`
  ("that folder is no longer on disk"), and unknown scans (typed error, the C18
  standing pattern). `node_id: None` maps to the tree root, so the heading star works on
  the scan root.
- `set_favourite` validates **shape only** — absolute, no `..` component — the exact
  rule C25 taught: readability/existence belongs to the walk or the stat, never to
  validation (`malformed_paths_are_refused_before_anything_is_walked`).
- All four are synchronous commands doing microseconds to a few milliseconds of work
  (string building, one stat per favourite, a tiny file write) — the same class as
  `peek_root`, which does a `read_dir` synchronously. No `spawn_blocking` needed; A4 is
  unaffected.

### 3.2 Frontend: `apps/desktop/src/favourites.ts` (the tested half)

Pure model, the precedent of `overview.ts`/`snapshots.ts` — no DOM, `bun test`-able:

```ts
export interface FavouriteRowModel {
  name: string        // basename of the stored path (existing tested helper, overview.ts:44)
  path: string        // the store's own string, passed back verbatim on unstar
  detail: string      // visible text: the path, or the missing sentence
  unavailable: boolean
}
export function favouritesModel(report: FavouritesReport): FavouriteRowModel[]
```

Model properties, each pinned by a test:

- a missing favourite produces `unavailable: true` and the exact sentence
  `not on this disk right now` — the model **never filters** rows by `exists`;
- store order is preserved, no sorting, no dedup in the model (dedup is the store's
  contract, tested in Rust; the model renders what it is given);
- the `notice`, when present, is returned to the caller to render as the section's
  visible text — never swallowed (a corrupt store must not read as "no favourites").

### 3.3 Frontend wiring: `main.ts`, `bridge.ts`, `types.ts`

- **Overview:** `showOverview` gains a favourites block **above every other row** —
  favourites first, then home, Trash, snapshots, volumes (the roadmap says "above the
  volumes"; favourites-first is the stronger reading, and moving the block is a one-line
  change). One `bridge.favourites()` call; rows built by `volumeRow` with
  `detail` = the path (or the missing sentence), `actionLabel: 'Scan'` →
  `scanPath(entry.path)`, and a second control: an unstar button. `volumeRow` grows a
  `disabled` option (button `disabled` + explanatory `title`) for the missing case — a
  small extension of an existing options object, not a new row type. The unstar button
  calls `set_favourite(path, false)` and re-renders the block from the response.
- **Scan screen:** after a scan resolves and after every `navigate()`, one
  `bridge.favouriteMarks(scanId, nodeId)` call fills a `Map<number, boolean>` of child
  marks plus a `self` boolean. `renderLegend` renders the star on `kind === 'dir'` rows
  (filled `★` when marked, hollow `☆` otherwise) and the heading star on the current
  folder. `up()` re-uses cached frames and does not re-fetch: marks only change through
  star clicks on the level being viewed, which update the map synchronously from the
  command response — and the overview (the only place a favourite can be *removed*)
  cannot be left back into an existing scan today, so the staleness window is
  unreachable. A new scan clears the map. Star click: `stopPropagation`, then
  `bridge.favouriteNode(...)`; failure shows the `showError` banner (the C18 lesson:
  a broken control must not look like a working one).
- **Bridge demo mode** (`isTauri === false`): an in-memory list seeded with one existing
  and one missing synthetic entry, so the overview and the model stay viewable in a
  browser — the existing demo pattern (`bridge.ts:214` onward), with the demo never
  claiming persistence it does not have.

### 3.4 What is deliberately absent

**No sizes on favourite rows.** A size would require walking every favourite on each
overview render (A4's spirit, violated synchronously) or building a last-scan cache
keyed by path (a real design, deferred with the rest in §6). A row without a size has no
size to get wrong — A2 is untouched by construction.

## 4. Minimal first slice

1. `favourites.rs`: store, load/persist, recovery, four commands; `state.rs` gains the
   `Mutex<Favourites>` field; `lib.rs` registers the commands.
2. `src-tauri/tests/ipc.rs` (or a sibling `favourites` integration test file reusing the
   harness): the nine tests of §5.
3. Frontend: `types.ts`, `bridge.ts` (+ demo), `favourites.ts` model + `bun test`s,
   `main.ts` (legend star, heading star, overview block, `volumeRow.disabled`),
   `style.css` (`.favourite` sharing the `.collect` pattern; the favourites block).
4. README: a "Favourite folders" section — star where, unstar where, where the list
   lives on disk, what "not on this disk right now" means.
5. Verification: full five-gate battery; the two screenshots and the driven-app click of
   §5.

## DEFERS (explicitly not in this increment)

- **Sizes or last-scan dates on favourite rows** — needs a scan-result cache keyed by
  canonical path; its own design (overlaps R14/R11 "Forget scan results").
- **Starring from the picker or the typed path field.** The OS picker cannot host UI; the
  affordance would be a separate "Star a folder…" button beside "Scan Folder…". The scan
  screen already covers every folder a scan can see, which is the principal's stated
  flow. Reopen trigger: the principal asks for it.
- **Case-folding the key on case-insensitive volumes** (§2.4 limit). Reopen trigger: a
  real duplicate row reported from a real disk.
- **Recency ordering, drag-to-reorder, favourite folders shown in the breadcrumb** —
  polish, not function.
- **Multi-process store sync** (single store per process, last write wins).
- **Starring restricted or symlink nodes from the scan screen** — star is `kind === 'dir'`
  only; a favourite *added by path* may point at anything the shape check allows, and its
  existence check is honest about what it finds.

## 5. Pre-committed acceptance rule

Written before any implementation, before any screenshot, before any number was looked
at. The correctness half rests on tests; the appearance half rests on two named
screenshots — the split is stated per property so neither half can quietly cover the
other.

**Tests (correctness — `cargo test -p diskguardian-app` + `bun test`):**

1. `starring_a_folder_persists_across_relaunch` — star via `favourite_node`, tear the
   app down, rebuild with the same seam path, `favourites` still lists it (`exists:
   true`).
2. `starring_twice_is_a_no_op_never_a_duplicate_row` — two stars, one entry.
3. `symlink_spellings_collapse_to_one_entry` — unix-gated (test **and** its imports and
   fixture helper gated, per the platform-gating rule): star `/tmp`-spelling and
   `/private/tmp`-spelling of one fixture, one entry.
4. `a_missing_favourite_is_reported_not_dropped` — star, delete the folder, `favourites`
   returns it with `exists: false`.
5. `marks_resolve_children_and_self` — star a child; `favourite_marks(parent)` names it
   in `children`, `self_: false`; `favourite_marks(child)` gives `self_: true`.
6. `files_and_missing_nodes_cannot_be_favourited` — typed errors.
7. `set_favourite_refuses_relative_and_dotdot_paths` — shape rule (C25 pattern).
8. `a_corrupt_store_is_reported_and_reset` — garbage file → `notice` set, empty list,
   `.corrupt-…` file preserved.
9. `unstarring_removes_the_entry` — exact string removal, list shrinks.
10. `bun test` on `favourites.ts` — the model never filters missing favourites, preserves
    order, surfaces the notice, and the missing sentence is exact.

**RED discrimination, required before any green is believed:** neutralise the store's
duplicate refusal (make `set` always push) → test 2 fails; neutralise write-time
canonicalisation (store the literal) → test 3 fails; neutralise the existence stat
(always `true`) → test 4 fails. A test that stays green with its mechanism neutralised
is fluff and blocks the increment until rewritten.

**Screenshots and driven app (appearance/behaviour — on the built bundle):**

- `/tmp/dg-r8-overview.png`: the favourites block above the volumes, one starred row
  with a working Scan, one unavailable row whose reason is **visible text**, an unstar
  control per row, no duplicate rows.
- `/tmp/dg-r8-legend.png`: a starred folder's legend row with a filled star beside its
  collect button, and the heading star on the current folder.
- One driven click: starring from the legend must not navigate (row stays on the same
  folder) — the C18 class of check, closed only by driving the app.
- One relaunch: star in the built app, quit, relaunch, the favourite is still there —
  persistence proven on the real store location, not just the seam.

**The rule in one sentence.** Ship if and only if tests 1–10 pass with all three RED
discriminations demonstrated, and both screenshots and the driven click check out on the
built app; **kill** (fix before merge) if any probe or screenshot shows a duplicate row,
a silently dropped missing favourite, a favourite lost across relaunch, or a star click
that navigates; **if the screenshots cannot be captured this run** (screen lock bit
C27 once), the code may merge on the tests alone but C28 stays open on appearance
exactly as C27 did — tests green plus no screenshot is "open", never "closed".
Sample size: the full suite once per gate, one real machine for the manual run; every
property is binary, so there is no effect size to tune and no threshold to move after
the fact.

## 6. Claim and anti-claim impact

- **A1** — untouched: no delete call site moves; unstarring removes a list entry, never
  a file.
- **A2** — untouched by construction: favourite rows show no size (§3.4).
- **A3** — extended in spirit: a favourite that has gone is *reported* (`exists: false`,
  visible text, kept row), never dropped. The claim text below carries this explicitly.
- **A4** — preserved: no new long-running command; all four are `peek_root`-class.
- **A5** — preserved: nothing here needs any privilege.
- **A6** — untouched: no arcs change.
- **C19** — no amendment needed: it does not assert the overview's list is exhaustive,
  and its falsifier ("only one hard-coded root") is unaffected. C28 owns the favourites
  section.
- **C17's lesson** — this design is downstream of it: canonical keying (§2.4) exists
  because of that incident and cites it.

**New claim, exact text proposed for ISA.md § Claims:**

> ### C28 — Favourite folders persist, and a favourite that has gone says so
> Any folder visible in a scan can be starred from the contents list, and the current
> folder from the list's heading, without a dialog; whether a node is a favourite is
> resolved from its path on the Rust side, never guessed in the UI. Favourites persist
> in the app's own data directory as JSON and appear on the "Disks and folders" page
> above every other row, each with a Scan button and an unstar. Starring a folder that
> is already a favourite — including the other spelling of a symlinked path — is a
> no-op that never produces a second row. A favourite whose folder no longer exists
> stays on the page, marked unavailable in visible text with its Scan disabled, until it
> is unstarred.
> **Probe:** `cargo test -p diskguardian-app --test ipc favourites` — persistence across
> a relaunch on one store file; star-twice is a no-op; `/tmp` and `/private/tmp`
> spellings collapse; a deleted favourite is returned with `exists: false`; files and
> missing nodes are refused; relative paths are refused; a corrupt store is reported and
> reset, not silently emptied. `bun test` on `favourites.ts` — the model never filters
> missing favourites and never swallows the recovery notice.
> **Falsifier:** a duplicate favourites row; a missing favourite silently dropped; a
> favourite lost across relaunch; favourite state written anywhere by the webview rather
> than through the command surface.

**New § Decisions entries proposed:**

1. *Favourites are Rust-owned data, not webview state* (2026-11-02) — a JSON store in
   the app data directory reached only through commands; `localStorage` is for UI
   preferences that degrade to defaults (the legend width), not for user data that must
   survive relaunch and be testable without a DOM.
2. *The canonical path is the favourites key* — writes canonicalise best-effort,
   comparisons canonicalise both sides (`path_of` returns literal walked paths; `guard`
   canonicalises — the C17 mismatch is the reason). Case-folding is a stated limit.
3. *The star lives in the legend, not the top bar* — rows for directory children, the
   heading for the current folder; no modal, no new top-bar row.

## 7. Surfaces that must move

| Surface | Change |
|---|---|
| Core (`crates/core`) | **none** |
| Serialized `Tree`/`Node`/`ViewNode` | **none** — additive command payloads only |
| Rust app | new `favourites.rs`; `state.rs` one field; `lib.rs` four registrations |
| Tests | `src-tauri/tests/` favourites suite; `apps/desktop/src/favourites.ts` + its tests |
| TypeScript | `types.ts`, `bridge.ts` (+ demo mode), `main.ts` (legend + overview + `volumeRow.disabled`), `style.css` |
| Docs | README "Favourite folders" section |
| ISA | C28 + three § Decisions entries (maintainer lands these) |

## 8. Top risks, each with the probe that would catch it

1. **`app_data_dir` resolution differs or fails across launch contexts** (dev binary vs
   bundle vs mock runtime; missing identifier or HOME). Caught by: the typed
   storage-unavailable error surfacing in the banner (never a silent no-favourites), and
   by the manual relaunch check on the built app — a star lost across relaunch kills.
2. **Path-identity drift produces duplicate or phantom rows** (case variants, symlink
   re-pointing, the canonicalise-fails fallback storing a literal). Caught by: tests 3
   and 5 (the symlink collapse and the two-sided canonical comparison), and by any
   duplicate row in the overview screenshot. The residual case-variant limit is stated
   in §2.4, not hidden.
3. **The legend refactor breaks existing interactions** — the star click navigating the
   row, the heading rebuild changing the heading render, keyboard focus order in the
   narrow column. Caught by: the driven-app click check (star click must not navigate),
   `bunx tsc --noEmit`, the existing `bun test` suite, and the legend screenshot. This
   is the C18 class: the click path is only honest once it has been driven.

## 9. Precedent

- **`needs_confirmation` computed in Rust** (`commands.rs:663-672`): UI-truth decided
  server-side; favourite-ness follows the same shape.
- **`node_path` seam** (`commands.rs:766`): `reveal`/`quicklook` resolve paths from
  `scan_id`+`node_id`; `favourite_node` is a fourth caller.
- **`snapshots.rs`**: commands plus pure helpers in one app module, with the
  failed-listing-never-reads-as-none doctrine this design's `notice` copies.
- **C25's shape-only validation**: absolute + no `..`, nothing more.
- **C17**: the canonicalisation mismatch that §2.4 exists to not re-import.
- **The mock-runtime IPC harness** (`src-tauri/tests/ipc.rs`, built for C18): every new
  test extends it; the `admin_launch` seam is the pattern for `set_path`.
- **`.collect` button** (`style.css:291`, `main.ts:527`): the star's visual and
  behavioural template, `stopPropagation` included.

## 10. Build or DON'T-BUILD

**BUILD.** No core change, no high-risk area, every surface has an in-tree precedent,
and the acceptance rule is checkable end to end: nine Rust tests, a model test suite,
two named screenshots, one driven click, one relaunch. The only genuinely new mechanism
is the store file, and it is small, atomic, and seam-testable.

## 11. Open questions for the maintainer

1. **Block placement** — favourites above *every* row (this design, the stronger reading
   of "above the volumes") or above the volumes but below home/Trash/snapshots?
   One-line change either way; say before the build.
2. **The case-variant limit** (§2.4) — accept as stated, or is a macOS case-fold worth
   its platform-specific rabbit hole now? Recommendation: accept, reopen on a real
   report.
3. **C28's wording** — drafted verbatim in §6 for landing; the "above every other row"
   phrase should match whatever you decide for question 1.
