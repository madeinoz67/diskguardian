import { describe, expect, test } from 'bun:test'
import { basename, gaugeFor, toneColor } from './overview'

const GB = 1024 ** 3

describe('volume gauges', () => {
  test('half empty is still healthy', () => {
    const gauge = gaugeFor({ total_bytes: 100 * GB, free_bytes: 50 * GB })
    expect(gauge.tone).toBe('healthy')
    expect(gauge.usedRatio).toBeCloseTo(0.5, 6)
  })

  test('a quarter free is tight, a tenth is critical', () => {
    expect(gaugeFor({ total_bytes: 100 * GB, free_bytes: 25 * GB }).tone).toBe('tight')
    expect(gaugeFor({ total_bytes: 100 * GB, free_bytes: 10 * GB }).tone).toBe('critical')
  })

  test('nonsense numbers are clamped rather than drawn off the bar', () => {
    const over = gaugeFor({ total_bytes: 10 * GB, free_bytes: 20 * GB })
    expect(over.usedRatio).toBe(0)
    const zero = gaugeFor({ total_bytes: 0, free_bytes: 0 })
    expect(zero.usedRatio).toBe(0)
    expect(zero.tone).toBe('critical')
  })

  test('the free label is human readable', () => {
    expect(gaugeFor({ total_bytes: 100 * GB, free_bytes: 42 * GB }).freeLabel).toBe('42 GB free')
  })

  test('tones get distinct colours', () => {
    const colours = new Set(['healthy', 'tight', 'critical'].map((t) => toneColor(t as never)))
    expect(colours.size).toBe(3)
  })
})

describe('basename', () => {
  test('handles both separators, because Windows exists', () => {
    expect(basename('/Users/you')).toBe('you')
    expect(basename('C:\\Users\\you')).toBe('you')
    expect(basename('/')).toBe('/')
  })
})
