import type * as React from "react"
import { useLayoutEffect, useRef, useState } from "react"
import { useVirtualizer } from "@tanstack/react-virtual"
import { gridColumns } from "@/lib/grid"
import { cn } from "@/lib/utils"
import { usePageScrollElement } from "./page-scroll"

/**
 * A grid of pictures that renders only the rows on screen — a character's
 * shots or everyone else's faces run into the thousands. Columns fill the
 * width like CSS `auto-fill` (each at least `minItemWidth`); every row is as
 * tall as `itemHeight(itemWidth)`. It scrolls with the page it is on.
 */
export function VirtualGrid<T>({
  items,
  minItemWidth,
  itemHeight,
  gap = 16,
  getKey,
  renderItem,
  overscan = 4,
  className,
}: {
  items: T[]
  minItemWidth: number
  /** An item's height for a width (pictures keep their shape). */
  itemHeight: (width: number) => number
  gap?: number
  getKey: (item: T) => React.Key
  renderItem: (item: T, index: number) => React.ReactNode
  overscan?: number
  className?: string
}) {
  const scroller = usePageScrollElement()
  const ref = useRef<HTMLDivElement>(null)
  const [width, setWidth] = useState(0)
  // Where the grid starts inside the scrolling page: content above it (a
  // hero, a header that wraps) moves it.
  const [top, setTop] = useState(0)

  useLayoutEffect(() => {
    const el = ref.current
    if (!el || !scroller) return
    const measure = () => {
      setWidth(el.clientWidth)
      setTop(el.getBoundingClientRect().top - scroller.getBoundingClientRect().top + scroller.scrollTop)
    }
    measure()
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    // Content above the grid changing height moves it without resizing it.
    if (scroller.firstElementChild) ro.observe(scroller.firstElementChild)
    return () => ro.disconnect()
  }, [scroller])

  const { columns, itemWidth } = gridColumns(width, minItemWidth, gap)
  const rowHeight = itemHeight(itemWidth) + gap
  const rows = Math.ceil(items.length / columns)
  // The virtualizer is read during this render only, never handed to a
  // memoised child (and the app doesn't run the React Compiler).
  // oxlint-disable-next-line react/incompatible-library
  const virtualizer = useVirtualizer({
    count: width > 0 ? rows : 0,
    getScrollElement: () => scroller,
    estimateSize: () => rowHeight,
    overscan,
    scrollMargin: top,
  })
  // Rows change height with the window's width; the virtualizer only asks once.
  useLayoutEffect(() => {
    virtualizer.measure()
  }, [virtualizer, rowHeight])

  return (
    <div ref={ref} className={cn("relative w-full", className)} style={{ height: Math.max(0, rows * rowHeight - gap) }}>
      {virtualizer.getVirtualItems().map((row) => (
        <div
          key={row.key}
          className="absolute inset-x-0 top-0 grid"
          style={{
            transform: `translateY(${row.start - top}px)`,
            gridTemplateColumns: `repeat(${columns}, minmax(0, 1fr))`,
            gap,
          }}
        >
          {items.slice(row.index * columns, row.index * columns + columns).map((item, i) => (
            <div key={getKey(item)} className="min-w-0">
              {renderItem(item, row.index * columns + i)}
            </div>
          ))}
        </div>
      ))}
    </div>
  )
}
