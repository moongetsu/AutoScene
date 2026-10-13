import { Link, useMatches } from "@tanstack/react-router"
import { Activity, ChevronRight, Search } from "lucide-react"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { Kbd } from "@/components/ui/kbd"
import { Tooltip } from "@/components/ui/tooltip"
import { useAppActions } from "@/lib/actions"
import { isMac, isTauri, isWindows } from "@/lib/platform"
import { shortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useUi } from "@/state/ui"
import { WindowControls } from "./WindowControls"

export type Crumb = { label: string; to?: string }

/** Route-provided breadcrumbs: each route may set `staticData.crumb`. */
function useCrumbs(): Crumb[] {
  const matches = useMatches()
  return matches.flatMap((m) => {
    const c = (m.staticData as { crumb?: string | ((p: Record<string, string>) => string) }).crumb
    if (!c) return []
    const label = typeof c === "function" ? c(m.params as Record<string, string>) : c
    return [{ label, to: m.pathname }]
  })
}

export function TitleBar() {
  const actions = useAppActions()
  const crumbs = useCrumbs()
  const activityOpen = useUi((s) => s.activityOpen)
  // macOS overlay title bar: leave room for the traffic lights.
  const macInset = isMac && isTauri

  return (
    <header
      data-tauri-drag-region
      className="relative z-30 flex h-[var(--titlebar-h)] shrink-0 items-center border-b border-hairline bg-canvas"
    >
      <div data-tauri-drag-region className={cn("flex min-w-0 flex-1 items-center gap-2.5", macInset ? "pl-[84px]" : "pl-3")}>
        <Link to="/" className="flex shrink-0 items-center gap-2 rounded-sm" aria-label="AutoScene library">
          <AutoSceneMark className="size-[22px]" />
          <span className="text-sm font-semibold tracking-tight text-fg-1">AutoScene</span>
        </Link>
        {crumbs.length > 0 && (
          <nav aria-label="Breadcrumb" data-tauri-drag-region className="flex min-w-0 items-center gap-1 text-sm">
            {crumbs.map((c, i) => (
              <span key={`${c.label}-${i}`} data-tauri-drag-region className="flex min-w-0 items-center gap-1">
                <ChevronRight className="size-3.5 shrink-0 text-fg-disabled" />
                {c.to && i < crumbs.length - 1 ? (
                  <Link to={c.to} className="truncate rounded-xs px-1 text-fg-3 transition-colors hover:text-fg-1">
                    {c.label}
                  </Link>
                ) : (
                  <span data-tauri-drag-region className="truncate px-1 text-fg-2">
                    {c.label}
                  </span>
                )}
              </span>
            ))}
          </nav>
        )}
      </div>

      <button
        onClick={actions.openPalette}
        className="absolute left-1/2 hidden h-7 w-[min(380px,32vw)] -translate-x-1/2 items-center gap-2 rounded-md border border-hairline bg-inset px-2.5 text-xs text-fg-3 transition-colors hover:border-soft hover:text-fg-2 md:flex"
      >
        <Search className="size-3.5" />
        <span className="flex-1 text-left">Search or jump to…</span>
        <Kbd keys={shortcutKeys("palette")} />
      </button>

      <div data-tauri-drag-region className="flex h-full flex-1 items-center justify-end gap-1">
        <Tooltip label="Activity" shortcut={shortcutKeys("activity")}>
          <button
            onClick={actions.toggleActivity}
            aria-pressed={activityOpen}
            className={cn(
              "relative flex size-7 items-center justify-center rounded-md text-fg-3 transition-colors hover:bg-hover hover:text-fg-1",
              activityOpen && "bg-active text-fg-1",
            )}
          >
            <Activity className="size-4" />
            <span className="sr-only">Activity</span>
          </button>
        </Tooltip>
        {isWindows && isTauri ? <WindowControls /> : <span className="w-2" />}
      </div>
    </header>
  )
}
