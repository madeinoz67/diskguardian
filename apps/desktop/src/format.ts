const UNITS = ['bytes', 'KB', 'MB', 'GB', 'TB', 'PB']

/** Human size, DaisyDisk-style: at most one decimal below 100, none above. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '0 bytes'
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < UNITS.length - 1) {
    value /= 1024
    unit += 1
  }
  if (unit === 0) return `${Math.round(value)} bytes`
  const text = value >= 100 ? Math.round(value).toString() : value.toFixed(1).replace(/\.0$/, '')
  return `${text} ${UNITS[unit]}`
}

export function formatCount(n: number): string {
  return n.toLocaleString('en-AU')
}
