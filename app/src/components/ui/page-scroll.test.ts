import { cleanup, renderHook } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"
import { REMEMBERED_PAGES, useScrollMemory } from "./page-scroll"

afterEach(cleanup)

/** A page's scrolling element, scrolled by hand (jsdom doesn't lay out). */
class Scroller extends EventTarget {
  scrollTop = 0
  scrollTo({ top }: { top: number }) {
    this.scrollTop = top
  }
  /** The person scrolls to `top`. */
  scroll(top: number) {
    this.scrollTop = top
    this.dispatchEvent(new Event("scroll"))
  }
}

describe("scroll memory", () => {
  it("remembers the latest pages' places, and only those", () => {
    const el = new Scroller()
    const page = renderHook(({ entry }) => useScrollMemory(el as unknown as HTMLElement, entry), {
      initialProps: { entry: "e0" },
    })
    const visit = (entry: string) => {
      page.rerender({ entry })
      return el.scrollTop
    }
    const extra = 50
    for (let i = 0; i < REMEMBERED_PAGES + extra; i++) {
      visit(`e${i}`)
      el.scroll(i + 1)
    }
    // The oldest are forgotten: they start at the top again.
    expect(visit("e0")).toBe(0)
    expect(visit(`e${extra - 1}`)).toBe(0)
    expect(visit(`e${REMEMBERED_PAGES + extra - 1}`)).toBe(REMEMBERED_PAGES + extra)
    // A page visited again counts as recent: the next new page pushes out
    // the one after it instead.
    expect(visit(`e${extra}`)).toBe(extra + 1)
    visit("new")
    el.scroll(7)
    expect(visit(`e${extra}`)).toBe(extra + 1)
    expect(visit(`e${extra + 1}`)).toBe(0)
  })
})
