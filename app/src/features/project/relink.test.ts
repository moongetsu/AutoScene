import { describe, expect, it } from "vitest"
import { folderResult } from "./relink"

describe("looking for missing episodes in a folder", () => {
  it("says what was found, what is checked again and what is still missing", () => {
    expect(folderResult({ same: 1, reindex: 0, missing: 0 }, 1)).toEqual({ title: "Found the episode" })
    expect(folderResult({ same: 2, reindex: 1, missing: 0 }, 3)).toEqual({
      title: "Found all 3 episodes",
      description: "1 is another copy, indexed again.",
    })
    expect(folderResult({ same: 1, reindex: 0, missing: 2 }, 3)).toEqual({
      title: "Found 1 of 3 episodes",
      description: "2 episodes still can't be found.",
    })
    expect(folderResult({ same: 0, reindex: 0, missing: 2 }, 2).title).toBe("None of them are in that folder")
    expect(folderResult({ same: 1, reindex: 0, missing: 1 }, 2, true).title).toBe("Found 1 of 2 files")
  })
})
