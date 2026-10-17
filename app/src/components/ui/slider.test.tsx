import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { Slider } from "./slider"

afterEach(cleanup)

describe("Slider", () => {
  it("slides with the main button only: a right-click leaves the value (and opens the menu)", () => {
    const onChange = vi.fn()
    render(<Slider label="Volume" value={0.7} onChange={onChange} />)
    // A press on the bar lands on its track, inside the slider's root.
    const root = screen.getByRole("slider", { name: "Volume" }).closest("[data-orientation]")!.parentElement!
    fireEvent.pointerDown(root, { button: 2, pointerId: 1, clientX: 10 })
    expect(onChange).not.toHaveBeenCalled()
    fireEvent.pointerDown(root, { button: 0, pointerId: 1, clientX: 10 })
    expect(onChange).toHaveBeenCalled()
  })
})
