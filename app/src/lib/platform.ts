/** True inside the Tauri webview; false in a plain browser (mock mode). */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window

const ua = typeof navigator !== "undefined" ? navigator.userAgent : ""
export const isMac = /Mac OS X|Macintosh/.test(ua)
export const isWindows = /Windows/.test(ua)

type UaData = { getHighEntropyValues(hints: string[]): Promise<{ platformVersion?: string }> }

/**
 * The native material behind the window (configured in tauri.*.conf.json):
 * Mica on Windows 11, vibrancy on macOS. Windows 10 and Linux have none, so
 * the app stays solid there.
 */
export async function detectMaterial(): Promise<"mica" | "vibrancy" | null> {
  if (!isTauri) return null
  if (isMac) return "vibrancy"
  if (!isWindows) return null
  const uad = (navigator as Navigator & { userAgentData?: UaData }).userAgentData
  const v = await uad?.getHighEntropyValues(["platformVersion"]).catch(() => undefined)
  // Windows 11 reports platform version 13 and up.
  return Number(v?.platformVersion?.split(".")[0]) >= 13 ? "mica" : null
}

/** Primary modifier label for shortcuts: ⌘ on macOS, Ctrl elsewhere. */
export const modKey = isMac ? "⌘" : "Ctrl"
