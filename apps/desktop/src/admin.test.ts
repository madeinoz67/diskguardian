import { describe, expect, test } from 'bun:test'
import { ADMIN_REFUSED_STATUS, adminStatus, hasAdminRunner } from './admin'

const MB = 1024 ** 2
const GB = 1024 ** 3

describe('administrator measurement status', () => {
  test('names what was recovered and what is still restricted', () => {
    expect(adminStatus(3.5 * GB, 0)).toBe('Measured as administrator · recovered 3.5 GB')
    expect(adminStatus(512 * MB, 1)).toBe(
      'Measured as administrator · recovered 512 MB · 1 area still restricted',
    )
    expect(adminStatus(0, 7)).toBe(
      'Measured as administrator · recovered 0 bytes · 7 areas still restricted',
    )
  })

  test('the refusal says the tree stays as it was', () => {
    expect(ADMIN_REFUSED_STATUS).toContain('declined')
    expect(ADMIN_REFUSED_STATUS).toContain('unmeasured')
  })

  // This predicate used to be macOS-only, and the Windows runner was added in Rust
  // without it: on the 0.1.1 Windows build a partial scan offered no Measure button, so
  // the whole self-elevation path was unreachable in the shipped product. The predicate
  // answers "does a runner exist here", which is the question the gate actually asks.
  test('the affordance follows the runner, which macOS and Windows both ship', () => {
    expect(hasAdminRunner('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15')).toBe(
      true,
    )
    // A real WebView2 user agent, not a hand-simplified one.
    expect(
      hasAdminRunner(
        'Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36 Edg/120.0.0.0',
      ),
    ).toBe(true)
    // Linux has no runner yet (the launcher returns Unsupported), so the affordance must
    // not appear and promise a measurement that cannot happen.
    expect(hasAdminRunner('Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/605.1.15')).toBe(false)
  })
})
