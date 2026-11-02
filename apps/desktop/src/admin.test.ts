import { describe, expect, test } from 'bun:test'
import { ADMIN_REFUSED_STATUS, adminStatus, isMacPlatform } from './admin'

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

  test('macOS is detected from the user agent, other platforms are left alone', () => {
    expect(isMacPlatform('Mozilla/5.0 (Macintosh; Intel Mac OS X 14_0) AppleWebKit/605.1.15')).toBe(
      true,
    )
    expect(isMacPlatform('Mozilla/5.0 (Windows NT 10.0; Win64; x64)')).toBe(false)
    expect(isMacPlatform('Mozilla/5.0 (X11; Linux x86_64)')).toBe(false)
  })
})
