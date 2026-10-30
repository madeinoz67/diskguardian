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
