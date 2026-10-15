import type * as React from "react"
import { useCallback, useEffect, useRef, useState } from "react"
import { CaretLeft, CaretRight } from "@/components/icons"
import { cn } from "@/lib/utils"

/** A titled part of a page: a heading with a count, and an action on the right. */
export function Section({
  title,
  count,
  action,
  className,
  children,
}: {
  title: React.ReactNode
  count?: React.ReactNode
  action?: React.ReactNode
  className?: string
  children: React.ReactNode
}) {
  return (
    <section className={cn("px-10", className)}>
      <div className="mb-4 flex items-end justify-between gap-4">
        <h2 className="flex items-baseline gap-2.5 text-xl font-semibold tracking-tight text-fg-1">
          {title}
          {count != null && <span className="text-md font-medium text-fg-3">{count}</span>}
        </h2>
        {action}
      </div>
      {children}
    </section>
  )
}

/**
 * A horizontal row of cards that scrolls (wheel, trackpad, or the arrows that
 * appear on hover), its edges fading into the page.
 */
export function Shelf({ className, children }: { className?: string; children: React.ReactNode }) {
  const ref = useRef<HTMLDivElement>(null)
  const [edges, setEdges] = useState({ start: false, end: false })
  const measure = useCallback(() => {
    const el = ref.current
    if (!el) return
    const start = el.scrollLeft > 4
    const end = el.scrollLeft + el.clientWidth < el.scrollWidth - 4
    setEdges((e) => (e.start === start && e.end === end ? e : { start, end }))
  }, [])
  useEffect(() => {
    measure()
    const el = ref.current
    if (!el) return
    const ro = new ResizeObserver(measure)
    ro.observe(el)
    return () => ro.disconnect()
  }, [measure])
  const page = (dir: 1 | -1) => {
    const el = ref.current
    el?.scrollBy({ left: dir * el.clientWidth * 0.85, behavior: "smooth" })
  }
  const from = edges.start ? "transparent" : "black"
  const to = edges.end ? "transparent" : "black"
  return (
    <div className="group/shelf relative -mx-10">
      <div
        ref={ref}
        onScroll={measure}
        className={cn(
          "flex snap-x scroll-px-10 gap-4 overflow-x-auto px-10 pt-1 pb-6 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden",
          className,
        )}
        style={{ maskImage: `linear-gradient(90deg, ${from}, black 48px, black calc(100% - 48px), ${to})` }}
      >
        {children}
      </div>
      {edges.start && <ShelfArrow side="left" onClick={() => page(-1)} />}
      {edges.end && <ShelfArrow side="right" onClick={() => page(1)} />}
    </div>
  )
}

function ShelfArrow({ side, onClick }: { side: "left" | "right"; onClick: () => void }) {
  return (
    <button
      onClick={onClick}
      aria-label={side === "left" ? "Scroll back" : "Scroll on"}
      className={cn(
        "glass-strong absolute top-[calc(50%-12px)] flex size-10 -translate-y-1/2 items-center justify-center rounded-full text-fg-1 opacity-0 transition-opacity duration-[var(--dur-med)] group-hover/shelf:opacity-100 focus-visible:opacity-100 [&_svg]:size-4",
        side === "left" ? "left-4" : "right-4",
      )}
    >
      {side === "left" ? <CaretLeft weight="bold" /> : <CaretRight weight="bold" />}
    </button>
  )
}
