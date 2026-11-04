import { describe, expect, test } from 'bun:test'
import { isWindowsPath, pathPlaceholder } from './platform'

describe('isWindowsPath', () => {
  test('recognises drive-letter paths in either separator', () => {
    expect(isWindowsPath('C:\\Users\\seaton')).toBe(true)
    expect(isWindowsPath('c:/temp')).toBe(true)
  })

  test('recognises UNC shares', () => {
    expect(isWindowsPath('\\\\server\\share')).toBe(true)
  })

  test('does not mistake POSIX paths for Windows ones', () => {
    expect(isWindowsPath('/tmp')).toBe(false)
    expect(isWindowsPath('/Users/seaton')).toBe(false)
    expect(isWindowsPath('~/Documents')).toBe(false)
    expect(isWindowsPath('')).toBe(false)
  })

  test('a bare drive with no separator is drive-relative, not a root', () => {
    // `C:temp` means "temp under the current directory of drive C", which is not what the
    // home path ever is. Requiring the separator keeps the detector honest.
    expect(isWindowsPath('C:temp')).toBe(false)
  })
})

describe('pathPlaceholder', () => {
  test('offers Windows syntax on Windows, and never promises `~` or /tmp', () => {
    const hint = pathPlaceholder('C:\\Users\\seaton')
    expect(hint).toContain('C:\\temp')
    expect(hint).toContain('%USERPROFILE%')
    expect(hint).not.toContain('/tmp')
    expect(hint).not.toContain('~')
  })

  test('offers POSIX syntax everywhere else', () => {
    const hint = pathPlaceholder('/Users/seaton')
    expect(hint).toContain('/tmp')
    expect(hint).toContain('~/Documents')
    expect(hint).not.toContain('%USERPROFILE%')
  })

  test('an unresolved home degrades to the POSIX wording rather than a broken hint', () => {
    expect(pathPlaceholder('')).toContain('/tmp')
  })
})
