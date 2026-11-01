import type * as React from "react"
import { Link, useNavigate } from "@tanstack/react-router"
import { CaretLeft, SidebarSimple } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { contextKit } from "@/components/ui/menu-actions"
import { WindowItems } from "@/components/shell/WindowControls"
import { isTauri, isWindows } from "@/lib/platform"
import { usePageScrolled } from "@/components/ui/page-scroll"
import { Tooltip } from "@/components/ui/tooltip"
import { useShortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useUi } from "@/state/ui"

/** What in the bar is a control of its own (not the window's caption). */
const CONTROLS = "a[href], button, input, textarea, [role=button], [role=tab], [role=link], [role=switch], [tabindex]"

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
  const keys = useShortcutKeys()
  const scrolled = usePageScrolled()
  const glass = !overlay || scrolled
  const railHidden = useUi((s) => s.sidebarCollapsed)
  const showRail = useUi((s) => s.toggleSidebar)
  const navigate = useNavigate()
  return (
    // The bar is the window's caption: on Windows a right-click on its empty
    // parts (and the title) is the window's own — minimize, maximize, close —
    // as a title bar's is. Its buttons and tabs keep theirs.
    <ContextArea
      tail={false}
      disabled={!(isWindows && isTauri)}
      only={(target) => !target.closest(CONTROLS)}
      menu={<WindowItems kit={contextKit} />}
    >
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
            <Tooltip label="Show the rail" shortcut={keys("global.sidebar")}>
              <Button variant="glass" size="icon-sm" aria-label="Show the rail" onClick={showRail}>
                <SidebarSimple />
              </Button>
            </Tooltip>
          )}
          {back && (
            <Tooltip label={`Back to ${back.label}`}>
              <ContextArea
                menu={
                  <ContextMenuItem onSelect={() => void navigate({ to: back.to })}>
                    <CaretLeft /> Back to {back.label}
                  </ContextMenuItem>
                }
              >
                <Button variant="glass" size="icon-sm" asChild>
                  <Link to={back.to} aria-label={`Back to ${back.label}`}>
                    <CaretLeft weight="bold" />
                  </Link>
                </Button>
              </ContextArea>
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
    </ContextArea>
  )
}
