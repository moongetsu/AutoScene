import { useEffect, useLayoutEffect, useRef } from "react"
import { layerOpen } from "./layers"
import { isMac, modKey } from "./platform"

/**
 * Every keyboard shortcut of the app, as data: the handlers (`useShortcuts`),
 * Settings → Shortcuts and the player's Keys card all read these lists, so
 * what is documented is what works.
 */

/**
 * A key press that triggers a shortcut. `key` is `KeyboardEvent.key` in lower
 * case — a digit means the key that types it on a US layout (see `matches`);
 * `mod` is Ctrl (⌘ on macOS).
 */
export type Binding = { key: string; mod?: boolean; shift?: boolean; alt?: boolean }

export type Shortcut<Id extends string = string> = {
  id: Id
  /** What it does, as listed in Settings → Shortcuts. */
  label: string
  /** The keys as key caps, e.g. ["Ctrl", "K"]. */
  keys: string[]
  bindings: Binding[]
}

const key = (k: string, more: Omit<Binding, "key"> = {}): Binding => ({ key: k, ...more })

export type GlobalId = "palette" | "activity" | "sidebar" | "settings" | "library" | "newProject"

/** Anywhere in the window. On macOS desktop the native menu owns them (same keys). */
export const GLOBAL_SHORTCUTS: Shortcut<GlobalId>[] = [
  { id: "palette", label: "Command palette", keys: [modKey, "K"], bindings: [key("k", { mod: true })] },
  { id: "activity", label: "Show activity", keys: [modKey, "J"], bindings: [key("j", { mod: true })] },
  { id: "sidebar", label: "Show or hide the rail", keys: [modKey, "B"], bindings: [key("b", { mod: true })] },
  { id: "settings", label: "Settings", keys: [modKey, ","], bindings: [key(",", { mod: true })] },
  { id: "library", label: "Library", keys: [modKey, "1"], bindings: [key("1", { mod: true })] },
  { id: "newProject", label: "New project", keys: [modKey, "N"], bindings: [key("n", { mod: true })] },
]

export type PlayerId =
  | "playPause"
  | "frame"
  | "second"
  | "shot"
  | "ends"
  | "mark"
  | "goToMark"
  | "markShot"
  | "clearMarks"
  | "goTo"
  | "mute"
  | "export"

/** While an episode is open. */
export const PLAYER_SHORTCUTS: Shortcut<PlayerId>[] = [
  { id: "playPause", label: "Play / pause", keys: ["Space"], bindings: [key(" "), key("k")] },
  { id: "frame", label: "Previous / next frame", keys: ["←", "→"], bindings: [key("arrowleft"), key("arrowright")] },
  {
    id: "second",
    label: "Back / forward one second",
    keys: ["Shift", "← →"],
    bindings: [key("arrowleft", { shift: true }), key("arrowright", { shift: true })],
  },
  { id: "shot", label: "Previous / next shot", keys: ["↑", "↓"], bindings: [key("arrowup"), key("arrowdown")] },
  { id: "ends", label: "First / last frame", keys: ["Home", "End"], bindings: [key("home"), key("end")] },
  { id: "mark", label: "Mark in / out", keys: ["I", "O"], bindings: [key("i"), key("o")] },
  {
    id: "goToMark",
    label: "Go to in / out",
    keys: ["Shift", "I / O"],
    bindings: [key("i", { shift: true }), key("o", { shift: true })],
  },
  { id: "markShot", label: "Mark the shot as the clip", keys: ["X"], bindings: [key("x")] },
  { id: "clearMarks", label: "Clear in and out", keys: ["Alt", "X"], bindings: [key("x", { alt: true })] },
  { id: "goTo", label: "Go to a timecode", keys: [modKey, "G"], bindings: [key("g", { mod: true })] },
  { id: "mute", label: "Mute", keys: ["M"], bindings: [key("m")] },
  { id: "export", label: "Export the marked clip", keys: [modKey, "E"], bindings: [key("e", { mod: true })] },
]

export type ReviewId = "yes" | "no" | "notFace" | "skip" | "pick"

/** While reviewing faces. */
export const REVIEW_SHORTCUTS: Shortcut<ReviewId>[] = [
  { id: "yes", label: "Yes, it's them", keys: ["Y"], bindings: [key("y"), key("j"), key("enter")] },
  { id: "no", label: "No, someone else", keys: ["N"], bindings: [key("n")] },
  { id: "notFace", label: "Not a face", keys: ["X"], bindings: [key("x")] },
  { id: "skip", label: "Skip for now", keys: ["→"], bindings: [key("arrowright"), key("s")] },
  {
    id: "pick",
    label: "It's character 1 to 9",
    keys: ["1 – 9"],
    bindings: ["1", "2", "3", "4", "5", "6", "7", "8", "9"].map((d) => key(d)),
  },
]

export type FacesId = "selectAll" | "clearSelection"

/** On a character's faces. */
export const FACES_SHORTCUTS: Shortcut<FacesId>[] = [
  { id: "selectAll", label: "Select every face", keys: [modKey, "A"], bindings: [key("a", { mod: true })] },
  { id: "clearSelection", label: "Clear the selection", keys: ["Esc"], bindings: [key("escape")] },
]

/** Settings → Shortcuts, in this order. */
export const SHORTCUT_GROUPS: { title: string; description: string; shortcuts: Shortcut[] }[] = [
  { title: "Everywhere", description: "Work faster without leaving the keyboard.", shortcuts: GLOBAL_SHORTCUTS },
  { title: "Player", description: "While an episode is open.", shortcuts: PLAYER_SHORTCUTS },
  { title: "Review faces", description: "Answer with one key; every answer is kept.", shortcuts: REVIEW_SHORTCUTS },
  { title: "A character's faces", description: "In a character's Faces view.", shortcuts: FACES_SHORTCUTS },
]

export function shortcutKeys(id: GlobalId): string[] {
  return GLOBAL_SHORTCUTS.find((s) => s.id === id)?.keys ?? []
}

/**
 * The digit a key press means: the digit row goes by the physical key, as
 * AZERTY types "&" there and needs Shift for the 1; the number pad counts
 * while it types digits.
 */
function digitOf(e: KeyboardEvent): string | null {
  const row = /^Digit([0-9])$/.exec(e.code)
  if (row) return row[1]
  return /^[0-9]$/.test(e.key) ? e.key : null
}

/** Does this key press trigger `b`? Modifiers must match exactly (Shift doesn't count for digits). */
export function matches(b: Binding, e: KeyboardEvent): boolean {
  const mod = isMac ? e.metaKey : e.ctrlKey
  const other = isMac ? e.ctrlKey : e.metaKey
  const digit = /^[0-9]$/.test(b.key)
  return (
    (digit ? digitOf(e) === b.key : e.key.toLowerCase() === b.key) &&
    mod === Boolean(b.mod) &&
    (digit && !b.shift ? true : e.shiftKey === Boolean(b.shift)) &&
    e.altKey === Boolean(b.alt) &&
    !other
  )
}

/** The shortcut a key press triggers, if any. */
export function matchShortcut<Id extends string>(shortcuts: Shortcut<Id>[], e: KeyboardEvent): Id | null {
  return shortcuts.find((s) => s.bindings.some((b) => matches(b, e)))?.id ?? null
}

/** A field that takes typing: its keys are its own. */
export function isEditable(t: EventTarget | null): boolean {
  return t instanceof Element && t.closest("input, textarea, select, [contenteditable=''], [contenteditable='true']") != null
}

/** Return `false` from a handler to let the key through (it wasn't meant for it). */
export type ShortcutHandlers<Id extends string> = Partial<Record<Id, (e: KeyboardEvent) => boolean | void>>

/**
 * Answer `shortcuts` while mounted (and `enabled`). Keys belong to a field
 * being typed in, to an open dialog or menu, and to a focused button (Space
 * and Enter press it) — except the ids in `anywhere` (the palette and the
 * activity drawer, which open and close a layer of their own). A held key
 * repeats only where `repeat` is set (stepping through frames): an answer
 * must never fire twice because a key was held a little long.
 */
export function useShortcuts<Id extends string>(
  shortcuts: Shortcut<Id>[],
  handlers: ShortcutHandlers<Id>,
  { enabled = true, repeat = false, anywhere = [] }: { enabled?: boolean; repeat?: boolean; anywhere?: Id[] } = {},
) {
  const latest = useRef(handlers)
  useLayoutEffect(() => {
    latest.current = handlers
  })
  const always = anywhere.join(",")
  useEffect(() => {
    if (!enabled) return
    const everywhere = new Set(always ? always.split(",") : [])
    const onKey = (e: KeyboardEvent) => {
      if (e.defaultPrevented || e.isComposing || (e.repeat && !repeat)) return
      const id = matchShortcut(shortcuts, e)
      if (!id) return
      if (!everywhere.has(id)) {
        if (isEditable(e.target) || layerOpen()) return
        const pressable = e.target instanceof Element && e.target.closest("button, a, [role=button]")
        if ((e.key === " " || e.key === "Enter") && pressable) return
      }
      const run = latest.current[id]
      if (!run || run(e) === false) return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [shortcuts, enabled, repeat, always])
}
