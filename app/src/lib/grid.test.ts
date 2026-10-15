import { describe, expect, it } from "vitest"
import { clickSelect, gridColumns } from "./grid"

describe("gridColumns", () => {
  it("fits as many columns as the minimum width allows, sharing the rest", () => {
    expect(gridColumns(1000, 240, 16)).toEqual({ columns: 3, itemWidth: (1000 - 32) / 3 })
    // Exactly four fit: 4 × 240 + 3 × 16 = 1008.
    expect(gridColumns(1008, 240, 16).columns).toBe(4)
    expect(gridColumns(100, 240, 16)).toEqual({ columns: 1, itemWidth: 100 })
    expect(gridColumns(0, 240, 16).columns).toBe(1)
  })
})

describe("clickSelect", () => {
  const keys = ["a", "b", "c", "d", "e"]
  const click = (selected: string[], anchor: string | null, index: number, shift = false, toggle = false) =>
    clickSelect(keys, new Set(selected), anchor, index, { shift, toggle })

  it("starts a selection with a plain click", () => {
    expect(click([], null, 2)).toEqual({ selected: new Set(["c"]), anchor: "c" })
  })

  it("flips one once something is selected, or with Ctrl/⌘", () => {
    expect(click(["c"], "c", 4).selected).toEqual(new Set(["c", "e"]))
    expect(click(["c", "e"], "e", 2).selected).toEqual(new Set(["e"]))
    expect(click([], null, 1, false, true).selected).toEqual(new Set(["b"]))
  })

  it("adds the range from the anchor with Shift, either direction, keeping the anchor", () => {
    expect(click(["b"], "b", 3, true)).toEqual({ selected: new Set(["b", "c", "d"]), anchor: "b" })
    expect(click(["d"], "d", 0, true).selected).toEqual(new Set(["a", "b", "c", "d"]))
    // An anchor that left the list: just the clicked one.
    expect(click([], "z", 1, true).selected).toEqual(new Set(["b"]))
  })
})
