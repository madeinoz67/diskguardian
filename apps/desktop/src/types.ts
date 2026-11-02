export interface ViewNode {
  index: number
  node_id: number | null
  name: string
  bytes: number
  kind: 'dir' | 'file' | 'symlink' | 'restricted' | 'aggregate' | 'measuring'
  children: number[]
  aggregated_count: number
  partial: boolean
  /** Gone from disk since the scan: drawn struck through, worth zero. */
  missing: boolean
}

export interface ScanResponse {
  scan_id: string
  name: string
  path: string
  bytes: number
  files: number
  duration_ms: number
  errors: number
  partial: boolean
  view: ViewNode[]
}

export interface CollectState {
  total: number
  count: number
  ids: number[]
  /** Decided by Rust: a top-level folder asks whatever its size. */
  needs_confirmation: boolean
}

export interface CleanupItem {
  node: number
  path: string
  bytes: number
  newest_modified: number | null
  kind: string | null
  reason: string
}

export interface CleanupReport {
  candidates: CleanupItem[]
  biggest: CleanupItem[]
  recent: CleanupItem[]
  total_candidate_bytes: number
}

export interface DuplicateMember {
  node: number
  path: string
  /** The newest copy in the group: the one the panel does not offer to delete. */
  keep: boolean
}

export interface DuplicateGroup {
  bytes_each: number
  /** `bytes_each * (members - 1)` — a ceiling, not a promise. See `caveat`. */
  wasted_bytes: number
  members: DuplicateMember[]
  /** The engine's own sentence about APFS clones. Shown as visible text, always. */
  caveat: string
}

export interface DuplicateReport {
  groups: DuplicateGroup[]
  /** The pass ran out of time or candidates: more duplicates may exist unexamined. */
  partial: boolean
  files_hashed: number
  errors: number
  excluded_links: number
  duration_ms: number
}

export interface TrashStats {
  path: string
  bytes: number
  items: number
  supported: boolean
}

/** One APFS local snapshot, named the way `tmutil` names it. */
export interface LocalSnapshot {
  name: string
  /** `2026-11-01-085943` — the stamp `tmutil deletelocalsnapshots` accepts. */
  stamp: string
}

export interface SnapshotReport {
  /** False where the platform has no local Time Machine snapshots to speak of. */
  supported: boolean
  mount: string
  snapshots: LocalSnapshot[]
  /** Why the listing failed, when it did. A failed listing is never shown as "none". */
  error: string | null
}

export interface PurgeReport {
  purged: boolean
  /** The OS refused for want of administrator rights — a different answer for the user. */
  refused: boolean
  /** What to tell the user, in full sentences, whatever happened. */
  message: string
  /** Measured free-space change across the attempt. Signed: others may take space too. */
  freed_bytes: number
  /** The snapshots that are still there afterwards. */
  remaining: LocalSnapshot[]
}

export interface EmptyReport {
  freed: number
  removed: number
  failed: number
}

export interface FreshnessReport {
  removed: number
  bytes: number
  files: number
  view: ViewNode[]
}

export interface BranchDone {
  /**
   * The scan this report belongs to. `scan-progress` is one global event name, so without
   * this the next scan's listener accepts the previous scan's branches and paints them at
   * the wrong widths.
   */
  nonce: string
  name: string
  bytes: number
  files: number
}

export interface VolumeInfo {
  name: string
  mount: string
  total_bytes: number
  free_bytes: number
}

export interface DeleteReport {
  freed: number
  removed: number
  failed: number
  mode: 'trash' | 'permanent'
  outcomes: { path: string; ok: boolean; bytes: number; error: string | null }[]
  view: ViewNode[]
  bytes: number
  files: number
}

/** What an administrator measurement pass did to the stored tree. */
export interface AdminScanReport {
  /** Which scan this was grafted into. A pass can outlive the scan that asked for it. */
  scan_id: string
  patched: number
  still_restricted: number
  bytes: number
  files: number
  view: ViewNode[]
}

export interface AdminProgress {
  measured: number
  total: number
}
