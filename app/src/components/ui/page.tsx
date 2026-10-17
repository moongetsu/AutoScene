import type * as React from "react"
import { useState } from "react"
import { useRouterState } from "@tanstack/react-router"
import { cn } from "@/lib/utils"
import { ContextArea, type MenuContent } from "./context-area"
import { PageScrollElement, PageScrolled, useScrollMemory } from "./page-scroll"

/**
 * A view: it scrolls as a whole, its top bar (`TopBar`) stays on top and
 * floats over a hero, and it arrives with a short rise. Back and Forward
 * return to where it was scrolled; a new visit starts at the top.
 */
export function Page({
  topbar,
  className,
  menu,
  children,
}: {
  topbar?: React.ReactNode
  className?: string
  /** What a right-click on the view's empty space offers, above the app's own entries. */
  menu?: MenuContent
  children: React.ReactNode
}) {
  // The element, as state: lists that render only what is on screen follow its scrolling.
  const [scroller, setScroller] = useState<HTMLDivElement | null>(null)
  const path = useRouterState({ select: (s) => s.location.pathname })
  // The history entry: each visit of a place has its own key (the router
  // gives every entry one, the first page's too).
  const entry = useRouterState({ select: (s) => s.location.state.__TSR_key ?? s.location.href })
  // The place that was scrolled: a new place starts unscrolled.
  const [scrolledAt, setScrolledAt] = useState<string | null>(null)
  useScrollMemory(scroller, entry)
  return (
    <PageScrolled.Provider value={scrolledAt === path}>
      <ContextArea kind="page" menu={menu ?? null}>
        <div
          ref={setScroller}
          onScroll={(e) => setScrolledAt(e.currentTarget.scrollTop > 12 ? path : null)}
          className={cn("relative h-full overflow-x-hidden overflow-y-auto", className)}
        >
          {topbar}
          <PageScrollElement.Provider value={scroller}>
            <div key={path} className="animate-rise-in">
              {children}
            </div>
          </PageScrollElement.Provider>
        </div>
      </ContextArea>
    </PageScrolled.Provider>
  )
}
