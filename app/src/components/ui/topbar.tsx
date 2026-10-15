import type * as React from "react"
import { Link } from "@tanstack/react-router"
import { CaretLeft, SidebarSimple } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { usePageScrolled } from "@/components/ui/page-scroll"
import { Tooltip } from "@/components/ui/tooltip"
import { shortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useUi } from "@/state/ui"

/**
 * A view's top bar: back and title on the left, the view's sections in the
 * middle, its actions on the right. It is the window's drag region
 * (double-click maximises), so its empty parts carry `data-tauri-drag-region`;
 * the Windows caption buttons sit over its right end.
 *
 * `overlay`: the bar floats over a hero, clear until the page scrolls, and
 * shows its title only then (the hero shows it big).
 */
export function TopBar({
  back,
  title,
  nav,
  overlay,
  className,
  children,
}: {
  back?: { label: string; to: string }
  title?: React.ReactNode
  nav?: React.ReactNode
  overlay?: boolean
  className?: string
  children?: React.ReactNode
}) {
  const scrolled = usePageScrolled()
  const glass = !overlay || scrolled
  const railHidden = useUi((s) => s.sidebarCollapsed)
  const showRail = useUi((s) => s.toggleSidebar)
  return (
    <header
      data-tauri-drag-region
      className={cn(
        "sticky top-0 z-20 grid h-[var(--topbar-h)] grid-cols-[minmax(0,1fr)_auto_minmax(0,1fr)] items-center gap-4 pr-[calc(var(--caption-inset)+16px)]",
        // Without the rail the macOS traffic lights sit over the bar's left end.
        railHidden ? "pl-[calc(var(--traffic-x)+16px)]" : "pl-6",
        "border-b transition-[background-color,border-color,backdrop-filter] duration-[var(--dur-slow)]",
        overlay && "mb-[calc(var(--topbar-h)*-1)]",
        glass ? "border-hairline bg-canvas/70 backdrop-blur-2xl backdrop-saturate-150" : "border-transparent",
        className,
      )}
    >
      <div data-tauri-drag-region className="flex h-full min-w-0 items-center gap-3">
        {railHidden && (
          <Tooltip label="Show the rail" shortcut={shortcutKeys("sidebar")}>
            <Button variant="glass" size="icon-sm" aria-label="Show the rail" onClick={showRail}>
              <SidebarSimple />
            </Button>
          </Tooltip>
        )}
        {back && (
          <Tooltip label={`Back to ${back.label}`}>
            <Button variant="glass" size="icon-sm" asChild>
              <Link to={back.to} aria-label={`Back to ${back.label}`}>
                <CaretLeft weight="bold" />
              </Link>
            </Button>
          </Tooltip>
        )}
        {title != null && (
          <h1
            data-tauri-drag-region
            className={cn(
              "min-w-0 truncate text-lg font-semibold tracking-tight text-fg-1 transition-opacity duration-[var(--dur-med)]",
              overlay && !scrolled && "opacity-0",
            )}
          >
            {title}
          </h1>
        )}
      </div>
      <div className="flex justify-center">{nav}</div>
      <div data-tauri-drag-region className="flex h-full min-w-0 items-center justify-end gap-2">
        {children}
      </div>
    </header>
  )
}
