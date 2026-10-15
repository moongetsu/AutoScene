import { createContext, useContext, useLayoutEffect } from "react"

export const PageScrolled = createContext(false)

/** Whether the page under the top bar has scrolled (the bar turns to glass). */
export function usePageScrolled(): boolean {
  return useContext(PageScrolled)
}

export const PageScrollElement = createContext<HTMLElement | null>(null)

/** The element the current page scrolls in (for lists that render only what is on screen). */
export function usePageScrollElement(): HTMLElement | null {
  return useContext(PageScrollElement)
}

/** Where each visited page was scrolled to, by history entry (for Back). */
const positions = new Map<string, number>()
/** How long a page coming back keeps trying to reach its place while its lists load. */
const SETTLE_MS = 1000

/**
 * A page scrolls back to where it was when it is visited again (Back,
 * Forward); a new visit starts at the top. Lists that render what is on
 * screen grow as their data arrives, so the place is tried again for a
 * moment — until it is reached or the person scrolls.
 */
export function useScrollMemory(el: HTMLElement | null, entry: string) {
  useLayoutEffect(() => {
    if (!el) return
    const target = positions.get(entry) ?? 0
    el.scrollTo({ top: target })
    let frame = 0
    const until = performance.now() + SETTLE_MS
    const settle = () => {
      el.scrollTo({ top: target })
      if (Math.abs(el.scrollTop - target) > 1 && performance.now() < until) frame = requestAnimationFrame(settle)
    }
    if (Math.abs(el.scrollTop - target) > 1) frame = requestAnimationFrame(settle)
    const stop = () => cancelAnimationFrame(frame)
    const remember = () => positions.set(entry, el.scrollTop)
    el.addEventListener("scroll", remember, { passive: true })
    for (const e of ["wheel", "touchstart", "pointerdown", "keydown"]) el.addEventListener(e, stop, { passive: true })
    return () => {
      stop()
      el.removeEventListener("scroll", remember)
      for (const e of ["wheel", "touchstart", "pointerdown", "keydown"]) el.removeEventListener(e, stop)
    }
  }, [el, entry])
}
