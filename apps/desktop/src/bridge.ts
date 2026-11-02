// The bridge to Rust. In the Tauri webview every call is an IPC command; opened in a
// plain browser (dev, screenshot, layout review) it serves a synthetic tree with the
// same shape, so the sunburst can be looked at without a native window.

import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import type {
  AdminProgress,
  AdminScanReport,
  BranchDone,
  CollectState,
  DeleteReport,
  DuplicateReport,
  FavouriteMarks,
  FavouritesReport,
  FreshnessReport,
  CleanupReport,
  EmptyReport,
  HiddenSpaceReport,
  PurgeReport,
  ScanResponse,
  SnapshotReport,
  TrashStats,
  ViewNode,
  VolumeInfo,
} from './types'

export const isTauri = typeof window !== 'undefined' && '__TAURI_INTERNALS__' in window

export async function homeDir(): Promise<string> {
  if (!isTauri) return '/Users/you'
  return invoke<string>('home_dir')
}

export async function volumes(): Promise<VolumeInfo[]> {
  if (!isTauri) {
    return [{ name: 'Macintosh HD', mount: '/', total_bytes: 994_662_584_320, free_bytes: 412_316_860_416 }]
  }
  return invoke<VolumeInfo[]>('volumes')
}

export async function peekRoot(path: string): Promise<string[]> {
  if (!isTauri) return DEMO.map((spec) => spec.name).sort()
  return invoke<string[]>('peek_root', { path })
}

/** Subscribe to per-branch progress. Returns an unsubscribe function. */
export async function onScanProgress(handler: (branch: BranchDone) => void): Promise<() => void> {
  if (!isTauri) {
    demoListeners.add(handler)
    return () => demoListeners.delete(handler)
  }
  const unlisten = await listen<BranchDone>('scan-progress', (event) => handler(event.payload))
  return unlisten
}

export async function scan(path: string, nonce: string): Promise<ScanResponse> {
  if (!isTauri) return demoScan(path, nonce)
  return invoke<ScanResponse>('scan', { path, nonce })
}

/**
 * Ask the walk in flight to stop. Called before every scan, so it has to be a no-op when
 * nothing is running — which is the common case.
 */
export async function cancelScan(): Promise<void> {
  if (!isTauri) return
  await invoke('cancel_scan')
}

export async function view(scanId: string, nodeId: number | null): Promise<ViewNode[]> {
  if (!isTauri) return demoView(nodeId)
  return invoke<ViewNode[]>('view', { scanId, nodeId })
}

export async function cleanup(
  scanId: string,
  minBytes: number,
  ageDays: number | null,
  newer: boolean,
): Promise<CleanupReport> {
  if (!isTauri) return { candidates: [], biggest: [], recent: [], total_candidate_bytes: 0 }
  return invoke<CleanupReport>('cleanup', { scanId, minBytes, ageDays, newer })
}

/**
 * Content-identical files in a finished scan. Read-only: the panel proposes, the collector
 * disposes. `budgetMs` bounds the pass so a click returns; the report says `partial` when it
 * ran out of time rather than pretending it found everything.
 */
export async function duplicates(
  scanId: string,
  minBytes: number,
  budgetMs?: number,
): Promise<DuplicateReport> {
  if (!isTauri) return demoDuplicates(minBytes)
  return invoke<DuplicateReport>('duplicates', { scanId, minBytes, budgetMs })
}

export async function trashStats(): Promise<TrashStats> {
  if (!isTauri) return { path: '', bytes: 0, items: 0, supported: false }
  return invoke<TrashStats>('trash_stats')
}

/**
 * What Time Machine is holding on the boot volume. The command takes no arguments: the
 * volume is fixed on the Rust side, so no UI-chosen path can ever ride along.
 */
export async function localSnapshots(): Promise<SnapshotReport> {
  if (!isTauri) return demoSnapshots()
  return invoke<SnapshotReport>('local_snapshots')
}

/**
 * Delete every local snapshot on the boot volume, as the user — never elevated. The
 * boolean is the whole payload; there is no path to send.
 */
export async function purgeLocalSnapshots(confirmed: boolean): Promise<PurgeReport> {
  if (!isTauri) return demoPurge()
  return invoke<PurgeReport>('purge_local_snapshots', { confirmed })
}

export async function emptyTrash(confirmed: boolean): Promise<EmptyReport> {
  if (!isTauri) return { freed: 0, removed: 0, failed: 0 }
  return invoke<EmptyReport>('empty_trash', { confirmed })
}

/** Drop view nodes whose paths have vanished since the scan. */
export async function revalidate(scanId: string, nodeId: number | null): Promise<FreshnessReport> {
  if (!isTauri) return { removed: 0, bytes: 0, files: 0, view: [], volume: null }
  return invoke<FreshnessReport>('revalidate', { scanId, nodeId })
}

/** Read the collector without mutating it. Also the cheapest IPC probe after a scan. */
export async function collected(scanId: string): Promise<CollectState> {
  if (!isTauri) return { total: 0, count: 0, ids: [], needs_confirmation: false }
  return invoke<CollectState>('collected', { scanId })
}

export async function collect(scanId: string, nodeId: number, add: boolean): Promise<CollectState> {
  if (!isTauri) return demoCollect(nodeId, add)
  return invoke<CollectState>('collect', { scanId, nodeId, add })
}

export async function deleteCollected(
  scanId: string,
  nodeId: number | null,
  permanent: boolean,
  confirmed: boolean,
): Promise<DeleteReport> {
  if (!isTauri) {
    const total = demoCollectorTotal()
    const collected = [...demoCollected]
    demoCollected.clear()
    // Drop the collected branches from the demo tree so the flower visibly shrinks.
    for (const id of collected) {
      for (const node of demoNodes) {
        node.children = node.children.filter((child) => demoNodes[child]?.node_id !== id)
      }
    }
    return {
      freed: total,
      removed: collected.length,
      failed: 0,
      mode: permanent ? 'permanent' : 'trash',
      outcomes: [],
      view: reroot(demoNodes, 0),
      bytes: demoNodes[0]?.bytes ?? 0,
      files: 0,
      volume: null,
    }
  }
  return invoke<DeleteReport>('delete_collected', { scanId, nodeId, permanent, confirmed })
}

export async function reveal(scanId: string, nodeId: number): Promise<void> {
  if (!isTauri) return
  await invoke('reveal', { scanId, nodeId })
}

export async function quicklook(scanId: string, nodeId: number): Promise<void> {
  if (!isTauri) return
  await invoke('quicklook', { scanId, nodeId })
}

/**
 * Measure the restricted areas of a stored scan with one OS consent dialog. The
 * command takes a scan id and nothing else: the paths are derived on the Rust side.
 */
export async function adminAuthorised(): Promise<boolean> {
  if (!isTauri) return false
  return invoke<boolean>('admin_authorised')
}

export async function scanRestricted(scanId: string): Promise<AdminScanReport> {
  if (!isTauri) {
    // The demo tree carries one restricted petal; "measuring" it is a no-op that keeps
    // the browser demo shape-complete without pretending a dialog appeared.
    return {
      scan_id: scanId,
      patched: 1,
      still_restricted: 0,
      bytes: demoNodes[0]?.bytes ?? 0,
      files: 0,
      view: demoView(0),
      volume: null,
    }
  }
  return invoke<AdminScanReport>('scan_restricted', { scanId })
}

/** Per-area progress while an administrator pass runs. */
export async function onAdminProgress(handler: (p: AdminProgress) => void): Promise<() => void> {
  if (!isTauri) return () => {}
  const unlisten = await listen<AdminProgress>('admin-progress', (event) => handler(event.payload))
  return unlisten
}

// ─── hidden space (C29) ──────────────────────────────────────────────────────────────

/**
 * The hidden-space breakdown of the current volume scan. The command takes a scan id
 * and nothing else: the webview cannot name a path.
 */
export async function hiddenSpace(scanId: string): Promise<HiddenSpaceReport> {
  if (!isTauri) return demoHidden()
  return invoke<HiddenSpaceReport>('hidden_space', { scanId })
}

/**
 * Open the Full Disk Access pane. The URL is fixed on the Rust side — this command has
 * no arguments, so it can never be aimed anywhere else.
 */
export async function openFdaPane(): Promise<void> {
  if (!isTauri) return
  await invoke('open_fda_pane')
}

// ─── favourites ──────────────────────────────────────────────────────────────────────
//
// The webview never writes favourite state itself: every change goes through these
// commands, and every path is resolved on the Rust side (by node id from the scan
// tree, or shape-validated when named directly).

/** Every favourite with an existence stat, plus the corrupt-store notice when set. */
export async function favourites(): Promise<FavouritesReport> {
  if (!isTauri) return demoFavourites()
  return invoke<FavouritesReport>('favourites')
}

/** Whether the viewed node and its children are favourites. */
export async function favouriteMarks(
  scanId: string,
  nodeId: number | null,
): Promise<FavouriteMarks> {
  if (!isTauri) return demoMarks(nodeId)
  return invoke<FavouriteMarks>('favourite_marks', { scanId, nodeId })
}

/** Star or unstar a node of a scan; `nodeId: null` is the tree root (the heading star). */
export async function favouriteNode(
  scanId: string,
  nodeId: number | null,
  add: boolean,
): Promise<string[]> {
  if (!isTauri) return demoToggleFavourite(nodeId, add)
  return invoke<string[]>('favourite_node', { scanId, nodeId, add })
}

/** Add or remove a favourite by path — the overview's unstar, which passes the store's
 *  own string back verbatim. */
export async function setFavourite(path: string, add: boolean): Promise<string[]> {
  if (!isTauri) return demoSetFavourite(path, add)
  return invoke<string[]>('set_favourite', { path, add })
}

// ─── demo data ────────────────────────────────────────────────────────────────────────

const GB = 1024 ** 3

interface DemoSpec {
  name: string
  size: number
  kind: ViewNode['kind']
  children?: DemoSpec[]
  spread?: number
}

const DEMO: DemoSpec[] = [
  {
    name: 'Library',
    size: 83.6 * GB,
    kind: 'dir',
    children: [
      { name: 'Caches', size: 28.4 * GB, kind: 'dir', spread: 26 },
      { name: 'Application Support', size: 22.1 * GB, kind: 'dir', spread: 18 },
      { name: 'Containers', size: 14.6 * GB, kind: 'dir', spread: 14 },
      { name: 'Developer', size: 11.2 * GB, kind: 'dir', spread: 9 },
      { name: 'Mail', size: 7.3 * GB, kind: 'dir', spread: 6 },
    ],
  },
  {
    name: 'Archive',
    size: 34 * GB,
    kind: 'dir',
    children: [
      { name: '2023', size: 18.2 * GB, kind: 'dir', spread: 12 },
      { name: '2022', size: 15.8 * GB, kind: 'dir', spread: 10 },
    ],
  },
  {
    name: 'Pictures',
    size: 22.3 * GB,
    kind: 'dir',
    children: [
      { name: 'Photos Library.photoslibrary', size: 19.4 * GB, kind: 'dir', spread: 8 },
      { name: 'Screenshots', size: 2.9 * GB, kind: 'dir', spread: 30 },
    ],
  },
  { name: 'Documents', size: 9.5 * GB, kind: 'dir', spread: 34 },
  { name: 'Desktop', size: 6.1 * GB, kind: 'dir', spread: 22 },
  { name: 'Trash', size: 5.1 * GB, kind: 'dir', spread: 16 },
  { name: 'Virtual Machines.vmwarevm', size: 4.4 * GB, kind: 'file' },
  { name: 'Downloads', size: 1.1 * GB, kind: 'dir', spread: 28 },
  { name: 'Private', size: 0.9 * GB, kind: 'restricted' },
]

let demoNodes: ViewNode[] = []
const demoCollected = new Set<number>()
const demoListeners = new Set<(branch: BranchDone) => void>()

function emitDemo(branch: BranchDone): void {
  for (const listener of demoListeners) listener(branch)
}

function buildDemo(): ViewNode[] {
  const nodes: ViewNode[] = []
  const push = (node: Omit<ViewNode, 'index'>): number => {
    const index = nodes.length
    nodes.push({ ...node, index })
    return index
  }

  const rootIndex = push({
    node_id: 0,
    name: 'you',
    bytes: 0,
    kind: 'dir',
    children: [],
    aggregated_count: 0,
    partial: false,
    missing: false,
  })

  let seed = 7
  const rand = (): number => {
    seed = (seed * 1103515245 + 12345) % 2 ** 31
    return seed / 2 ** 31
  }

  const add = (spec: DemoSpec, parent: number, id: number): number => {
    const index = push({
      node_id: id,
      name: spec.name,
      bytes: spec.size,
      kind: spec.kind,
      children: [],
      aggregated_count: 0,
      partial: spec.kind === 'restricted',
      missing: false,
    })
    nodes[parent]!.children.push(index)

    if (spec.spread) {
      let remaining = spec.size * 0.72
      for (let i = 0; i < spec.spread; i += 1) {
        const share = remaining * (0.35 + rand() * 0.4)
        remaining -= share
        if (share <= 0) break
        const child = push({
          node_id: id * 1000 + i + 1,
          name: `item-${i + 1}`,
          bytes: share,
          kind: i % 4 === 0 ? 'file' : 'dir',
          children: [],
          aggregated_count: 0,
          partial: false,
          missing: false,
        })
        nodes[index]!.children.push(child)
      }
    }

    for (const [i, child] of (spec.children ?? []).entries()) {
      add(child, index, id * 100 + i + 1)
    }
    return index
  }

  DEMO.forEach((spec, i) => add(spec, rootIndex, i + 1))

  nodes[rootIndex]!.bytes = nodes[rootIndex]!.children.reduce((sum, c) => sum + nodes[c]!.bytes, 0)
  return nodes
}

function demoScan(path: string, nonce: string): ScanResponse {
  demoNodes = buildDemo()
  // Report branches the way the real scanner does: one at a time, out of size order, and
  // stamped with the scan they belong to — the demo would otherwise hide the very bug the
  // nonce exists to stop.
  const order = [...DEMO].sort((a, b) => a.size - b.size)
  order.forEach((spec, i) => {
    setTimeout(
      () => emitDemo({ nonce, name: spec.name, bytes: spec.size, files: Math.round(spec.size / 9000) }),
      120 + i * 260,
    )
  })
  demoCollected.clear()
  const root = demoNodes[0]!
  return {
    scan_id: 'demo',
    name: path.split('/').filter(Boolean).pop() ?? 'you',
    path,
    bytes: root.bytes,
    files: 184_120,
    duration_ms: 1420,
    errors: 2,
    partial: true,
    view: reroot(demoNodes, 0),
    // The demo scans a synthetic folder, not a volume: no anchor, honestly.
    volume: null,
  }
}

function reroot(nodes: ViewNode[], rootIndex: number): ViewNode[] {
  const out: ViewNode[] = []
  const walk = (index: number): number => {
    const node = nodes[index]
    if (!node) return -1
    const next = out.length
    const clone: ViewNode = { ...node, index: next, children: [] }
    out.push(clone)
    clone.children = node.children.map(walk).filter((i) => i >= 0)
    return next
  }
  walk(rootIndex)
  return out
}

function demoView(nodeId: number | null): ViewNode[] {
  return reroot(demoNodes, nodeId ?? 0)
}

function demoCollect(nodeId: number, add: boolean): CollectState {
  if (add) demoCollected.add(nodeId)
  else demoCollected.delete(nodeId)
  const names = [...demoCollected].map(
    (id) => demoNodes.find((node) => node.node_id === id)?.name ?? '',
  )
  const topLevel = names.some((name) => DEMO.some((spec) => spec.name === name))
  return {
    total: demoCollectorTotal(),
    count: demoCollected.size,
    ids: [...demoCollected],
    needs_confirmation: topLevel || demoCollectorTotal() >= 5 * GB,
  }
}

function demoDuplicates(minBytes: number): DuplicateReport {
  const bytes = 128 * 1024 * 1024
  const empty: DuplicateReport = {
    groups: [],
    partial: false,
    files_hashed: 0,
    errors: 0,
    excluded_links: 0,
    duration_ms: 40,
  }
  if (minBytes > bytes) return empty

  return {
    ...empty,
    files_hashed: 2,
    groups: [
      {
        bytes_each: bytes,
        wasted_bytes: bytes,
        // Empty on purpose: the browser demo has no engine, so the panel shows its own
        // fallback wording rather than pretending to quote one.
        caveat: '',
        members: [
          { node: 1001, path: '/Users/you/Library/Caches/keynote.mov', keep: false },
          { node: 2001, path: '/Users/you/Archive/keynote copy.mov', keep: true },
        ],
      },
    ],
  }
}

function demoCollectorTotal(): number {
  return [...demoCollected].reduce((sum, id) => {
    const node = demoNodes.find((n) => n.node_id === id)
    return sum + (node?.bytes ?? 0)
  }, 0)
}

// Synthetic, and shaped exactly like `tmutil listlocalsnapshots /` answers: oldest first,
// header omitted, one name per snapshot.
const DEMO_SNAPSHOTS = [
  { name: 'com.apple.TimeMachine.2026-11-01-085943.local', stamp: '2026-11-01-085943' },
  { name: 'com.apple.TimeMachine.2026-11-02-080842.local', stamp: '2026-11-02-080842' },
]

function demoSnapshots(): SnapshotReport {
  return {
    supported: true,
    mount: '/',
    snapshots: DEMO_SNAPSHOTS.map((snapshot) => ({ ...snapshot })),
    error: null,
  }
}

/**
 * The browser demo has no `tmutil`, so there is nothing to delete and nothing that could
 * refuse. The report stays refused with everything still listed: a demo that claimed a
 * successful purge would be a lie about the machine it runs on.
 */
function demoPurge(): PurgeReport {
  return {
    purged: false,
    refused: true,
    message:
      'This is the browser demo: there is no tmutil here, so no snapshots were deleted. Run the app for the real answer.',
    freed_bytes: 0,
    remaining: DEMO_SNAPSHOTS.map((snapshot) => ({ ...snapshot })),
  }
}

/**
 * The browser demo's hidden-space report: the same shape and the same honest wording,
 * with synthetic figures. It claims no purge and invents no snapshot size — the demo
 * tree's numbers are not a volume's.
 */
function demoHidden(): HiddenSpaceReport {
  return {
    mount: '/',
    used_bytes: 994_662_584_320,
    tree_bytes: 830_000_000_000,
    hidden_bytes: 164_662_584_320,
    other_volumes: [
      { name: 'Preboot', bytes: 9_041_682_432 },
      { name: 'Recovery', bytes: 1_307_287_552 },
      { name: 'VM', bytes: 4_295_032_832 },
    ],
    snapshot_count: DEMO_SNAPSHOTS.length,
    snapshot_note:
      'macOS does not report how much each holds; any they hold is part of the still-hidden figure',
    restricted_roots: [{ path: '/Users/you/Private', note: 'permission denied' }],
    purgeable_bytes: 164_000_000_000,
    purgeable_note:
      'up to this much: snapshots, caches and offloaded content macOS may free on its own',
    still_hidden_bytes: 150_018_681_504,
    skipped_mounts: ['/Volumes/com.apple.TimeMachine.localsnapshots'],
    measured_at: Math.floor(Date.now() / 1000),
    negative_note: null,
  }
}

// Synthetic favourites for the browser demo: one entry that "exists" (Downloads is in
// the demo tree, so the legend star lights for it too) and one that does not. The list
// lives in memory only — the demo never claims the persistence the real store has.
const demoFavouritePaths: { path: string; exists: boolean }[] = [
  { path: '/Users/you/Downloads', exists: true },
  { path: '/Users/you/Archived 2019', exists: false },
]

function demoFavourites(): FavouritesReport {
  return {
    favourites: demoFavouritePaths.map((entry) => ({ ...entry })),
    notice: null,
  }
}

/** The demo-tree path a node id stands for; `null` is the demo root. */
function demoNodePath(nodeId: number | null): string {
  if (nodeId === null) return '/Users/you'
  const node = demoNodes.find((candidate) => candidate.node_id === nodeId)
  return node ? `/Users/you/${node.name}` : ''
}

function demoToggleFavourite(nodeId: number | null, add: boolean): string[] {
  const path = demoNodePath(nodeId)
  if (path !== '') {
    const index = demoFavouritePaths.findIndex((entry) => entry.path === path)
    if (add && index === -1) demoFavouritePaths.push({ path, exists: true })
    if (!add && index !== -1) demoFavouritePaths.splice(index, 1)
  }
  return demoFavouritePaths.map((entry) => entry.path)
}

function demoSetFavourite(path: string, add: boolean): string[] {
  const index = demoFavouritePaths.findIndex((entry) => entry.path === path)
  if (add && index === -1) demoFavouritePaths.push({ path, exists: true })
  if (!add && index !== -1) demoFavouritePaths.splice(index, 1)
  return demoFavouritePaths.map((entry) => entry.path)
}

function demoMarks(nodeId: number | null): FavouriteMarks {
  const node = nodeId === null ? demoNodes[0] : demoNodes.find((candidate) => candidate.node_id === nodeId)
  if (!node) return { self_: false, children: [] }
  const paths = new Set(demoFavouritePaths.map((entry) => entry.path))
  return {
    self_: paths.has(demoNodePath(nodeId)),
    children: node.children
      .map((index) => demoNodes[index])
      .filter((child): child is ViewNode => child !== undefined && child.kind === 'dir')
      .filter((child) => paths.has(`/Users/you/${child.name}`))
      .map((child) => child.node_id)
      .filter((id): id is number => id !== null),
  }
}
