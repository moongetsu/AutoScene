import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { Character } from "@/bindings"
import { installBrowserGuards } from "@/lib/browser-guards"
import { CharacterCard } from "./CharacterCard"

// The router's link as a plain one that, like it, skips its onClick once the
// click's default is prevented: what matters is which clicks reach the card.
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to: _to, params: _params, onClick, ...props }: React.ComponentProps<"a"> & Record<string, unknown>) => (
    <a href="#walter" {...props} onClick={(e) => !e.defaultPrevented && onClick?.(e)} />
  ),
  useNavigate: () => () => {},
}))

afterEach(cleanup)

const walter: Character = {
  id: "walter",
  name: "Walter",
  label: "Walter",
  hidden: false,
  tracks: 3,
  shots: 3,
  media: 1,
  screenSec: 30,
  cover: null,
  coverSmall: null,
  unsure: 0,
  waiting: 0,
}

describe("CharacterCard", () => {
  it("lets the selection take a Ctrl/⌘- or Shift-click, though the browser guards stop it first", () => {
    const remove = installBrowserGuards({ dev: false, mac: false })
    const onPick = vi.fn(() => true)
    render(<CharacterCard pid="p" character={walter} onPick={onPick} />)
    const card = screen.getByRole("link")
    expect(fireEvent.click(card, { ctrlKey: true })).toBe(false)
    expect(fireEvent.click(card, { shiftKey: true })).toBe(false)
    expect(onPick).toHaveBeenCalledTimes(2)
    remove()
  })

  it("opens the character on a click the selection doesn't take", () => {
    const onPick = vi.fn(() => false)
    render(<CharacterCard pid="p" character={walter} onPick={onPick} />)
    // `fireEvent` returns false once the click's default was prevented.
    expect(fireEvent.click(screen.getByRole("link"))).toBe(true)
    onPick.mockReturnValue(true)
    expect(fireEvent.click(screen.getByRole("link"))).toBe(false)
  })

  it("shows a selected card as selected", () => {
    render(<CharacterCard pid="p" character={walter} selected selecting onPick={() => true} />)
    expect(screen.getByText("Selected")).toBeTruthy()
  })
})
