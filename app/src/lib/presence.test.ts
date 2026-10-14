import { describe, expect, it } from "vitest"
import type { Character, Presence } from "@/bindings"
import { charactersIn, lanesOf } from "./presence"

const who = (id: string, hidden = false): Character => ({
  id,
  name: null,
  label: id,
  hidden,
  tracks: 0,
  shots: 0,
  media: 1,
  screenSec: 0,
  cover: null,
  unsure: 0,
  waiting: 0,
})
const at = (characterId: string, start: number, end: number, trackId = start): Presence => ({ characterId, trackId, start, end })
const cast = new Map([
  ["a", who("a")],
  ["b", who("b")],
  ["h", who("h", true)],
])

describe("presence lanes", () => {
  const lanes = lanesOf([at("a", 0, 10), at("b", 5, 40), at("a", 50, 60), at("h", 0, 100), at("gone", 0, 5)], cast)

  it("groups spans per character, most on screen first, without hidden or unknown ones", () => {
    expect(lanes.map((l) => [l.character.id, l.frames, l.spans.length])).toEqual([
      ["b", 35, 1],
      ["a", 20, 2],
    ])
  })

  it("tells who is on screen in a range", () => {
    expect(charactersIn(lanes, 0, 5).map((l) => l.character.id)).toEqual(["a"])
    expect(charactersIn(lanes, 8, 9).map((l) => l.character.id)).toEqual(["b", "a"])
    expect(charactersIn(lanes, 40, 50)).toEqual([])
  })
})
