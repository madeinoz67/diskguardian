export interface ViewNode {
  index: number
  node_id: number | null
  name: string
  bytes: number
  kind: 'dir' | 'file' | 'symlink' | 'restricted' | 'aggregate' | 'measuring'
  children: number[]
  aggregated_count: number
  partial: boolean
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

export interface TrashStats {
  path: string
  bytes: number
  items: number
  supported: boolean
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
