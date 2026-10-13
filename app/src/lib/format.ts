const UNITS = ["B", "KB", "MB", "GB", "TB"] as const

export function formatBytes(bytes: number, digits = 1): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B"
  const i = Math.min(UNITS.length - 1, Math.floor(Math.log(bytes) / Math.log(1024)))
  const v = bytes / 1024 ** i
  return `${v.toFixed(i === 0 ? 0 : digits)} ${UNITS[i]}`
}

/** 3725.4 → "1:02:05"; 65 → "1:05". */
export function formatDuration(sec: number): string {
  if (!Number.isFinite(sec) || sec < 0) return "–"
  const s = Math.round(sec)
  const h = Math.floor(s / 3600)
  const m = Math.floor((s % 3600) / 60)
  const r = s % 60
  const mm = h > 0 ? String(m).padStart(2, "0") : String(m)
  return h > 0 ? `${h}:${mm}:${String(r).padStart(2, "0")}` : `${mm}:${String(r).padStart(2, "0")}`
}
