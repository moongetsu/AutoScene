import { describe, expect, it } from "vitest"
import type { Cast, MediaItem } from "@/bindings"
import { characterEntries, episodeEntries, episodeKeywords } from "./palette-items"

describe("palette entries", () => {
  it("finds an episode by its number written any common way, or by its file", () => {
    const words = episodeKeywords({ season: 1, episode: 3, fileName: "Breaking.Bad.S01E03.1080p.mkv" })
    for (const w of ["S01E03", "S1E3", "1x03", "E03", "Episode 3", "Breaking.Bad.S01E03.1080p"]) {
      expect(words).toContain(w)
    }
    // A movie has its file only.
    expect(episodeKeywords({ season: null, episode: null, fileName: "Blade Runner 2049.mkv" })).toEqual([
      "Blade Runner 2049",
    ])
  })

  it("lists the cast without hidden characters, and episodes with what is wrong", () => {
    const character = (id: string, hidden: boolean, name: string | null) =>
      ({ id, label: name ?? "Character 7", name, hidden, screenSec: 90, shots: 4, coverSmall: null }) as never
    const cast = { characters: [character("a", false, "Walter"), character("b", true, "Extra"), character("c", false, null)] }
    expect(characterEntries(cast as unknown as Cast).map((c) => [c.id, c.hint, c.keywords])).toEqual([
      ["a", "2 min · 4 shots", []],
      ["c", "2 min · 4 shots", ["unnamed"]],
    ])
    const media = [{ id: "m", label: "S01E03", fileName: "x.mkv", season: 1, episode: 3, status: "missing" }]
    expect(episodeEntries(media as unknown as MediaItem[])[0]).toMatchObject({ label: "S01E03 · x", hint: "File missing" })
  })
})
