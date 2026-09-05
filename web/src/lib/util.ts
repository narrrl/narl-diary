const pad = (n: number) => String(n).padStart(2, '0')

export function toDate(seconds: number): Date {
  return new Date(seconds * 1000)
}

/** `2026-09-05` — the diary is organised by day, so this is the primary label. */
export function formatDay(seconds: number): string {
  const d = toDate(seconds)
  return `${d.getFullYear()}-${pad(d.getMonth() + 1)}-${pad(d.getDate())}`
}

export function formatTime(seconds: number): string {
  const d = toDate(seconds)
  return `${pad(d.getHours())}:${pad(d.getMinutes())}`
}

export function formatStamp(seconds: number): string {
  return `${formatDay(seconds)} ${formatTime(seconds)}`
}

export function formatWeekday(seconds: number): string {
  return toDate(seconds).toLocaleDateString(undefined, { weekday: 'short' }).toLowerCase()
}

export function relative(seconds: number): string {
  const delta = Math.floor(Date.now() / 1000) - seconds
  if (delta < 60) return 'just now'
  if (delta < 3600) return `${Math.floor(delta / 60)}m ago`
  if (delta < 86400) return `${Math.floor(delta / 3600)}h ago`
  if (delta < 86400 * 30) return `${Math.floor(delta / 86400)}d ago`
  return formatDay(seconds)
}

export function formatBytes(bytes: number): string {
  const units = ['B', 'K', 'M', 'G']
  let value = bytes
  let unit = 0
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit++
  }
  return `${value < 10 && unit > 0 ? value.toFixed(1) : Math.round(value)}${units[unit]}`
}

/** Parse the `YYYY-MM-DD` argument of `:date` into a Unix timestamp at noon. */
export function parseDay(input: string): number | null {
  const match = /^(\d{4})-(\d{2})-(\d{2})$/.exec(input.trim())
  if (!match) return null
  const [, y, m, d] = match
  const date = new Date(Number(y), Number(m) - 1, Number(d), 12, 0, 0)
  return Number.isNaN(date.getTime()) ? null : Math.floor(date.getTime() / 1000)
}

/**
 * True on phones and tablets, where modal editing would be a burden. Both
 * checks are needed: a touchscreen laptop reports touch points but still has a
 * fine primary pointer, and headless browsers report `hover: none`.
 */
export const isTouchDevice =
  typeof window !== 'undefined' &&
  navigator.maxTouchPoints > 0 &&
  window.matchMedia('(pointer: coarse)').matches
