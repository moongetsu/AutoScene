import { describe, expect, it } from "vitest"
import {
  accelerator,
  assign,
  exportWithMenu,
  isChanged,
  keymapOf,
  menuKeys,
  refusal,
  resetShortcut,
  sameBinding,
  slotLabel,
  withPreset,
} from "./keymap"
import { DEFAULT_KEYMAP, bindingsOf, shortcutOf } from "./shortcuts"

const km = DEFAULT_KEYMAP
const labels = (took: Parameters<typeof slotLabel>[0][]) => took.map(slotLabel)

describe("changing a key", () => {
  it("takes it from what had it on the same page", () => {
    const mute = shortcutOf("player.mute")
    const { next, took } = assign(km, mute, 0, { key: "x" })
    expect(labels(took)).toEqual(["Mark the shot as the clip"])
    expect(bindingsOf(mute, 0, next)).toEqual([{ key: "x" }])
    expect(bindingsOf(shortcutOf("player.markShot"), 0, next)).toEqual([])
    // Other pages keep theirs: X is still "Not a face" while reviewing.
    expect(bindingsOf(shortcutOf("review.notFace"), 0, next)).toEqual([{ key: "x" }])
  })

  it("takes only the key that was asked for, from a slot with several", () => {
    // "Yes" also answers to J and Enter.
    const { next, took } = assign(km, shortcutOf("review.no"), 0, { key: "j" })
    expect(labels(took)).toEqual(["Yes, it's them"])
    expect(bindingsOf(shortcutOf("review.yes"), 0, next)).toEqual([{ key: "y" }, { key: "enter" }])
  })

  it("takes a key that works everywhere from every page", () => {
    const { next, took } = assign(km, shortcutOf("global.palette"), 0, { key: "g", mod: true })
    expect(labels(took)).toEqual(["Go to a timecode"])
    expect(bindingsOf(shortcutOf("player.goTo"), 0, next)).toEqual([])
  })

  it("refuses keys that can't be a shortcut there", () => {
    const palette = shortcutOf("global.palette")
    expect(refusal({ key: "p" }, palette, 0, km)).toMatch(/need Ctrl/)
    expect(refusal({ key: "f2" }, palette, 0, km)).toBeNull()
    expect(refusal({ key: "c", mod: true }, shortcutOf("player.mute"), 0, km)).toMatch(/editing text/)
    expect(refusal({ key: "z", mod: true, shift: true }, shortcutOf("player.mute"), 0, km)).toMatch(/editing text/)
    // Premiere's Clear in and out is no editing key.
    expect(refusal({ key: "x", mod: true, shift: true }, shortcutOf("player.clearMarks"), 0, km)).toBeNull()
    // Keys that stay are never taken.
    expect(refusal({ key: "3" }, shortcutOf("review.yes"), 0, km)).toMatch(/It's character 1 to 9/)
    expect(refusal({ key: "a", mod: true }, palette, 0, km)).toMatch(/Select every/)
    // The palette answers in fields: the caret's keys stay with them.
    expect(refusal({ key: "backspace", mod: true }, palette, 0, km)).toMatch(/caret/)
    expect(refusal({ key: "arrowleft", mod: true, shift: true }, palette, 0, km)).toMatch(/caret/)
    expect(refusal({ key: "arrowright", mod: true }, shortcutOf("player.shot"), 1, km)).toBeNull()
    expect(refusal({ key: "f4", alt: true }, shortcutOf("player.mute"), 0, km)).toMatch(/closes the window/)
  })

  it("keeps only real changes", () => {
    const mute = shortcutOf("player.mute")
    const { next } = assign(km, mute, 0, { key: "u" })
    expect(isChanged(mute, next)).toBe(true)
    const back = assign(next, mute, 0, { key: "m" }).next
    expect(back.own).toEqual({})
    expect(isChanged(mute, back)).toBe(false)
  })

  it("takes the keys back on a reset", () => {
    const markShot = shortcutOf("player.markShot")
    const mute = shortcutOf("player.mute")
    const moved = assign(km, mute, 0, { key: "x" }).next
    const { next, took } = resetShortcut(moved, markShot)
    expect(labels(took)).toEqual(["Mute"])
    expect(bindingsOf(markShot, 0, next)).toEqual([{ key: "x" }])
    expect(bindingsOf(mute, 0, next)).toEqual([])
    expect(isChanged(markShot, next)).toBe(false)
  })

  it("takes a sibling's key without a word when a row is reset", () => {
    const frame = shortcutOf("player.frame")
    // Next frame on ←: taken from Previous frame, which is said.
    const swapped = assign(km, frame, 1, { key: "arrowleft" })
    expect(labels(swapped.took)).toEqual(["Previous frame"])
    const { next, took } = resetShortcut(swapped.next, frame)
    expect(took).toEqual([])
    expect(next.own).toEqual({})
  })

  it("resets to the preset's keys, not AutoScene's", () => {
    const exp = shortcutOf("player.export")
    const premiere = assign(withPreset("premiere"), exp, 0, { key: "p", mod: true }).next
    expect(bindingsOf(exp, 0, resetShortcut(premiere, exp).next)).toEqual([{ key: "m", mod: true }])
  })

  it("sees a digit without Shift as taking Shift too", () => {
    expect(sameBinding({ key: "1" }, { key: "1", shift: true })).toBe(true)
    expect(sameBinding({ key: "a" }, { key: "a", shift: true })).toBe(false)
    expect(sameBinding({ key: "a", mod: true }, { key: "a" })).toBe(false)
  })
})

describe("the stored keymap", () => {
  it("reads what the settings hold, leaving out what it doesn't know", () => {
    expect(keymapOf(undefined)).toEqual(DEFAULT_KEYMAP)
    expect(keymapOf({ preset: "", keys: {} })).toEqual(DEFAULT_KEYMAP)
    expect(keymapOf({ preset: "avid", keys: {} }).preset).toBe("autoscene")
    const read = keymapOf({
      preset: "premiere",
      keys: {
        "player.mute.0": [{ key: "u" }],
        "player.gone.0": [{ key: "q" }],
        "player.frame.7": [{ key: "q" }],
        "review.pick.0": [{ key: "q" }],
        // What the preset has anyway.
        "player.export.0": [{ key: "m", mod: true }],
      },
    })
    expect(read).toEqual({ preset: "premiere", own: { "player.mute.0": [{ key: "u" }] } })
  })

  it("keeps the person's key when a default would take it too", () => {
    // Say "M" was a newer default for something else: the stored Mute on X
    // must not leave X doing two things.
    const read = keymapOf({ preset: "", keys: { "player.mute.0": [{ key: "x" }] } })
    expect(bindingsOf(shortcutOf("player.mute"), 0, read)).toEqual([{ key: "x" }])
    expect(bindingsOf(shortcutOf("player.markShot"), 0, read)).toEqual([])
  })
})

describe("the macOS menu", () => {
  it("gets the keys of the shortcuts it answers", () => {
    const keys = menuKeys(km)
    expect(keys["view:command-palette"]).toBe("CmdOrCtrl+K")
    expect(keys["app:settings"]).toBe("CmdOrCtrl+,")
    expect(keys["file:export"]).toBe("CmdOrCtrl+E")
    expect(menuKeys(withPreset("premiere"))["file:export"]).toBe("CmdOrCtrl+M")
  })

  it("leaves keys without ⌘ to the page, and every key while one is recorded", () => {
    const plain = assign(km, shortcutOf("player.export"), 0, { key: "e" }).next
    expect(menuKeys(plain)["file:export"]).toBe("")
    expect(Object.values(menuKeys(null)).every((k) => k === "")).toBe(true)
    expect(accelerator({ key: " ", mod: true, shift: true })).toBe("CmdOrCtrl+Shift+Space")
    expect(accelerator({ key: "arrowleft", mod: true })).toBe("CmdOrCtrl+arrowleft")
    // A key the menu can't name ("+" and "?" take Shift) stays with the page.
    const plus = assign(km, shortcutOf("global.palette"), 0, { key: "+", mod: true }).next
    expect(menuKeys(plus)["view:command-palette"]).toBe("")
    const slash = assign(km, shortcutOf("global.palette"), 0, { key: "/", mod: true }).next
    expect(menuKeys(slash)["view:command-palette"]).toBe("CmdOrCtrl+/")
  })

  it("exports the scenepack from the menu while it is open", () => {
    const pack = assign(km, shortcutOf("pack.export"), 0, { key: "p", mod: true }).next
    expect(menuKeys(pack)["file:export"]).toBe("CmdOrCtrl+E")
    exportWithMenu("pack.export")
    expect(menuKeys(pack)["file:export"]).toBe("CmdOrCtrl+P")
    exportWithMenu("player.export")
    expect(menuKeys(pack)["file:export"]).toBe("CmdOrCtrl+E")
  })
})
