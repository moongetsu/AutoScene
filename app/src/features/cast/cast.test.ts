import { describe, expect, it } from "vitest"
import { formatScreenTime, plural } from "./cast"

describe("cast wording", () => {
  it("says screen time the way people do", () => {
    expect([0, 42.4, 61, 59 * 60, 3725].map(formatScreenTime)).toEqual(["0 s", "42 s", "1 min", "59 min", "1 h 02 min"])
  })

  it("counts", () => {
    expect([plural(1, "shot"), plural(1414, "face"), plural(2, "episode")]).toEqual(["1 shot", "1,414 faces", "2 episodes"])
  })
})
