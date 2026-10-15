import { cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import {
  type Shortcut,
  GLOBAL_SHORTCUTS,
  PLAYER_SHORTCUTS,
  REVIEW_SHORTCUTS,
  SHORTCUT_GROUPS,
  matchShortcut,
  useShortcuts,
} from "./shortcuts"

afterEach(() => {
  cleanup()
  document.body.innerHTML = ""
})

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

  it("never gives one key press two meanings in the same place", () => {
    for (const g of SHORTCUT_GROUPS) {
      const seen = new Map<string, string>()
      for (const s of g.shortcuts as Shortcut[]) {
        for (const b of s.bindings) {
          const sig = `${b.mod ? "mod+" : ""}${b.shift ? "shift+" : ""}${b.alt ? "alt+" : ""}${b.key}`
          expect(seen.get(sig), `${g.title}: ${sig}`).toBeUndefined()
          seen.set(sig, s.id)
        }
      }
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
