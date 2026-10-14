import { describe, expect, it } from "vitest"
import { cellAt, formatShotLength, restingCell, spriteStyle } from "./shots"

describe("shot sprites", () => {
  it("rest on the middle of the shot", () => {
    expect(restingCell(8)).toBe(3)
    expect(restingCell(1)).toBe(0)
    expect(restingCell(0)).toBe(0)
  })

  it("follow the pointer across the card", () => {
    expect(cellAt(0, 200, 8)).toBe(0)
    expect(cellAt(24, 200, 8)).toBe(0)
    expect(cellAt(25, 200, 8)).toBe(1)
    expect(cellAt(199.9, 200, 8)).toBe(7)
    // Outside the card (a pointer captured past its edge) stays on the ends.
    expect(cellAt(-5, 200, 8)).toBe(0)
    expect(cellAt(260, 200, 8)).toBe(7)
    expect(cellAt(50, 0, 8)).toBe(0)
  })
})

describe("formatShotLength", () => {
  it("says how long a shot is the way editors read it", () => {
    expect(formatShotLength(1 / 24)).toBe("<0.1 s")
    expect(formatShotLength(0.4)).toBe("0.4 s")
    expect(formatShotLength(2.46)).toBe("2.5 s")
    expect(formatShotLength(12.4)).toBe("12 s")
    expect(formatShotLength(65)).toBe("1:05")
    expect(formatShotLength(Number.NaN)).toBe("")
  })
})

describe("spriteStyle", () => {
  it("fills the card's width with wide cells and its height with tall ones", () => {
    expect(spriteStyle(8, 0, 16 / 9).width).toBe("800%")
    expect(spriteStyle(8, 0, 2.39).width).toBe("800%")
    expect(spriteStyle(8, 0, 4 / 3).height).toBe("100%")
    expect(spriteStyle(8, 0, 300 / 8 / 22).width).toBe("800%")
  })

  it("centres the cell shown", () => {
    // 8 cells: cell 0's centre is 3.5 cells (43.75% of the strip) left of the strip's.
    expect(spriteStyle(8, 0, 16 / 9).transform).toBe("translate(calc(-50% + 43.75%), -50%)")
    expect(spriteStyle(8, 7, 16 / 9).transform).toBe("translate(calc(-50% + -43.75%), -50%)")
    expect(spriteStyle(1, 0, 16 / 9).transform).toBe("translate(calc(-50% + 0%), -50%)")
  })
})
