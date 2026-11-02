// The pure half of administrator measurement: the sentences the status line shows.
// The impure half (the bridge call, the OS dialog) lives in bridge.ts and main.ts.

import { formatBytes } from './format'

/** Shown when the OS dialog was cancelled: the tree is exactly as it was. */
export const ADMIN_REFUSED_STATUS =
  'Administrator access was declined — restricted areas stay unmeasured'

/** The status line after a completed pass: what came back, what is still purple. */
export function adminStatus(recovered: number, stillRestricted: number): string {
  const areas =
    stillRestricted > 0
      ? ` · ${stillRestricted} area${stillRestricted === 1 ? '' : 's'} still restricted`
      : ''
  return `Measured as administrator · recovered ${formatBytes(recovered)}${areas}`
}

/**
 * The affordance exists only where the runner exists. The user agent is the cheapest
 * honest detector for now; a proper platform command is deferred with the Linux and
 * Windows runners.
 */
export function isMacPlatform(userAgent: string): boolean {
  return /Mac/i.test(userAgent)
}
