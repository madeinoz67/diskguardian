// The landing screen: choose what to scan. Pure helpers first, so the gauge rules are
// testable without a window.

import type { VolumeInfo } from './types'
import { formatBytes } from './format'

export type Tone = 'healthy' | 'tight' | 'critical'

export interface Gauge {
  /** 0..1, how much of the volume is in use. */
  usedRatio: number
  tone: Tone
  freeLabel: string
}

/**
 * DaisyDisk's rule of thumb, kept because it is honest: green means at least half the disk
 * is empty, amber means it is getting tight, red means act now.
 */
export function gaugeFor(volume: Pick<VolumeInfo, 'total_bytes' | 'free_bytes'>): Gauge {
  const total = volume.total_bytes
  const free = Math.min(Math.max(volume.free_bytes, 0), Math.max(total, 0))
  const usedRatio = total > 0 ? 1 - free / total : 0
  const freeRatio = total > 0 ? free / total : 0

  return {
    usedRatio: Math.min(Math.max(usedRatio, 0), 1),
    tone: freeRatio >= 0.5 ? 'healthy' : freeRatio >= 0.2 ? 'tight' : 'critical',
    freeLabel: `${formatBytes(free)} free`,
  }
}

export function toneColor(tone: Tone): string {
  switch (tone) {
    case 'healthy':
      return '#30d158'
    case 'tight':
      return '#ffd60a'
    default:
      return '#ff453a'
  }
}

export function basename(path: string): string {
  const parts = path.split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] ?? path
}
