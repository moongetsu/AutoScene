import { api } from "@/lib/api"

export function isTauri(): boolean {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window
}

export function apiBase(): string {
  if (isTauri()) return "http://127.0.0.1:8741"
  return ""
}

/** Mirror server open-folder allowlist before native reveal. */
function revealPathAllowed(path: string): boolean {
  const norm = path.replace(/\\/g, "/").toLowerCase()
  return [
    "/downloads/",
    "/videos/",
    "/documents/",
    "/desktop/",
    "/.autoscene/",
    "/autoscene/",
  ].some((m) => norm.includes(m))
}

export async function revealInFolder(path: string): Promise<void> {
  // Always enforce server allowlist first.
  await api.openFolder(path)
  if (isTauri() && revealPathAllowed(path)) {
    try {
      const { revealItemInDir } = await import("@tauri-apps/plugin-opener")
      await revealItemInDir(path)
    } catch {
      /* explorer already opened via API */
    }
  }
}

export async function pickMergedVideoPath(defaultName: string): Promise<string | null> {
  if (isTauri()) {
    const { save } = await import("@tauri-apps/plugin-dialog")
    const result = await save({
      defaultPath: defaultName,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv"] }],
    })
    return result ?? null
  }
  const value = window.prompt("Save merged video as (full path):", defaultName)
  if (!value?.trim()) return null
  return value.trim()
}

/** Pick an existing local video path (no copy into uploads). */
export async function pickLocalVideoPath(): Promise<string | null> {
  if (isTauri()) {
    const { open } = await import("@tauri-apps/plugin-dialog")
    const result = await open({
      multiple: false,
      filters: [{ name: "Video", extensions: ["mp4", "mov", "mkv", "avi", "webm"] }],
    })
    if (typeof result === "string") return result
    if (Array.isArray(result) && result[0]) return result[0]
    return null
  }
  const value = window.prompt("Full path to local video file:")
  if (!value?.trim()) return null
  return value.trim()
}
