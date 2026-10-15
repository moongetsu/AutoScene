import type * as React from "react"
import { useLayoutEffect, useRef, useState } from "react"
import { useRouterState } from "@tanstack/react-router"
import { cn } from "@/lib/utils"
import { PageScrolled } from "./page-scroll"

/**
 * A view: it scrolls as a whole, its top bar (`TopBar`) stays on top and
 * floats over a hero, and it arrives with a short rise. Another place (a
 * route reusing this view with new params) starts at the top again.
 */
export function Page({
  topbar,
  className,
  children,
}: {
  topbar?: React.ReactNode
  className?: string
  children: React.ReactNode
}) {
  const ref = useRef<HTMLDivElement>(null)
  const path = useRouterState({ select: (s) => s.location.pathname })
  // The place that was scrolled: a new place starts unscrolled.
  const [scrolledAt, setScrolledAt] = useState<string | null>(null)
  useLayoutEffect(() => {
    ref.current?.scrollTo({ top: 0 })
  }, [path])
  return (
    <PageScrolled.Provider value={scrolledAt === path}>
      <div
        ref={ref}
        onScroll={(e) => setScrolledAt(e.currentTarget.scrollTop > 12 ? path : null)}
        className={cn("relative h-full overflow-x-hidden overflow-y-auto", className)}
      >
        {topbar}
        <div key={path} className="animate-rise-in">
          {children}
        </div>
      </div>
    </PageScrolled.Provider>
  )
}
