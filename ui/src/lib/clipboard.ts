import { toast } from "sonner"

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

export function formatSegmentsForCopy(
  segments: { start_sec: number; end_sec: number }[],
  label: string,
): string {
  const lines = segments.map(
    (s, i) =>
      `${String(i + 1).padStart(2, "0")}\t${s.start_sec.toFixed(2)}s\t${s.end_sec.toFixed(2)}s\t${(s.end_sec - s.start_sec).toFixed(2)}s`,
  )
  return [`# ${label}`, "clip\tstart\tend\tduration", ...lines].join("\n")
}

export function formatClipsForCopy(clips: string[]): string {
  return clips.join("\n")
}
