import { describe, expect, it } from "vitest"
import { suggestTitle } from "./suggest-title"

describe("suggestTitle", () => {
  it("cleans release names", () => {
    expect(suggestTitle(String.raw`D:\Anime\[SubsPlease] Sousou no Frieren - 03 (1080p) [ABCD].mkv`, true)).toBe(
      "Sousou no Frieren",
    )
    expect(suggestTitle("/tv/Breaking.Bad.S02E09.720p.mkv", true)).toBe("Breaking Bad")
    expect(suggestTitle(String.raw`D:\Movies\Spirited Away (2001).mkv`, true)).toBe("Spirited Away")
  })

  it("names season folders after the show", () => {
    expect(suggestTitle(String.raw`D:\Video\Dark\Season 1`, false)).toBe("Dark")
    expect(suggestTitle("/Volumes/TV/The Office/S03/", false)).toBe("The Office")
    expect(suggestTitle(String.raw`D:\Video\Dark\Staffel 2\Dark.S02E01.mkv`, true)).toBe("Dark")
    expect(suggestTitle(String.raw`D:\Video\Frieren`, false)).toBe("Frieren")
  })
})
