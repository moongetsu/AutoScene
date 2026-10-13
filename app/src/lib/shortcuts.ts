import { modKey } from "./platform"

export type ShortcutId = "palette" | "activity" | "sidebar" | "settings" | "library"

export type Shortcut = {
  id: ShortcutId
  label: string
  /** Display keys, e.g. ["Ctrl", "K"]. */
  keys: string[]
  /** Physical key (KeyboardEvent.key, lower-case) pressed with the primary modifier. */
  key: string
}

/** Global shortcuts. On macOS Tauri these are native menu accelerators; elsewhere JS handles them. */
export const SHORTCUTS: Shortcut[] = [
  { id: "palette", label: "Command palette", keys: [modKey, "K"], key: "k" },
  { id: "activity", label: "Show activity", keys: [modKey, "J"], key: "j" },
  { id: "sidebar", label: "Toggle sidebar", keys: [modKey, "B"], key: "b" },
  { id: "settings", label: "Settings", keys: [modKey, ","], key: "," },
  { id: "library", label: "Library", keys: [modKey, "1"], key: "1" },
]

export function shortcutKeys(id: ShortcutId): string[] {
  return SHORTCUTS.find((s) => s.id === id)?.keys ?? []
}

/** Match a keydown against the registry (primary modifier = Ctrl, or ⌘ on macOS). */
export function matchShortcut(e: KeyboardEvent): ShortcutId | null {
  const mod = modKey === "⌘" ? e.metaKey : e.ctrlKey
  if (!mod || e.altKey || e.shiftKey) return null
  const k = e.key.toLowerCase()
  return SHORTCUTS.find((s) => s.key === k)?.id ?? null
}
