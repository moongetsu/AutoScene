/** True inside the Tauri webview; false in a plain browser (mock mode). */
export const isTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window

const ua = typeof navigator !== "undefined" ? navigator.userAgent : ""
export const isMac = /Mac OS X|Macintosh/.test(ua)
export const isWindows = /Windows/.test(ua)

/** Primary modifier label for shortcuts: ⌘ on macOS, Ctrl elsewhere. */
export const modKey = isMac ? "⌘" : "Ctrl"
