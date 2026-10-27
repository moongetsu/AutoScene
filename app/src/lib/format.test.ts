import { describe, expect, it } from "vitest"
import { shortFormatName } from "./format"

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
