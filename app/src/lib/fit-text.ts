import { type RefObject, useLayoutEffect, useRef, useState } from "react"
import { flushSync } from "react-dom"

/**
 * The font size (px) a one-line title `natural` px wide at `base` px takes to
 * fit `room` px, never below `min`; `null` when it fits as it is.
 */
export function fittedFontSize(base: number, natural: number, room: number, min: number): number | null {
  if (natural <= room || room <= 0) return null
  return Math.max(min, Math.floor((base * room) / natural))
}

/**
 * The font size (px) that keeps a one-line title within its `row`, for its
 * `style`; `undefined`: the size its classes give it. When its text is wider
 * than the room from where it starts to the row's end (less what follows it
 * in its own box, like an icon), the size gets smaller, down to `min`; below
 * that it is cut short. The room is the title's when it has the row to
 * itself: what shares the line with it moves to the next one first (a
 * wrapping row).
 *
 * The row is an element (held in state, from a callback ref), not a ref: a
 * row around the title gets its ref only after the title's effects ran.
 */
export function useFittedTitle(
  title: RefObject<HTMLElement | null>,
  row: HTMLElement | null,
  text: string,
  min: number,
): number | undefined {
  const [size, setSize] = useState<number>()
  // The size its classes give it, seen while it had no size of its own.
  const base = useRef(0)
  useLayoutEffect(() => {
    const el = title.current
    if (!el || !row) return
    const fit = () => {
      const current = parseFloat(getComputedStyle(el).fontSize)
      if (!(current > 0)) return
      if (el.style.fontSize === "") base.current = current
      if (!base.current) return
      const own = el.getBoundingClientRect()
      const after = Math.max(0, (el.parentElement?.getBoundingClientRect().right ?? own.right) - own.right)
      const room = row.getBoundingClientRect().right - own.left - after
      // Text widths follow the font size.
      const natural = (el.scrollWidth * base.current) / current
      setSize(fittedFontSize(base.current, natural, room, min) ?? undefined)
    }
    fit()
    // Its row's height follows the title (the line it wraps onto): only a
    // new width asks for another fit, drawn in the same frame.
    let width = row.clientWidth
    const resized = new ResizeObserver(() => {
      if (row.clientWidth === width) return
      width = row.clientWidth
      flushSync(fit)
    })
    resized.observe(row)
    // The display face may arrive after the first fit.
    let live = true
    void document.fonts?.ready.then(() => live && flushSync(fit))
    return () => {
      live = false
      resized.disconnect()
    }
  }, [title, row, text, min])
  return size
}
