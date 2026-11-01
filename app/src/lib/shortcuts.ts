import { useCallback, useEffect, useLayoutEffect, useRef, useSyncExternalStore } from "react"
import { layerOpen } from "./layers"
import { altKey, isMac, modKey, shiftKey } from "./platform"

/**
 * Every keyboard shortcut of the app, as data: the handlers (`useShortcuts`),
 * Settings → Shortcuts and the player's Keys card all read these lists, so
 * what is documented is what works. The lists hold AutoScene's own keys; the
 * keymap (`setKeymap`) says which keys do what now — an editor's preset and
 * the person's own changes, from Settings → Shortcuts.
 */

/**
 * A key press that triggers a shortcut. `key` is `KeyboardEvent.key` in lower
 * case — a digit means the key that types it on a US layout (see `matches`);
 * `mod` is Ctrl (⌘ on macOS).
 */
export type Binding = { key: string; mod?: boolean; shift?: boolean; alt?: boolean }

/** One thing a shortcut does, and the keys that do it: the first is the one shown, the others work too. */
export type Slot = { label?: string; bindings: Binding[] }

/** Where a shortcut works: everywhere, or on one kind of page. */
export type Scope = "global" | "player" | "review" | "cast" | "faces" | "pack"

export type Shortcut<Id extends string = string> = {
  scope: Scope
  id: Id
  /** What it does, as listed in Settings → Shortcuts. */
  label: string
  /** One per key: "Previous / next frame" has two. */
  slots: Slot[]
  /** Its keys can't be changed: a range of keys, or what they mean to a focused shot or a selection. */
  fixed?: boolean
  /** Shown instead of the keys, e.g. ["1 – 9"]. */
  caps?: string[]
}

const key = (k: string, more: Omit<Binding, "key"> = {}): Binding => ({ key: k, ...more })
/** One thing, done by these keys. */
const one = (...bindings: Binding[]): Slot[] => [{ bindings }]
/** A few things in one row (previous / next), each with its own key. */
const each = (...slots: [label: string, ...bindings: Binding[]][]): Slot[] =>
  slots.map(([label, ...bindings]) => ({ label, bindings }))
const scoped = <Id extends string>(scope: Scope, list: Omit<Shortcut<Id>, "scope">[]): Shortcut<Id>[] =>
  list.map((s) => ({ ...s, scope }))

export type GlobalId = "palette" | "activity" | "sidebar" | "settings" | "library" | "newProject"

/** Anywhere in the window. On macOS desktop the native menu owns them (same keys). */
export const GLOBAL_SHORTCUTS = scoped<GlobalId>("global", [
  { id: "palette", label: "Command palette", slots: one(key("k", { mod: true })) },
  { id: "activity", label: "Show activity", slots: one(key("j", { mod: true })) },
  { id: "sidebar", label: "Show or hide the rail", slots: one(key("b", { mod: true })) },
  { id: "settings", label: "Settings", slots: one(key(",", { mod: true })) },
  { id: "library", label: "Library", slots: one(key("1", { mod: true })) },
  { id: "newProject", label: "New project", slots: one(key("n", { mod: true })) },
])

export type PlayerId =
  | "playPause"
  | "shuttle"
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
  | "zoomIn"
  | "zoomOut"
  | "zoomFit"
  | "export"

/** While an episode is open. */
export const PLAYER_SHORTCUTS = scoped<PlayerId>("player", [
  { id: "playPause", label: "Play / pause", slots: one(key(" ")) },
  {
    id: "shuttle",
    label: "Backwards / stop / forwards (again: faster; stop held: a frame)",
    slots: each(["Backwards", key("j")], ["Stop", key("k")], ["Forwards", key("l")]),
  },
  {
    id: "frame",
    label: "Previous / next frame",
    slots: each(["Previous frame", key("arrowleft")], ["Next frame", key("arrowright")]),
  },
  {
    id: "second",
    label: "Back / forward one second",
    slots: each(
      ["Back one second", key("arrowleft", { shift: true })],
      ["Forward one second", key("arrowright", { shift: true })],
    ),
  },
  {
    id: "shot",
    label: "Previous / next shot",
    slots: each(["Previous shot", key("arrowup")], ["Next shot", key("arrowdown")]),
  },
  { id: "ends", label: "First / last frame", slots: each(["First frame", key("home")], ["Last frame", key("end")]) },
  { id: "mark", label: "Mark in / out", slots: each(["Mark in", key("i")], ["Mark out", key("o")]) },
  {
    id: "goToMark",
    label: "Go to in / out",
    slots: each(["Go to in", key("i", { shift: true })], ["Go to out", key("o", { shift: true })]),
  },
  { id: "markShot", label: "Mark the shot as the clip", slots: one(key("x")) },
  { id: "clearMarks", label: "Clear in and out", slots: one(key("x", { alt: true })) },
  { id: "goTo", label: "Go to a timecode", slots: one(key("g", { mod: true })) },
  { id: "mute", label: "Mute", slots: one(key("m")) },
  {
    id: "zoomIn",
    label: "Zoom the timeline in",
    // "+" has its own key on many layouts; on US ones it's Shift+"=".
    slots: one(key("+"), key("+", { shift: true }), key("="), key("=", { shift: true })),
  },
  { id: "zoomOut", label: "Zoom the timeline out", slots: one(key("-")) },
  { id: "zoomFit", label: "The whole episode on the timeline", slots: one(key("z", { shift: true }), key("\\")) },
  { id: "export", label: "Export the marked clip", slots: one(key("e", { mod: true })) },
])

export type ReviewId = "yes" | "no" | "notFace" | "skip" | "pick"

/** While reviewing faces. */
export const REVIEW_SHORTCUTS = scoped<ReviewId>("review", [
  { id: "yes", label: "Yes, it's them", slots: one(key("y"), key("j"), key("enter")) },
  { id: "no", label: "No, someone else", slots: one(key("n")) },
  { id: "notFace", label: "Not a face", slots: one(key("x")) },
  { id: "skip", label: "Skip for now", slots: one(key("arrowright"), key("s")) },
  {
    id: "pick",
    label: "It's character 1 to 9",
    slots: one(...["1", "2", "3", "4", "5", "6", "7", "8", "9"].map((d) => key(d))),
    fixed: true,
    caps: ["1 – 9"],
  },
])

export type FacesId = "selectAll" | "clearSelection"

/** On a character's faces. */
export const FACES_SHORTCUTS = scoped<FacesId>("faces", [
  { id: "selectAll", label: "Select every face", slots: one(key("a", { mod: true })), fixed: true },
  { id: "clearSelection", label: "Clear the selection", slots: one(key("escape")), fixed: true },
])

export type CastId = "selectAll" | "clearSelection"

/** On the Cast page's cards. */
export const CAST_SHORTCUTS = scoped<CastId>("cast", [
  { id: "selectAll", label: "Select every character", slots: one(key("a", { mod: true })), fixed: true },
  { id: "clearSelection", label: "Clear the selection", slots: one(key("escape")), fixed: true },
])

export type PackId = "toggle" | "watch" | "export"

/** While building a scenepack (the first two on the focused shot). */
export const PACK_SHORTCUTS = scoped<PackId>("pack", [
  {
    id: "toggle",
    label: "Take the shot in or out (Shift: every shot since the last)",
    slots: one(key(" "), key(" ", { shift: true })),
    fixed: true,
  },
  { id: "watch", label: "Watch the shot", slots: one(key("enter")), fixed: true },
  { id: "export", label: "Export the pack", slots: one(key("e", { mod: true })) },
])

/** Settings → Shortcuts, in this order. */
export const SHORTCUT_GROUPS: { title: string; description: string; shortcuts: Shortcut[] }[] = [
  { title: "Everywhere", description: "Work faster without leaving the keyboard.", shortcuts: GLOBAL_SHORTCUTS },
  { title: "Player", description: "While an episode is open.", shortcuts: PLAYER_SHORTCUTS },
  { title: "Review faces", description: "Answer with one key; every answer is kept.", shortcuts: REVIEW_SHORTCUTS },
  {
    title: "Cast",
    description: `On the Cast page. ${modKey}-click selects a character too, Shift-click every one between.`,
    shortcuts: CAST_SHORTCUTS,
  },
  { title: "A character's faces", description: "In a character's Faces view.", shortcuts: FACES_SHORTCUTS },
  { title: "Scenepack", description: "While picking a character's shots.", shortcuts: PACK_SHORTCUTS },
]

export const ALL_SHORTCUTS: Shortcut[] = SHORTCUT_GROUPS.flatMap((g) => g.shortcuts)

/** A shortcut by where it works and its id, e.g. `player.mark`. */
export type ShortcutRef =
  | `global.${GlobalId}`
  | `player.${PlayerId}`
  | `review.${ReviewId}`
  | `cast.${CastId}`
  | `faces.${FacesId}`
  | `pack.${PackId}`

const BY_REF = new Map(ALL_SHORTCUTS.map((s) => [`${s.scope}.${s.id}`, s]))

export function shortcutOf(ref: ShortcutRef): Shortcut {
  const s = BY_REF.get(ref)
  if (!s) throw new Error(`no shortcut ${ref}`)
  return s
}

// -- the keymap --------------------------------------------------------------

export type PresetId = "autoscene" | "premiere" | "resolve"

/** Which keys are in use: an editor's, and the person's own changes on top (by `slotRef`). */
export type Keymap = { preset: PresetId; own: Record<string, Binding[]> }

export const DEFAULT_KEYMAP: Keymap = { preset: "autoscene", own: {} }

/** How a slot is named in a preset and in the person's changes: `player.frame.1` is the next frame. */
export const slotRef = (s: Shortcut, slot: number) => `${s.scope}.${s.id}.${slot}`

/**
 * Keys like in the editors people come from, where their defaults differ
 * from AutoScene's (which already follow them for playing and marking).
 */
export const PRESETS: { id: PresetId; label: string; description: string; keys: Record<string, Binding[]> }[] = [
  { id: "autoscene", label: "AutoScene", description: "The keys AutoScene comes with.", keys: {} },
  {
    id: "premiere",
    label: "Premiere Pro",
    // On macOS Premiere clears in and out with ⌥X like AutoScene, and ⌘M
    // minimizes the window (the menu bar's), so Export keeps ⌘E there.
    description: isMac
      ? "Zoom with = − \\, the rest like AutoScene."
      : `Export with ${modKey} M, zoom with = − \\, clear in and out with ${modKey} Shift X.`,
    keys: {
      ...(!isMac && { "player.clearMarks.0": [key("x", { mod: true, shift: true })] }),
      "player.zoomIn.0": [key("="), key("+"), key("=", { shift: true }), key("+", { shift: true })],
      "player.zoomFit.0": [key("\\")],
      ...(!isMac && { "player.export.0": [key("m", { mod: true })] }),
    },
  },
  {
    id: "resolve",
    label: "DaVinci Resolve",
    description: `Zoom with ${modKey} = and ${modKey} −.`,
    keys: {
      "player.zoomIn.0": [
        key("=", { mod: true }),
        key("+", { mod: true }),
        key("=", { mod: true, shift: true }),
        key("+", { mod: true, shift: true }),
      ],
      "player.zoomOut.0": [key("-", { mod: true })],
    },
  },
]

let keymap = DEFAULT_KEYMAP
const listeners = new Set<() => void>()

export const getKeymap = () => keymap

export function setKeymap(next: Keymap) {
  if (next === keymap) return
  keymap = next
  for (const l of listeners) l()
}

export function subscribeKeymap(listener: () => void) {
  listeners.add(listener)
  return () => void listeners.delete(listener)
}

/** The keymap, re-rendering when it changes. */
export const useKeymap = () => useSyncExternalStore(subscribeKeymap, getKeymap)

/** The keys a preset gives slot `slot` of `s`, before the person's own changes. */
export function presetBindings(s: Shortcut, slot: number, preset: PresetId): Binding[] {
  if (s.fixed) return s.slots[slot].bindings
  return PRESETS.find((p) => p.id === preset)?.keys[slotRef(s, slot)] ?? s.slots[slot].bindings
}

/** The keys that do slot `slot` of `s` now (none when the person took them away). */
export function bindingsOf(s: Shortcut, slot: number, km: Keymap = keymap): Binding[] {
  if (s.fixed) return s.slots[slot].bindings
  return km.own[slotRef(s, slot)] ?? presetBindings(s, slot, km.preset)
}

// -- key caps ----------------------------------------------------------------

const KEY_NAMES: Record<string, string> = {
  " ": "Space",
  arrowleft: "←",
  arrowright: "→",
  arrowup: "↑",
  arrowdown: "↓",
  escape: "Esc",
  enter: "Enter",
  backspace: "Backspace",
  delete: "Del",
  insert: "Ins",
  pageup: "Page Up",
  pagedown: "Page Down",
  "-": "−",
}

/** A key as its cap shows it. */
export function keyCap(k: string): string {
  return KEY_NAMES[k] ?? (k.length === 1 ? k.toUpperCase() : k[0].toUpperCase() + k.slice(1))
}

/** The modifiers of `b` as caps, in the platform's order. */
export function modifierCaps(b: Omit<Binding, "key">): string[] {
  const caps: [boolean | undefined, string][] = isMac
    ? [
        [b.alt, altKey],
        [b.shift, shiftKey],
        [b.mod, modKey],
      ]
    : [
        [b.mod, modKey],
        [b.shift, shiftKey],
        [b.alt, altKey],
      ]
  return caps.filter(([on]) => on).map(([, cap]) => cap)
}

/** A key press as caps, e.g. ["Ctrl", "K"]. */
export const bindingCaps = (b: Binding) => [...modifierCaps(b), keyCap(b.key)]

/** The caps of slot `slot` of `s` as the keymap has it (empty without a key). */
export function slotCaps(s: Shortcut, slot: number, km: Keymap = keymap): string[] {
  if (s.caps) return s.caps
  const b = bindingsOf(s, slot, km)[0]
  return b ? bindingCaps(b) : []
}

/**
 * All keys of `s` as groups of caps: one group when they share their
 * modifiers ("Shift ← →"), else one per key — an empty group for a key the
 * person took away, so the others keep their meaning. None without any key.
 */
export function capGroups(s: Shortcut, km: Keymap = keymap): string[][] {
  if (s.caps) return [s.caps]
  const firsts = s.slots.map((_, i) => bindingsOf(s, i, km)[0])
  const set = firsts.filter((b) => b !== undefined)
  if (set.length === 0) return []
  const mods = modifierCaps(set[0])
  if (set.length === firsts.length && set.every((b) => modifierCaps(b).join() === mods.join())) {
    return [[...mods, ...set.map((b) => keyCap(b.key))]]
  }
  return firsts.map((b) => (b ? bindingCaps(b) : []))
}

/** Caps as running text reads them: "Shift+I" (⇧I on macOS). */
export const keysText = (caps: string[]) => caps.join(isMac ? "" : "+")

/** The caps of one of a shortcut's keys (the first by default), for a menu entry or a tooltip. */
export function shortcutKeys(ref: ShortcutRef, slot = 0): string[] {
  return slotCaps(shortcutOf(ref), slot)
}

/** `shortcutKeys`, re-rendering when the keymap changes. */
export function useShortcutKeys(): (ref: ShortcutRef, slot?: number) => string[] {
  const km = useKeymap()
  return useCallback((ref: ShortcutRef, slot = 0) => slotCaps(shortcutOf(ref), slot, km), [km])
}

// -- matching ----------------------------------------------------------------

/**
 * The digit a key press means: the digit row goes by the physical key, as
 * AZERTY types "&" there and needs Shift for the 1; the number pad counts
 * while it types digits.
 */
export function digitOf(e: KeyboardEvent): string | null {
  const row = /^Digit([0-9])$/.exec(e.code)
  if (row) return row[1]
  return /^[0-9]$/.test(e.key) ? e.key : null
}

/**
 * The key a press means, in lower case. On macOS ⌥ types a character of its
 * own (⌥X is "≈", ⌥E a dead key), so with it a letter key goes by its place.
 */
function keyOf(e: KeyboardEvent): string {
  const letter = isMac && e.altKey ? /^Key([A-Z])$/.exec(e.code) : null
  return letter ? letter[1].toLowerCase() : e.key.toLowerCase()
}

/** Does this key press trigger `b`? Modifiers must match exactly (Shift doesn't count for digits). */
export function matches(b: Binding, e: KeyboardEvent): boolean {
  const mod = isMac ? e.metaKey : e.ctrlKey
  const other = isMac ? e.ctrlKey : e.metaKey
  const digit = /^[0-9]$/.test(b.key)
  return (
    (digit ? digitOf(e) === b.key : keyOf(e) === b.key) &&
    mod === Boolean(b.mod) &&
    (digit && !b.shift ? true : e.shiftKey === Boolean(b.shift)) &&
    e.altKey === Boolean(b.alt) &&
    !other
  )
}

/** Whether this key press is `b`'s key, whatever the modifiers (a held key let go). */
export function sameKey(b: Binding, e: KeyboardEvent): boolean {
  return /^[0-9]$/.test(b.key) ? digitOf(e) === b.key : keyOf(e) === b.key
}

/** The shortcut a key press triggers, and which of its keys it was. */
export function matchSlot<Id extends string>(
  shortcuts: Shortcut<Id>[],
  e: KeyboardEvent,
  km: Keymap = keymap,
): { id: Id; slot: number } | null {
  for (const s of shortcuts) {
    for (let slot = 0; slot < s.slots.length; slot++) {
      if (bindingsOf(s, slot, km).some((b) => matches(b, e))) return { id: s.id, slot }
    }
  }
  return null
}

/** The shortcut a key press triggers, if any. */
export function matchShortcut<Id extends string>(shortcuts: Shortcut<Id>[], e: KeyboardEvent): Id | null {
  return matchSlot(shortcuts, e)?.id ?? null
}

/** Keys that never make a shortcut on their own. */
const MODIFIERS = new Set(["Shift", "Control", "Alt", "AltGraph", "Meta", "OS", "Fn", "FnLock", "Hyper", "Super"])

/**
 * The binding a key press makes (Settings → Shortcuts records it), read the
 * way `matches` reads it; null while only modifiers are down, or for a key
 * the browser can't name (a dead key, an input method's).
 */
export function bindingOf(e: KeyboardEvent): Binding | null {
  if (MODIFIERS.has(e.key) || e.isComposing) return null
  const k = digitOf(e) ?? keyOf(e)
  if (["dead", "unidentified", "process"].includes(k)) return null
  const b: Binding = { key: k }
  if (isMac ? e.metaKey : e.ctrlKey) b.mod = true
  if (e.shiftKey) b.shift = true
  if (e.altKey) b.alt = true
  return b
}

/** A field that takes typing: its keys are its own. */
export function isEditable(t: EventTarget | null): boolean {
  return t instanceof Element && t.closest("input, textarea, select, [contenteditable=''], [contenteditable='true']") != null
}

/**
 * Return `false` from a handler to let the key through (it wasn't meant for
 * it). `slot` says which of the shortcut's keys it was (0: previous, 1: next).
 */
export type ShortcutHandlers<Id extends string> = Partial<Record<Id, (e: KeyboardEvent, slot: number) => boolean | void>>

/**
 * Answer `shortcuts` while mounted (and `enabled`), with the keys the keymap
 * gives them at the moment of the press. Keys belong to a field being typed
 * in, to an open dialog or menu, and to a focused button (Space and Enter
 * press it) — except the ids in `anywhere` (the palette and the activity
 * drawer, which open and close a layer of their own). A held key repeats
 * only where `repeat` is set (stepping through frames): an answer must never
 * fire twice because a key was held a little long.
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
      const hit = matchSlot(shortcuts, e)
      if (!hit) return
      if (!everywhere.has(hit.id)) {
        if (isEditable(e.target) || layerOpen()) return
        const pressable = e.target instanceof Element && e.target.closest("button, a, [role=button]")
        if ((e.key === " " || e.key === "Enter") && pressable) return
      }
      const run = latest.current[hit.id]
      if (!run || run(e, hit.slot) === false) return
      e.preventDefault()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [shortcuts, enabled, repeat, always])
}
