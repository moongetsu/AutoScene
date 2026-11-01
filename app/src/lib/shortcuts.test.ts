import { cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import {
  ALL_SHORTCUTS,
  type Binding,
  DEFAULT_KEYMAP,
  GLOBAL_SHORTCUTS,
  PACK_SHORTCUTS,
  PLAYER_SHORTCUTS,
  PRESETS,
  REVIEW_SHORTCUTS,
  SHORTCUT_GROUPS,
  bindingOf,
  bindingsOf,
  capGroups,
  digitOf,
  matchShortcut,
  matchSlot,
  matches,
  setKeymap,
  shortcutOf,
  slotCaps,
  slotRef,
  useShortcuts,
} from "./shortcuts"

afterEach(() => {
  cleanup()
  document.body.innerHTML = ""
})

const sig = (b: Binding) => `${b.mod ? "mod+" : ""}${b.shift ? "shift+" : ""}${b.alt ? "alt+" : ""}${b.key}`

const press = (key: string, init: KeyboardEventInit = {}, target: EventTarget = window) => {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init })
  target.dispatchEvent(e)
  return e
}

describe("matching", () => {
  it("takes modifiers exactly", () => {
    const e = (key: string, init: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key, ...init })
    expect(matchShortcut(GLOBAL_SHORTCUTS, e("k", { ctrlKey: true }))).toBe("palette")
    expect(matchShortcut(GLOBAL_SHORTCUTS, e("k"))).toBeNull()
    expect(matchShortcut(GLOBAL_SHORTCUTS, e("K", { ctrlKey: true, shiftKey: true }))).toBeNull()
    expect(matchShortcut(PLAYER_SHORTCUTS, e("ArrowLeft"))).toBe("frame")
    expect(matchShortcut(PLAYER_SHORTCUTS, e("ArrowLeft", { shiftKey: true }))).toBe("second")
    expect(matchShortcut(PLAYER_SHORTCUTS, e("X", { shiftKey: false }))).toBe("markShot")
    expect(matchShortcut(PLAYER_SHORTCUTS, e("x", { altKey: true }))).toBe("clearMarks")
    expect(matchShortcut(REVIEW_SHORTCUTS, e("7"))).toBe("pick")
  })

  it("finds digits where the layout puts them", () => {
    const e = (key: string, init: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key, ...init })
    // AZERTY: the 1 key types "&", and "1" with Shift.
    expect(matchShortcut(REVIEW_SHORTCUTS, e("&", { code: "Digit1" }))).toBe("pick")
    expect(matchShortcut(REVIEW_SHORTCUTS, e("1", { code: "Digit1", shiftKey: true }))).toBe("pick")
    expect(matchShortcut(GLOBAL_SHORTCUTS, e("&", { code: "Digit1", ctrlKey: true }))).toBe("library")
    expect(matchShortcut(REVIEW_SHORTCUTS, e("3", { code: "Numpad3" }))).toBe("pick")
    // The number pad without Num Lock moves instead.
    expect(matchShortcut(REVIEW_SHORTCUTS, e("End", { code: "Numpad1" }))).toBeNull()
    // And the handler reads the same digit the binding matched.
    expect(digitOf(e("&", { code: "Digit1" }))).toBe("1")
    expect(digitOf(e("End", { code: "Numpad1" }))).toBeNull()
  })

  it("never gives one key press two meanings in the same place, in any preset", () => {
    for (const p of PRESETS) {
      const km = { preset: p.id, own: {} }
      for (const g of SHORTCUT_GROUPS) {
        const seen = new Map<string, string>()
        for (const s of g.shortcuts) {
          s.slots.forEach((_, i) => {
            for (const b of bindingsOf(s, i, km)) {
              expect(seen.get(sig(b)), `${p.id} ${g.title}: ${sig(b)}`).toBeUndefined()
              seen.set(sig(b), slotRef(s, i))
            }
          })
        }
      }
    }
  })

  it("keeps a page's keys apart from the ones that work everywhere, in any preset", () => {
    for (const p of PRESETS) {
      const km = { preset: p.id, own: {} }
      const everywhere = new Set(GLOBAL_SHORTCUTS.flatMap((s) => s.slots.flatMap((_, i) => bindingsOf(s, i, km).map(sig))))
      for (const s of ALL_SHORTCUTS) {
        if (s.scope === "global") continue
        s.slots.forEach((_, i) => {
          for (const b of bindingsOf(s, i, km)) expect(everywhere.has(sig(b)), `${p.id} ${slotRef(s, i)}: ${sig(b)}`).toBe(false)
        })
      }
    }
  })

  it("says which of a shortcut's keys was pressed", () => {
    const e = (key: string, init: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key, ...init })
    expect(matchSlot(PLAYER_SHORTCUTS, e("ArrowLeft"))).toEqual({ id: "frame", slot: 0 })
    expect(matchSlot(PLAYER_SHORTCUTS, e("ArrowRight"))).toEqual({ id: "frame", slot: 1 })
    expect(matchSlot(PLAYER_SHORTCUTS, e("l"))).toEqual({ id: "shuttle", slot: 2 })
    expect(matchSlot(PLAYER_SHORTCUTS, e("O", { shiftKey: true }))).toEqual({ id: "goToMark", slot: 1 })
  })
})

describe("the keymap", () => {
  const e = (key: string, init: KeyboardEventInit = {}) => new KeyboardEvent("keydown", { key, ...init })
  afterEach(() => setKeymap(DEFAULT_KEYMAP))

  it("answers the person's keys instead of the defaults", () => {
    setKeymap({ preset: "autoscene", own: { "player.mute.0": [{ key: "u" }], "player.frame.1": [{ key: "." }] } })
    expect(matchShortcut(PLAYER_SHORTCUTS, e("m"))).toBeNull()
    expect(matchShortcut(PLAYER_SHORTCUTS, e("u"))).toBe("mute")
    expect(matchSlot(PLAYER_SHORTCUTS, e("."))).toEqual({ id: "frame", slot: 1 })
    expect(matchSlot(PLAYER_SHORTCUTS, e("ArrowRight"))).toBeNull()
    expect(matchSlot(PLAYER_SHORTCUTS, e("ArrowLeft"))).toEqual({ id: "frame", slot: 0 })
    // A key taken away leaves the shortcut without one.
    setKeymap({ preset: "autoscene", own: { "player.mute.0": [] } })
    expect(matchShortcut(PLAYER_SHORTCUTS, e("m"))).toBeNull()
  })

  it("starts from an editor's keys", () => {
    setKeymap({ preset: "premiere", own: {} })
    expect(matchShortcut(PLAYER_SHORTCUTS, e("m", { ctrlKey: true }))).toBe("export")
    expect(matchShortcut(PLAYER_SHORTCUTS, e("e", { ctrlKey: true }))).toBeNull()
    expect(matchShortcut(PLAYER_SHORTCUTS, e("X", { ctrlKey: true, shiftKey: true }))).toBe("clearMarks")
    expect(matchShortcut(PLAYER_SHORTCUTS, e("\\"))).toBe("zoomFit")
    // The rest stays AutoScene's.
    expect(matchShortcut(PLAYER_SHORTCUTS, e("i"))).toBe("mark")
    setKeymap({ preset: "resolve", own: {} })
    expect(matchShortcut(PLAYER_SHORTCUTS, e("=", { ctrlKey: true }))).toBe("zoomIn")
    expect(matchShortcut(PLAYER_SHORTCUTS, e("-"))).toBeNull()
  })

  it("never changes the keys that stay", () => {
    setKeymap({ preset: "autoscene", own: { "review.pick.0": [{ key: "q" }], "pack.toggle.0": [] } })
    expect(matchShortcut(REVIEW_SHORTCUTS, e("4"))).toBe("pick")
    expect(matchShortcut(REVIEW_SHORTCUTS, e("q"))).toBeNull()
    expect(matchShortcut(PACK_SHORTCUTS, e(" "))).toBe("toggle")
  })

  it("shows keys as caps, a row's shared modifiers once", () => {
    const second = shortcutOf("player.second")
    expect(capGroups(second)).toEqual([["Shift", "←", "→"]])
    expect(capGroups(shortcutOf("player.shuttle"))).toEqual([["J", "K", "L"]])
    expect(slotCaps(shortcutOf("global.palette"), 0)).toEqual(["Ctrl", "K"])
    expect(slotCaps(shortcutOf("player.zoomOut"), 0)).toEqual(["−"])
    expect(capGroups(shortcutOf("review.pick"))).toEqual([["1 – 9"]])
    setKeymap({ preset: "autoscene", own: { "player.second.1": [{ key: "." }] } })
    expect(capGroups(second)).toEqual([["Shift", "←"], ["."]])
    // A key taken away keeps its place, so the other still reads as "next".
    setKeymap({ preset: "autoscene", own: { "player.frame.0": [] } })
    expect(capGroups(shortcutOf("player.frame"))).toEqual([[], ["→"]])
    setKeymap({ preset: "autoscene", own: { "player.second.0": [], "player.second.1": [] } })
    expect(capGroups(second)).toEqual([])
  })

  it("records a key press the way it is matched", () => {
    expect(bindingOf(e("K", { ctrlKey: true, shiftKey: true }))).toEqual({ key: "k", mod: true, shift: true })
    expect(bindingOf(e(" "))).toEqual({ key: " " })
    // AZERTY: the digit row records the digit it stands for.
    expect(bindingOf(e("&", { code: "Digit1" }))).toEqual({ key: "1" })
    // Only modifiers, or a dead key: nothing yet.
    expect(bindingOf(e("Control", { ctrlKey: true }))).toBeNull()
    expect(bindingOf(e("Shift", { shiftKey: true }))).toBeNull()
    expect(bindingOf(e("Dead"))).toBeNull()
    for (const ev of [
      e("x", { altKey: true }),
      e("ArrowLeft", { shiftKey: true }),
      e("!", { code: "Digit1", shiftKey: true }),
      e("F5"),
      e("+", { shiftKey: true }),
    ]) {
      const b = bindingOf(ev)
      expect(b && matches(b, ev), ev.key).toBe(true)
    }
  })
})

describe("useShortcuts", () => {
  it("answers a key and claims it", () => {
    const yes = vi.fn()
    renderHook(() => useShortcuts(REVIEW_SHORTCUTS, { yes }))
    expect(press("y").defaultPrevented).toBe(true)
    expect(yes).toHaveBeenCalledOnce()
  })

  it("lets a key through when the handler has nothing to do", () => {
    renderHook(() => useShortcuts(REVIEW_SHORTCUTS, { pick: () => false }))
    expect(press("3").defaultPrevented).toBe(false)
  })

  it("leaves typing, open dialogs and focused buttons alone", () => {
    const palette = vi.fn()
    const sidebar = vi.fn()
    renderHook(() => useShortcuts(GLOBAL_SHORTCUTS, { palette, sidebar }, { anywhere: ["palette"] }))
    const field = document.body.appendChild(document.createElement("input"))
    press("b", { ctrlKey: true }, field)
    expect(sidebar).not.toHaveBeenCalled()
    press("k", { ctrlKey: true }, field)
    expect(palette).toHaveBeenCalledOnce()

    const dialog = document.body.appendChild(document.createElement("div"))
    dialog.setAttribute("role", "dialog")
    dialog.dataset.state = "open"
    press("b", { ctrlKey: true })
    expect(sidebar).not.toHaveBeenCalled()
    dialog.remove()
    press("b", { ctrlKey: true })
    expect(sidebar).toHaveBeenCalledOnce()
  })

  it("lets a focused button take Space", () => {
    const playPause = vi.fn()
    renderHook(() => useShortcuts(PLAYER_SHORTCUTS, { playPause }))
    const button = document.body.appendChild(document.createElement("button"))
    press(" ", {}, button)
    expect(playPause).not.toHaveBeenCalled()
    press(" ")
    expect(playPause).toHaveBeenCalledOnce()
  })

  it("repeats a held key only where asked", () => {
    const yes = vi.fn()
    const frame = vi.fn()
    renderHook(() => useShortcuts(REVIEW_SHORTCUTS, { yes }))
    renderHook(() => useShortcuts(PLAYER_SHORTCUTS, { frame }, { repeat: true }))
    press("y", { repeat: true })
    press("ArrowRight", { repeat: true })
    expect(yes).not.toHaveBeenCalled()
    expect(frame).toHaveBeenCalledOnce()
  })

  it("tells the handler which of the keys it was", () => {
    const frame = vi.fn()
    renderHook(() => useShortcuts(PLAYER_SHORTCUTS, { frame }))
    press("ArrowLeft")
    press("ArrowRight")
    expect(frame.mock.calls.map((c) => c[1])).toEqual([0, 1])
  })

  it("uses the latest handler without listening again", () => {
    const first = vi.fn()
    const second = vi.fn()
    const { rerender } = renderHook(({ fn }) => useShortcuts(REVIEW_SHORTCUTS, { yes: fn }), { initialProps: { fn: first } })
    rerender({ fn: second })
    press("y")
    expect(first).not.toHaveBeenCalled()
    expect(second).toHaveBeenCalledOnce()
  })
})
