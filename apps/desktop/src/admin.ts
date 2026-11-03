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
 * The affordance exists only where the runner exists: macOS (osascript dialog) and
 * Windows (a `runas` re-exec of the app binary, design §3.7.2). Linux has no runner yet
 * and its launcher returns `Unsupported`, so offering the button there would promise a
 * measurement that cannot happen.
 *
 * This is deliberately a question about runners, not about the OS name. It was
 * `isMacPlatform` when macOS was the only runner; the Windows runner was added in Rust
 * and this predicate was not widened with it, which left the shipped Windows build with
 * no way to reach the elevation path at all. The user agent stays the cheapest honest
 * detector until a platform command exists; the tests pin both platforms' real strings.
 */
export function hasAdminRunner(userAgent: string): boolean {
  return /Macintosh|Windows NT/i.test(userAgent)
}
