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

// The interface is English; dates follow it ("13 Oct, 22:13") instead of mixing languages.
const dateFmt = new Intl.DateTimeFormat("en-GB", { day: "numeric", month: "short", hour: "2-digit", minute: "2-digit" })

/** Unix ms → "13 Oct, 19:42". */
export function formatDateTime(ms: number): string {
  return dateFmt.format(new Date(ms))
}

/** 0.4 → "40%". */
export function formatPercent(v: number | null | undefined): string {
  return v == null ? "" : `${Math.round(v * 100)}%`
}

/** "H.264 · Mobile (CapCut, Alight Motion)" → "H.264 · Mobile": an export format's name without what its brackets add. */
export function shortFormatName(name: string): string {
  return name.replace(/\s*\([^)]*\)\s*$/, "") || name
}

/** Remaining time for progress lines: "about 3 min left". */
export function formatEta(ms: number | null | undefined): string {
  if (ms == null || !Number.isFinite(ms)) return ""
  const s = Math.round(ms / 1000)
  if (s < 5) return "almost done"
  if (s < 60) return `${s} s left`
  const m = Math.round(s / 60)
  return m < 60 ? `about ${m} min left` : `about ${Math.round(m / 6) / 10} h left`
}
