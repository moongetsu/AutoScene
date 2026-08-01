import { toast } from "sonner"
import { formatTimecode } from "@/lib/timecode"

export async function copyText(text: string, label = "Copied") {
  try {
    await navigator.clipboard.writeText(text)
    toast.success(label)
    return true
  } catch {
    toast.error("Clipboard unavailable")
    return false
  }
}

export type CopyFormat = "timecode" | "csv" | "seconds"

export type CopyOptions = {
  format: CopyFormat
  /** Source frame rate; 0/undefined falls back to HH:MM:SS.mmm. */
  fps?: number | null
  dropFrame?: boolean
  /** e.g. "ep 1 cut" → rows named "ep 1 cut 01", "ep 1 cut 02", … */
  prefix?: string
  startIndex?: number
}

/** Sequential clip name for a row, matching the DaVinci naming convention. */
export function clipName(prefix: string | undefined, index: number): string {
  const n = String(index).padStart(2, "0")
  const p = (prefix ?? "").trim()
  return p ? `${p} ${n}` : `Clip ${n}`
}

/**
 * Segment list for pasting into an edit suite.
 *
 * "timecode" emits real HH:MM:SS:FF at the source frame rate (the previous
 * implementation printed raw decimal seconds, which no NLE accepts).
 */
export function formatSegmentsForCopy(
  segments: { start_sec: number; end_sec: number }[],
  label: string,
  opts: CopyOptions = { format: "timecode" },
): string {
  const { format, fps, dropFrame, prefix, startIndex = 1 } = opts
  const rate = format === "seconds" ? 0 : (fps ?? 0)
  const fmt = (sec: number) =>
    format === "seconds" ? `${sec.toFixed(2)}s` : formatTimecode(sec, rate, dropFrame)

  const rows = segments.map((s, i) => {
    const name = clipName(prefix, startIndex + i)
    return [name, fmt(s.start_sec), fmt(s.end_sec), fmt(s.end_sec - s.start_sec)]
  })

  if (format === "csv") {
    const esc = (v: string) => (/[",]/.test(v) ? `"${v.replace(/"/g, '""')}"` : v)
    return [
      "name,start,end,duration",
      ...rows.map((r) => r.map(esc).join(",")),
    ].join("\n")
  }

  return [
    `# ${label}`,
    "clip\tstart\tend\tduration",
    ...rows.map((r) => r.join("\t")),
  ].join("\n")
}

export function formatClipsForCopy(clips: string[]): string {
  return clips.join("\n")
}
