import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { Tooltip, TooltipProvider, TruncatedText } from "./tooltip"

afterEach(cleanup)

/** Make an element as wide as its text needs (`scroll`) and as it is shown (`client`). */
function size(el: HTMLElement, scroll: number, client: number) {
  Object.defineProperty(el, "scrollWidth", { configurable: true, value: scroll })
  Object.defineProperty(el, "clientWidth", { configurable: true, value: client })
}

describe("Tooltip", () => {
  it("shows its label and hint, styled, never as a browser title", () => {
    render(
      <TooltipProvider delayDuration={0}>
        <Tooltip label="S01E01 · 00:40:03:14" hint="Click to watch">
          <button type="button">Shot</button>
        </Tooltip>
      </TooltipProvider>,
    )
    const button = screen.getByRole("button", { name: "Shot" })
    expect(button.getAttribute("title")).toBeNull()
    fireEvent.focus(button)
    expect(screen.getAllByText("S01E01 · 00:40:03:14").length).toBeGreaterThan(0)
    expect(screen.getAllByText("Click to watch").length).toBeGreaterThan(0)
  })

  it("doesn't show while disabled, and its element stays the same one", () => {
    const tip = (disabled: boolean) => (
      <TooltipProvider delayDuration={0}>
        <Tooltip label="S01E01 · 00:40:03:14" disabled={disabled}>
          <button type="button">Shot</button>
        </Tooltip>
      </TooltipProvider>
    )
    const { rerender } = render(tip(true))
    const button = screen.getByRole("button", { name: "Shot" })
    button.focus()
    fireEvent.focus(button)
    expect(screen.queryByText("S01E01 · 00:40:03:14")).toBeNull()
    // The label arrives (the media loaded): the button isn't mounted again, so it keeps the focus.
    rerender(tip(false))
    expect(screen.getByRole("button", { name: "Shot" })).toBe(button)
    expect(document.activeElement).toBe(button)
    fireEvent.blur(button)
    fireEvent.focus(button)
    expect(screen.getAllByText("S01E01 · 00:40:03:14").length).toBeGreaterThan(0)
  })

  it("passes other props on to its element (a right-click's handler from an area around it)", () => {
    const onContextMenu = vi.fn()
    render(
      <TooltipProvider delayDuration={0}>
        <Tooltip label="Watch" onContextMenu={onContextMenu}>
          <button type="button">Shot</button>
        </Tooltip>
      </TooltipProvider>,
    )
    fireEvent.contextMenu(screen.getByRole("button", { name: "Shot" }))
    expect(onContextMenu).toHaveBeenCalledOnce()
  })
})

describe("TruncatedText", () => {
  it("shows all of the text only when it is cut", () => {
    render(
      <TooltipProvider delayDuration={0}>
        <TruncatedText tabIndex={0}>/Exports/Breaking Bad/Walter</TruncatedText>
      </TooltipProvider>,
    )
    const text = screen.getByText("/Exports/Breaking Bad/Walter")
    // It fits: no tooltip.
    size(text, 100, 100)
    fireEvent.focus(text)
    expect(screen.queryAllByText("/Exports/Breaking Bad/Walter")).toHaveLength(1)
    fireEvent.blur(text)
    // It is cut: the whole path in a tooltip.
    size(text, 300, 100)
    fireEvent.focus(text)
    expect(screen.queryAllByText("/Exports/Breaking Bad/Walter").length).toBeGreaterThan(1)
  })

  it("can show something else than the text (a path behind a name)", () => {
    render(
      <TooltipProvider delayDuration={0}>
        <TruncatedText tabIndex={0} label="/Clips/walter_s01e01.mov">
          walter_s01e01.mov
        </TruncatedText>
      </TooltipProvider>,
    )
    const text = screen.getByText("walter_s01e01.mov")
    size(text, 300, 100)
    fireEvent.focus(text)
    expect(screen.getAllByText("/Clips/walter_s01e01.mov").length).toBeGreaterThan(0)
  })
})
