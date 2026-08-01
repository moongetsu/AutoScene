import { useEffect, useState } from "react"
import { Copy, Minus, Square, X } from "lucide-react"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { isTauri } from "@/lib/desktop"
import { useProjects } from "@/state/project"
import { cn } from "@/lib/utils"

/**
 * Frameless custom chrome for Windows / Linux. The window is freely resizable —
 * bounds live in tauri.windows.conf.json, never here. Because the frame is
 * undecorated we supply the caption buttons ourselves; the OS still provides
 * the invisible resize border and Aero Snap on the WS_THICKFRAME window.
 */
export function TitleBar({ className }: { className?: string }) {
  const { active } = useProjects()
  const tauri = isTauri()
  const [maximized, setMaximized] = useState(false)

  // Keep the caption glyph in sync — the window also maximizes from a
  // drag-region double-click and from Aero Snap, neither of which route
  // through our button.
  useEffect(() => {
    if (!tauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false

    void (async () => {
      const { getCurrentWindow } = await import("@tauri-apps/api/window")
      const win = getCurrentWindow()

      const sync = async () => {
        try {
          const next = await win.isMaximized()
          if (!cancelled) setMaximized(next)
        } catch {
          /* ignore */
        }
      }

      await sync()
      const off = await win.onResized(() => void sync())
      if (cancelled) {
        off()
        return
      }
      unlisten = off
    })()

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [tauri])

  const minimize = async () => {
    if (!tauri) return
    const { getCurrentWindow } = await import("@tauri-apps/api/window")
    await getCurrentWindow().minimize()
  }

  const toggleMaximize = async () => {
    if (!tauri) return
    const { getCurrentWindow } = await import("@tauri-apps/api/window")
    await getCurrentWindow().toggleMaximize()
  }

  const close = async () => {
    if (!tauri) return
    const { getCurrentWindow } = await import("@tauri-apps/api/window")
    await getCurrentWindow().close()
  }

  return (
    <div
      className={cn(
        "flex h-10 shrink-0 select-none items-center border-b border-border/50 bg-[oklch(0.11_0.015_280)]",
        className,
      )}
      data-tauri-drag-region={tauri ? true : undefined}
    >
      <div
        className="flex min-w-0 flex-1 items-center gap-2.5 px-3"
        data-tauri-drag-region={tauri ? true : undefined}
      >
        <AutoSceneMark className="pointer-events-none size-5 ring-1 ring-primary/25" />
        <div
          className="min-w-0 leading-tight"
          data-tauri-drag-region={tauri ? true : undefined}
        >
          <p className="font-heading truncate text-[13px] tracking-tight text-foreground">
            AutoScene
          </p>
          {active ? (
            <p className="truncate text-[10px] text-muted-foreground">{active.name}</p>
          ) : (
            <p className="truncate text-[10px] text-muted-foreground">Scene pack studio</p>
          )}
        </div>
      </div>

      <div className="flex h-full shrink-0 items-stretch">
        <button
          type="button"
          aria-label="Minimize"
          onClick={() => void minimize()}
          className="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground"
        >
          <Minus className="size-3.5" strokeWidth={2} />
        </button>
        <button
          type="button"
          aria-label={maximized ? "Restore" : "Maximize"}
          onClick={() => void toggleMaximize()}
          className="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-white/5 hover:text-foreground"
        >
          {maximized ? (
            <Copy className="size-3" strokeWidth={2} />
          ) : (
            <Square className="size-3" strokeWidth={2} />
          )}
        </button>
        <button
          type="button"
          aria-label="Close"
          onClick={() => void close()}
          className="flex w-11 items-center justify-center text-muted-foreground transition-colors hover:bg-destructive/90 hover:text-destructive-foreground"
        >
          <X className="size-3.5" strokeWidth={2} />
        </button>
      </div>
    </div>
  )
}
