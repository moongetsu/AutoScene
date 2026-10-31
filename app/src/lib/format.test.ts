import { describe, expect, it } from "vitest"
import { formatEta, formatProgress, shortFormatName } from "./format"

describe("shortFormatName", () => {
  it("drops what the brackets add", () => {
    expect(shortFormatName("H.264 · Mobile (CapCut, Alight Motion)")).toBe("H.264 · Mobile")
    expect(shortFormatName("H.264 · All-Intra (Twixtor)")).toBe("H.264 · All-Intra")
    expect(shortFormatName("HEVC 10-bit (keeps HDR)")).toBe("HEVC 10-bit")
  })

  it("keeps a name without them, or that is nothing but them", () => {
    expect(shortFormatName("ProRes 422 HQ")).toBe("ProRes 422 HQ")
    expect(shortFormatName("(custom)")).toBe("(custom)")
  })
})

describe("formatProgress", () => {
  it("gives the time left before the percentage, as the Activity drawer does", () => {
    expect(formatProgress(0.42, 150_000)).toBe("about 3 min left · 42%")
    expect(formatProgress(0.42, 30_000)).toBe("30 s left · 42%")
    expect(formatProgress(0.99, 2_000)).toBe("almost done · 99%")
  })

  it("leaves out what isn't known yet", () => {
    expect(formatProgress(0.42, null)).toBe("42%")
    expect(formatProgress(null, null)).toBe("")
    expect(formatEta(Number.NaN)).toBe("")
  })
})
