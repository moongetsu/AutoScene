import { describe, expect, it } from "vitest"
import type { FaceTrackItem } from "@/bindings"
import { formatScreenTime, namesCredit, namesSource, pictureChoices, plural, unplacedNote } from "./cast"

describe("cast wording", () => {
  it("says screen time the way people do", () => {
    expect([0, 42.4, 61, 59 * 60, 3725].map(formatScreenTime)).toEqual(["0 s", "42 s", "1 min", "59 min", "1 h 02 min"])
  })

  it("counts", () => {
    expect([plural(1, "shot"), plural(1414, "face"), plural(2, "episode")]).toEqual(["1 shot", "1,414 faces", "2 episodes"])
  })

  it("says how many decisions wait for their faces, and that they're tried again", () => {
    expect([unplacedNote(0), unplacedNote(-1), unplacedNote(Number.NaN)]).toEqual([null, null, null])
    expect(unplacedNote(1)).toBe(
      "1 of your decisions waits for a face the latest analysis didn't find again. It's tried again at the next analysis.",
    )
    expect(unplacedNote(1414)).toBe(
      "1,414 of your decisions wait for faces the latest analysis didn't find again. They're tried again at the next analysis.",
    )
  })
})

describe("names source", () => {
  it("names anime from AniList, an animated series from TVmaze, an animated film from Fandom", () => {
    const kinds = ["series", "movie"] as const
    const of = (content: string) => kinds.map((k) => namesSource(content as never, k))
    expect(of("anime")).toEqual(["AniList", "AniList"])
    expect(of("animation")).toEqual(["TVmaze", "Fandom"])
    expect([...of("live"), ...of("mixed"), namesSource(undefined, undefined)]).toEqual([null, null, null, null, null])
    expect([namesCredit("AniList"), namesCredit(null)]).toEqual([null, null])
    expect(namesCredit("Fandom")).toContain("CC BY-SA")
  })
})

describe("picture choices", () => {
  const face = (id: number, quality: number | null, faceHeight: number | null = 0.2): FaceTrackItem => ({
    id,
    mediaId: "m",
    start: id * 10,
    end: id * 10 + 5,
    quality,
    faceHeight,
    thumb: `faces/${id}.jpg`,
    characterId: "c",
    assign: "auto",
    score: 1,
  })
  const ids = (list: FaceTrackItem[]) => list.map((t) => t.id)

  it("offers the clearest faces first, the bigger of two as clear, unknown ones last", () => {
    const list = [face(1, 0.5), face(2, null), face(3, 0.9), face(4, 0.9, 0.4), face(5, 0.7)]
    expect(ids(pictureChoices(list, null, 10))).toEqual([4, 3, 5, 1, 2])
    expect(ids(pictureChoices(list, null, 2))).toEqual([4, 3])
    // The list it was given keeps its order.
    expect(ids(list)).toEqual([1, 2, 3, 4, 5])
  })

  it("puts their picture now first, even when it isn't among the clearest", () => {
    const list = [face(1, 0.5), face(2, 0.9), face(3, 0.2)]
    expect(ids(pictureChoices(list, "faces/3.jpg", 2))).toEqual([3, 2])
    // A picture that isn't one of these faces changes nothing.
    expect(ids(pictureChoices(list, "faces/9.jpg", 2))).toEqual([2, 1])
  })
})
