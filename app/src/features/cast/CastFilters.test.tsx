import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { CastFilters } from "./CastFilters"
import { ALL_CAST } from "./cast"

afterEach(cleanup)

describe("CastFilters", () => {
  it("offers To review only while someone has faces to review, or while it is chosen", () => {
    const { rerender } = render(<CastFilters find={ALL_CAST} review={false} onChange={() => {}} />)
    expect(screen.queryByRole("radio", { name: "To review" })).toBeNull()
    rerender(<CastFilters find={ALL_CAST} review onChange={() => {}} />)
    expect(screen.getByRole("radio", { name: "To review" })).toBeTruthy()
    rerender(<CastFilters find={{ ...ALL_CAST, show: "review" }} review={false} onChange={() => {}} />)
    expect(screen.getByRole("radio", { name: "To review" }).getAttribute("aria-checked")).toBe("true")
  })

  it("searches as one types, and Esc clears the search", () => {
    const onChange = vi.fn()
    const { rerender } = render(<CastFilters find={ALL_CAST} review={false} onChange={onChange} />)
    const field = screen.getByLabelText("Find a character")
    fireEvent.change(field, { target: { value: "wal" } })
    expect(onChange).toHaveBeenLastCalledWith({ ...ALL_CAST, query: "wal" })
    rerender(<CastFilters find={{ ...ALL_CAST, query: "wal" }} review={false} onChange={onChange} />)
    fireEvent.keyDown(field, { key: "Escape" })
    expect(onChange).toHaveBeenLastCalledWith(ALL_CAST)
  })
})
