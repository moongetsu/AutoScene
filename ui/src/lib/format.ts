const K = 1024

export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B"
  const sizes = ["B", "KB", "MB", "GB", "TB"]
  const i = Math.min(Math.floor(Math.log(bytes) / Math.log(K)), sizes.length - 1)
  return `${parseFloat((bytes / K ** i).toFixed(i === 0 ? 0 : 1))} ${sizes[i]}`
}

export function formatTransferRate(bytesPerSecond: number): string {
  if (!Number.isFinite(bytesPerSecond) || bytesPerSecond <= 0) return "—"
  if (bytesPerSecond >= K ** 2) return `${(bytesPerSecond / K ** 2).toFixed(1)} MB/s`
  if (bytesPerSecond >= K) return `${(bytesPerSecond / K).toFixed(0)} KB/s`
  return `${Math.round(bytesPerSecond)} B/s`
}

export function formatEta(seconds: number | null): string {
  if (seconds === null || !Number.isFinite(seconds)) return "—"
  const s = Math.max(0, Math.ceil(seconds))
  if (s <= 0) return "finishing"
  if (s < 60) return `${s}s left`
  const m = Math.floor(s / 60)
  const r = s % 60
  if (m < 60) return `${m}:${String(r).padStart(2, "0")} left`
  const h = Math.floor(m / 60)
  return `${h}:${String(m % 60).padStart(2, "0")}:${String(r).padStart(2, "0")} left`
}

/** Filename from video_path, with project.name as fallback. */
export function mediaLabel(videoPath: string, fallbackName?: string): string {
  const base = videoPath.split(/[/\\]/).pop()?.trim()
  if (base) return base
  return fallbackName?.trim() || "Untitled media"
}

export function formatRelativeTime(iso: string, now = Date.now()): string {
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return "—"
  const delta = Math.round((now - t) / 1000)
  if (delta < 45) return "just now"
  if (delta < 3600) {
    const m = Math.floor(delta / 60)
    return `${m}m ago`
  }
  if (delta < 86400) {
    const h = Math.floor(delta / 3600)
    return `${h}h ago`
  }
  if (delta < 86400 * 14) {
    const d = Math.floor(delta / 86400)
    return `${d}d ago`
  }
  return new Date(t).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
  })
}

export function formatAbsoluteStamp(iso: string): string {
  const t = Date.parse(iso)
  if (!Number.isFinite(t)) return "—"
  return new Date(t).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    year: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  })
}
