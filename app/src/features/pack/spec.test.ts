import { describe, expect, it } from "vitest"
import type { PackChoices, PackClip, PackPreview } from "@/bindings"
import {
  DEFAULT_SPEC,
  between,
  fits,
  isDefault,
  keyOf,
  lengthOf,
  picksFor,
  shown,
  totalsOf,
  withChoices,
  withIncluded,
  withPicks,
} from "./spec"

const clip = (start: number, more: Partial<PackClip> = {}): PackClip => {
  const byRules = more.byRules ?? true
  const manual = more.manual ?? null
  const included = more.included ?? manual ?? byRules
  return {
    shot: { mediaId: "m1", start, end: start + 48, sprite: null, cells: 0, trackId: 1, face: "", faceHeight: 0.3 },
    start,
    end: start + 48,
    size: "close_up",
    quality: 0.8,
    coverage: 1,
    others: 0,
    score: 0.7,
    included,
    byRules,
    reason: included ? null : manual === false ? "removed" : "short",
    manual,
    ...more,
  }
}

const preview = (clips: PackClip[]): PackPreview => ({ clips, totals: totalsOf(clips, () => 24), stamp: "s", shape: "a" })

describe("picks", () => {
  it("picks by hand only against the filters, and hands back where they agree", () => {
    const kept = clip(0)
    const dropped = clip(100, { byRules: false })
    expect(picksFor([kept], false)).toEqual({ pick: [kept], unpick: [] })
    expect(picksFor([dropped], true)).toEqual({ pick: [dropped], unpick: [] })

    // Taken out by hand, then in again: the pick goes, the filters decide.
    const takenOut = clip(0, { manual: false })
    expect(picksFor([takenOut], true)).toEqual({ pick: [], unpick: [takenOut] })
    // Already as asked: nothing to do.
    expect(picksFor([kept], true)).toEqual({ pick: [], unpick: [] })
    // A missing file can't be taken.
    expect(picksFor([clip(0, { reason: "missing", included: false })], true)).toEqual({ pick: [], unpick: [] })
  })

  it("takes every shot between two cards, either way round", () => {
    const list = [clip(0), clip(100), clip(200), clip(300)]
    expect(between(list, keyOf(list[1]), keyOf(list[3])).map((c) => c.start)).toEqual([100, 200, 300])
    expect(between(list, keyOf(list[2]), keyOf(list[0])).map((c) => c.start)).toEqual([0, 100, 200])
    // An anchor that scrolled out of the list (another view): just the card.
    expect(between(list, "m9:0", keyOf(list[2])).map((c) => c.start)).toEqual([200])
  })

  it("shows a click at once, totals too, and leaves the other cards alone", () => {
    const list = [clip(0), clip(100), clip(200, { byRules: false })]
    const p = preview(list)
    expect(p.totals).toEqual({ clips: 2, frames: 96, seconds: 4 })

    const out = withIncluded(p, [list[0]], false, () => 24)
    expect(out.clips[0]).toMatchObject({ included: false, manual: false, reason: "removed" })
    expect(out.clips[1]).toBe(list[1])
    expect(out.totals).toEqual({ clips: 1, frames: 48, seconds: 2 })

    const back = withIncluded(out, [out.clips[0]], true, () => 24)
    expect(back.clips[0]).toMatchObject({ included: true, manual: null, reason: null })
    const added = withIncluded(p, [list[2]], true, () => 24)
    expect(added.clips[2]).toMatchObject({ included: true, manual: true })
    expect(added.totals.clips).toBe(3)
  })

  it("lays clicks not yet made part of the pack over any pack that arrives", () => {
    const p = preview([clip(0), clip(100), clip(200, { reason: "missing", included: false })])
    expect(withPicks(p, new Map(), () => 24)).toBe(p)
    // Nothing to change (already so, or a missing file): the same pack.
    expect(withPicks(p, new Map([[keyOf(p.clips[0]), true], [keyOf(p.clips[2]), true]]), () => 24)).toBe(p)
    const out = withPicks(p, new Map([[keyOf(p.clips[1]), false]]), () => 24)
    expect(out.clips[1]).toMatchObject({ included: false, manual: false, reason: "removed" })
    expect(out.totals.clips).toBe(1)
  })

  it("takes the choices a pick left onto the pack shown, leaving the other cards alone", () => {
    const p = preview([clip(0), clip(100), clip(200, { byRules: false })])
    const answer = (shape: string): PackChoices => ({
      choices: [
        { included: false, byRules: true, reason: "removed", manual: false },
        { included: true, byRules: true, reason: null, manual: null },
        { included: true, byRules: false, reason: null, manual: true },
      ],
      totals: { clips: 2, frames: 96, seconds: 4 },
      stamp: "after",
      shape,
    })
    expect(fits(p, answer("a"))).toBe(true)
    const out = withChoices(p, answer("a"))
    expect(out.clips[0]).toMatchObject({ included: false, manual: false, reason: "removed", start: 0 })
    expect(out.clips[1]).toBe(p.clips[1])
    expect(out.clips[2]).toMatchObject({ included: true, manual: true, byRules: false })
    expect([out.totals.clips, out.stamp, out.shape]).toEqual([2, "after", "a"])
    // Other shots (an analysis meanwhile), or other clips altogether: not this pack's.
    expect(fits(p, answer("b"))).toBe(false)
    expect(fits(preview([clip(0)]), answer("a"))).toBe(false)
  })

  it("splits the grid into what the pack takes and what it leaves", () => {
    const list = [clip(0), clip(100, { byRules: false }), clip(200)]
    expect(shown(list, "all")).toBe(list)
    expect(shown(list, "in").map((c) => c.start)).toEqual([0, 200])
    expect(shown(list, "out").map((c) => c.start)).toEqual([100])
  })
})

describe("filters", () => {
  it("names the length choice, a range set elsewhere too", () => {
    expect(lengthOf(DEFAULT_SPEC)).toMatchObject({ id: "any", label: "Any length" })
    expect(lengthOf({ ...DEFAULT_SPEC, minSec: 1, maxSec: 6 }).id).toBe("1-6")
    expect(lengthOf({ ...DEFAULT_SPEC, minSec: 1.5, maxSec: 8 })).toEqual({ id: null, label: "1.5 to 8 s" })
    expect(lengthOf({ ...DEFAULT_SPEC, minSec: 3 }).label).toBe("3 s or longer")
  })

  it("knows the defaults", () => {
    expect(isDefault(DEFAULT_SPEC)).toBe(true)
    expect(isDefault({ ...DEFAULT_SPEC, skipTiny: false })).toBe(false)
    expect(isDefault({ ...DEFAULT_SPEC, sizes: ["wide"] })).toBe(false)
  })
})
