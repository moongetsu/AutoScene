import type * as React from "react"
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import type { AppSettings } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { commands } from "@/lib/ipc"
import { queryClient } from "@/lib/queries"
import { DEFAULT_KEYMAP, getKeymap, setKeymap } from "@/lib/shortcuts"
import { ShortcutsSection } from "./ShortcutsSection"

const settings: AppSettings = { libraryRoot: "C:\\Library", exportPresetId: "prores-hq", audioLanguage: "eng" }

vi.mock("@/lib/ipc", () => ({
  commands: {
    settingsGet: vi.fn(() => Promise.resolve(settings)),
    settingsUpdate: vi.fn((s: AppSettings) => Promise.resolve(s)),
    menuSetKeys: vi.fn(() => Promise.resolve(null)),
  },
  errorMessage: (e: unknown) => String(e),
}))

const wrapper = ({ children }: { children: React.ReactNode }) => <TooltipProvider>{children}</TooltipProvider>
const key = (k: string, init: KeyboardEventInit = {}) =>
  act(() => void window.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init })))
const saved = () => vi.mocked(commands.settingsUpdate).mock.lastCall?.[0].shortcuts

beforeEach(() => {
  queryClient.clear()
  vi.mocked(commands.settingsUpdate).mockClear()
})
afterEach(() => {
  cleanup()
  setKeymap(DEFAULT_KEYMAP)
})

describe("ShortcutsSection", () => {
  it("records a new key and keeps it", async () => {
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Mute: M. Change" }))
    const listening = screen.getByRole("button", { name: "Press the new key for “Mute”" })
    expect(within(listening).getByText("Press a key")).toBeTruthy()
    // Modifiers alone only wait, shown as held.
    key("Shift", { shiftKey: true })
    expect(within(listening).getByText("Shift")).toBeTruthy()
    key("U", { shiftKey: true })
    await waitFor(() => expect(saved()).toEqual({ preset: "", keys: { "player.mute.0": [{ key: "u", shift: true }] } }))
    expect(screen.getByRole("button", { name: "Mute: Shift U. Change" })).toBeTruthy()
    expect(screen.getByRole("button", { name: "Reset “Mute”" })).toBeTruthy()
    expect(screen.getByText("1 key differs from AutoScene's.")).toBeTruthy()
  })

  it("takes only a fresh press, and keeps a held key's repeats to itself", () => {
    const after = vi.fn()
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Mute: M. Change" }))
    // Still held from before (Enter that started it): not a new key.
    key("Enter", { code: "Enter", repeat: true })
    expect(screen.getByRole("button", { name: "Press the new key for “Mute”" })).toBeTruthy()
    key("u", { code: "KeyU" })
    expect(getKeymap().own).toEqual({ "player.mute.0": [{ key: "u" }] })
    // Held on: its repeats and its release reach no one; the next press does.
    window.addEventListener("keydown", after)
    key("u", { code: "KeyU", repeat: true })
    act(() => void window.dispatchEvent(new KeyboardEvent("keyup", { key: "u", code: "KeyU", bubbles: true })))
    key("u", { code: "KeyU" })
    window.removeEventListener("keydown", after)
    expect(after).toHaveBeenCalledOnce()
  })

  it("keeps the last of quick changes, saved in order", async () => {
    const order: string[] = []
    let release: () => void = () => {}
    vi.mocked(commands.settingsUpdate).mockImplementationOnce(
      (s: AppSettings) =>
        new Promise((done) => {
          order.push("first")
          release = () => done(s)
        }),
    )
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Mute: M. Change" }))
    key("u")
    fireEvent.click(screen.getByRole("button", { name: "Mute: U. Change" }))
    key("q")
    await waitFor(() => expect(order).toEqual(["first"]))
    // The second waits for the first.
    expect(commands.settingsUpdate).toHaveBeenCalledOnce()
    await act(async () => release())
    await waitFor(() => expect(commands.settingsUpdate).toHaveBeenCalledTimes(2))
    expect(saved()).toEqual({ preset: "", keys: { "player.mute.0": [{ key: "q" }] } })
    expect(getKeymap().own).toEqual({ "player.mute.0": [{ key: "q" }] })
  })

  it("lets go on Esc, keeping the key", () => {
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Mute: M. Change" }))
    key("Escape")
    expect(screen.getByRole("button", { name: "Mute: M. Change" })).toBeTruthy()
    expect(commands.settingsUpdate).not.toHaveBeenCalled()
  })

  it("has every key to itself while it listens", () => {
    const palette = vi.fn()
    window.addEventListener("keydown", palette)
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Mute: M. Change" }))
    key("k", { ctrlKey: true })
    window.removeEventListener("keydown", palette)
    expect(palette).not.toHaveBeenCalled()
  })

  it("says why a key can't be one, and keeps listening", () => {
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Command palette: Ctrl K. Change" }))
    key("p")
    // Shown in the slot, and read out.
    const why = screen.getAllByText("Keys that work everywhere need Ctrl")
    expect(why.map((el) => el.getAttribute("role") ?? el.tagName)).toEqual(["BUTTON", "status"])
    key("p", { ctrlKey: true })
    expect(getKeymap().own).toEqual({ "global.palette.0": [{ key: "p", mod: true }] })
  })

  it("takes a key from the shortcut that had it", () => {
    render(<ShortcutsSection />, { wrapper })
    fireEvent.click(screen.getByRole("button", { name: "Mute: M. Change" }))
    key("x")
    expect(getKeymap().own).toEqual({ "player.mute.0": [{ key: "x" }], "player.markShot.0": [] })
    expect(screen.getByRole("button", { name: "Mark the shot as the clip: no key. Change" })).toBeTruthy()
    // Its reset takes the key back.
    fireEvent.click(screen.getByRole("button", { name: "Reset “Mark the shot as the clip”" }))
    expect(getKeymap().own).toEqual({ "player.mute.0": [] })
  })

  it("starts from an editor's keys and lets the person's changes go", () => {
    setKeymap({ preset: "autoscene", own: { "player.mute.0": [{ key: "u" }] } })
    render(<ShortcutsSection />, { wrapper })
    fireEvent.mouseDown(screen.getByRole("radio", { name: "Premiere Pro" }))
    fireEvent.click(screen.getByRole("radio", { name: "Premiere Pro" }))
    expect(getKeymap()).toEqual({ preset: "premiere", own: {} })
    expect(screen.getByRole("button", { name: "Export the marked clip: Ctrl M. Change" })).toBeTruthy()
    expect(screen.queryByText(/differs? from/)).toBeNull()
  })

  it("puts everything back on Reset all", () => {
    setKeymap({ preset: "resolve", own: { "player.mute.0": [{ key: "u" }], "player.goTo.0": [] } })
    render(<ShortcutsSection />, { wrapper })
    expect(screen.getByText("2 keys differ from DaVinci Resolve's.")).toBeTruthy()
    fireEvent.click(screen.getByRole("button", { name: "Reset all" }))
    expect(getKeymap()).toEqual({ preset: "resolve", own: {} })
  })

  it("shows the keys that stay without a way to change them", () => {
    render(<ShortcutsSection />, { wrapper })
    expect(screen.getByText("1 – 9")).toBeTruthy()
    expect(screen.queryByRole("button", { name: /^It's character 1 to 9/ })).toBeNull()
  })
})
