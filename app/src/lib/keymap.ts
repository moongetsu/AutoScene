import { useEffect } from "react"
import { toast } from "sonner"
import type { AppSettings, Shortcuts } from "@/bindings"
import { commands, errorMessage } from "./ipc"
import { isMac, isTauri, modKey } from "./platform"
import { qk, queryClient, useSettings } from "./queries"
import {
  ALL_SHORTCUTS,
  type Binding,
  DEFAULT_KEYMAP,
  type Keymap,
  PRESETS,
  type PresetId,
  type Shortcut,
  type ShortcutRef,
  bindingCaps,
  bindingsOf,
  getKeymap,
  presetBindings,
  setKeymap,
  shortcutOf,
  slotRef,
  subscribeKeymap,
} from "./shortcuts"

/**
 * Changing the keys (Settings → Shortcuts): one key per slot, taken from
 * whatever else in reach had it, so a key press never means two things in
 * one place; and keeping them in the library's settings.
 */

export type Use = { s: Shortcut; slot: number }

/** What a slot is called where one row does a few things ("Next frame"). */
export const slotLabel = ({ s, slot }: Use) => s.slots[slot].label ?? s.label

const sig = (b: Binding) => `${b.mod ? "mod+" : ""}${b.shift ? "shift+" : ""}${b.alt ? "alt+" : ""}${b.key}`

/** Whether two key presses are one to `matches` (a digit without Shift takes Shift too). */
export function sameBinding(a: Binding, b: Binding): boolean {
  if (a.key !== b.key || !a.mod !== !b.mod || !a.alt !== !b.alt) return false
  return !a.shift === !b.shift || (/^[0-9]$/.test(a.key) && (!a.shift || !b.shift))
}

/** Shortcuts whose keys could meet: on the same page, or where one of them works everywhere. */
const inReach = (a: Shortcut, b: Shortcut) => a.scope === "global" || b.scope === "global" || a.scope === b.scope

/** The other slots in reach of slot `slot` of `s` that `b` already does. */
export function usesOf(b: Binding, s: Shortcut, slot: number, km: Keymap = getKeymap()): Use[] {
  return ALL_SHORTCUTS.filter((o) => inReach(s, o)).flatMap((o) =>
    o.slots
      .map((_, j) => ({ s: o, slot: j }))
      .filter((u) => (u.s !== s || u.slot !== slot) && bindingsOf(u.s, u.slot, km).some((x) => sameBinding(x, b))),
  )
}

/** Copy, cut, paste, undo, redo: the text fields' own keys. */
const editing = (b: Binding) => Boolean(b.mod) && !b.alt && (b.shift ? b.key === "z" : ["c", "v", "x", "z", "y"].includes(b.key))

/**
 * What Ctrl (⌘) does with the caret in a field: by words, to the line's ends,
 * deleting words. The palette and the activity drawer answer in fields too.
 */
const CARET_KEYS = ["arrowleft", "arrowright", "arrowup", "arrowdown", "home", "end", "backspace", "delete"]

/** Keys the window keeps for itself: the macOS menu bar's own items, Alt+F4 on Windows. */
const WINDOW_KEYS = new Set(
  isMac ? ["mod+q", "mod+w", "mod+h", "mod+alt+h", "mod+m", "mod+o", "mod+a", "mod+2", "mod+3", "mod+shift+r", "mod+shift+c"] : ["alt+f4"],
)

/** Why `b` can't do slot `slot` of `s`, or null when it can (perhaps taken from others: `usesOf`). */
export function refusal(b: Binding, s: Shortcut, slot: number, km: Keymap = getKeymap()): string | null {
  const caps = bindingCaps(b).join(" ")
  if (editing(b)) return `${caps} stays with editing text`
  if (WINDOW_KEYS.has(sig(b))) return isMac ? `${caps} belongs to the menu bar` : `${caps} closes the window`
  if (s.scope === "global") {
    // They work in fields too (the palette, the activity drawer), and on
    // macOS the menu answers them before the page.
    if (!b.mod && !/^f[0-9]+$/.test(b.key)) return `Keys that work everywhere need ${modKey}`
    if (b.mod && CARET_KEYS.includes(b.key)) return `${caps} moves the caret in text`
  }
  const fixed = usesOf(b, s, slot, km).find((u) => u.s.fixed)
  return fixed ? `${caps} is “${fixed.s.label}”` : null
}

/** The slot a `slotRef` names, if it is one that can be changed. */
function useOf(ref: string): Use | null {
  const s = ALL_SHORTCUTS.find((o) => ref.startsWith(`${o.scope}.${o.id}.`))
  const slot = Number(ref.slice(ref.lastIndexOf(".") + 1))
  // A shortcut that is gone, or can't be changed (an older or a newer app's).
  if (!s || s.fixed || !Number.isInteger(slot) || slot < 0 || slot >= s.slots.length) return null
  return { s, slot }
}

/** Drops changes that are what the preset has anyway, and names that aren't slots. */
function tidy(km: Keymap): Keymap {
  const own = Object.fromEntries(
    Object.entries(km.own).filter(([ref, bindings]) => {
      const u = useOf(ref)
      return u !== null && bindings.map(sig).join() !== presetBindings(u.s, u.slot, km.preset).map(sig).join()
    }),
  )
  return { ...km, own }
}

/** `km` with `bindings` doing slot `slot` of `s`, each taken from the slots that had it (added to `took`). */
function put(km: Keymap, s: Shortcut, slot: number, bindings: Binding[], took: Use[]): Keymap {
  const own = { ...km.own, [slotRef(s, slot)]: bindings }
  const next = { ...km, own }
  for (const b of bindings) {
    for (const u of usesOf(b, s, slot, next)) {
      if (u.s.fixed) continue
      own[slotRef(u.s, u.slot)] = bindingsOf(u.s, u.slot, next).filter((x) => !sameBinding(x, b))
      took.push(u)
    }
  }
  return next
}

/** `km` with `b` as the only key of slot `slot` of `s`; `took` are the slots it was taken from. */
export function assign(km: Keymap, s: Shortcut, slot: number, b: Binding): { next: Keymap; took: Use[] } {
  const took: Use[] = []
  return { next: tidy(put(km, s, slot, [b], took)), took }
}

/** `km` with `s` on its preset's keys again, taken back from the others that had them (`took`). */
export function resetShortcut(km: Keymap, s: Shortcut): { next: Keymap; took: Use[] } {
  const took: Use[] = []
  const next = s.slots.reduce((k, _, i) => put(k, s, i, presetBindings(s, i, k.preset), took), km)
  // Keys swapped between its own slots come back too, unannounced.
  return { next: tidy(next), took: took.filter((u) => u.s !== s) }
}

/** Whether the person changed any of `s`'s keys. */
export const isChanged = (s: Shortcut, km: Keymap) => s.slots.some((_, i) => slotRef(s, i) in km.own)

/**
 * How the settings hold a keymap, and back. The person's keys win over the
 * preset's: one a newer AutoScene gave a shortcut by default, while the
 * person had it for something else, stays theirs.
 */
export function keymapOf(s: Shortcuts | undefined): Keymap {
  if (!s) return DEFAULT_KEYMAP
  const preset = PRESETS.find((p) => p.id === s.preset)?.id ?? "autoscene"
  const own = Object.fromEntries(Object.entries(s.keys ?? {}).filter(([, v]) => Array.isArray(v)))
  const km = tidy({ preset, own })
  const settled = Object.entries(km.own).reduce((k, [ref, bindings]) => {
    const u = useOf(ref)
    return u ? put(k, u.s, u.slot, bindings, []) : k
  }, km)
  return tidy(settled)
}

function shortcutsOf(km: Keymap): Shortcuts {
  return { preset: km.preset === "autoscene" ? "" : km.preset, keys: km.own }
}

/** Saves wait for the one before (the library takes them in any order), and say how many are left. */
let saving: Promise<void> = Promise.resolve()
let unsaved = 0

/**
 * Use `next` now, and keep it. The last change wins: while saves are on
 * their way the settings don't put back an older keymap (`useKeymapSync`).
 * When the last one fails, the keys go back to what the library holds.
 */
export function saveKeymap(next: Keymap): Promise<void> {
  setKeymap(next)
  unsaved++
  saving = saving.then(async () => {
    try {
      const settings = queryClient.getQueryData<AppSettings>(qk.settings) ?? (await commands.settingsGet())
      const saved = await commands.settingsUpdate({ ...settings, shortcuts: shortcutsOf(next) })
      queryClient.setQueryData(qk.settings, saved)
    } catch (e) {
      toast.error("Couldn't keep the keys", { description: errorMessage(e) })
      if (unsaved === 1) setKeymap(keymapOf(queryClient.getQueryData<AppSettings>(qk.settings)?.shortcuts))
    } finally {
      unsaved--
    }
  })
  return saving
}

/** Use the keys of `preset`, without the person's changes. */
export const withPreset = (preset: PresetId): Keymap => ({ preset, own: {} })

// -- the macOS menu ----------------------------------------------------------

/** The menu items that answer a shortcut's key on macOS (see `mac_menu.rs`). */
const MENU_ITEMS: [ShortcutRef, string][] = [
  ["global.palette", "view:command-palette"],
  ["global.activity", "view:activity"],
  ["global.sidebar", "view:toggle-sidebar"],
  ["global.settings", "app:settings"],
  ["global.library", "view:library"],
  ["global.newProject", "file:new-project"],
  ["player.export", "file:export"],
]

/**
 * The keys a menu accelerator can name (the menu goes by the key's place: no
 * "+" or "?", which take Shift on a US layout).
 */
const MENU_KEY = /^([a-z0-9`\\[\],=\-.';/ ]|f([1-9]|1[0-9]|2[0-4])|arrow(left|right|up|down)|home|end|pageup|pagedown|enter|tab|backspace|delete|insert|escape)$/

/** A key press as a menu accelerator, e.g. `CmdOrCtrl+Shift+K`. */
export function accelerator(b: Binding): string {
  const key = b.key === " " ? "Space" : b.key.length === 1 ? b.key.toUpperCase() : b.key
  const parts = [b.mod && "CmdOrCtrl", b.shift && "Shift", b.alt && "Alt", key]
  return parts.filter(Boolean).join("+")
}

/**
 * Whether the menu can carry `b`. One without ⌘ stays with the page (in the
 * menu it would never reach a field), and so does one it can't name.
 */
const menuCan = (b: Binding | undefined): b is Binding => b !== undefined && (Boolean(b.mod) || /^f[0-9]+$/.test(b.key)) && MENU_KEY.test(b.key)

/** Which export the menu's Export item does: the player's, or the scenepack's while it is open. */
let exportFrom: ShortcutRef = "player.export"

/** The menu item a shortcut's key sits on, if it has one. */
const itemOf = (ref: ShortcutRef) =>
  ref === exportFrom ? "file:export" : MENU_ITEMS.find(([r, id]) => r === ref && id !== "file:export")?.[1]

/**
 * The menu's accelerators for `km` (none at all while a key is recorded).
 */
export function menuKeys(km: Keymap | null): Record<string, string> {
  return Object.fromEntries(
    MENU_ITEMS.map(([ref, id]) => {
      const b = km ? bindingsOf(shortcutOf(id === "file:export" ? exportFrom : ref), 0, km)[0] : undefined
      return [id, menuCan(b) ? accelerator(b) : ""]
    }),
  )
}

/**
 * Whether on macOS the menu answers `ref`'s key now, so the page leaves it to
 * the menu (one it can't carry, the page answers itself).
 */
export function menuAnswers(ref: ShortcutRef): boolean {
  return isMac && isTauri && itemOf(ref) !== undefined && menuCan(bindingsOf(shortcutOf(ref), 0, getKeymap())[0])
}

/** While the scenepack is open, the menu's Export item exports it with its key. */
export function exportWithMenu(ref: "player.export" | "pack.export") {
  if (exportFrom === ref) return
  exportFrom = ref
  pushMenuKeys()
}

let paused = false

function pushMenuKeys() {
  if (!(isMac && isTauri)) return
  commands.menuSetKeys(menuKeys(paused ? null : getKeymap())).catch((e) => console.warn("menu keys:", e))
}

/** While Settings → Shortcuts records a key, the macOS menu lets every key through to it. */
export function pauseMenuKeys(on: boolean) {
  if (paused === on) return
  paused = on
  pushMenuKeys()
}

/** Keep the keymap on what the settings hold, and the macOS menu on the keymap. */
export function useKeymapSync() {
  const { data } = useSettings()
  useEffect(() => {
    if (data && unsaved === 0) setKeymap(keymapOf(data.shortcuts))
  }, [data])
  useEffect(() => {
    pushMenuKeys()
    return subscribeKeymap(pushMenuKeys)
  }, [])
}
