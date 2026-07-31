import { useEffect } from "react"
import { Minus, X } from "lucide-react"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { isTauri } from "@/lib/desktop"
import { useProjects } from "@/state/project"
import { cn } from "@/lib/utils"

const LOCK_W = 1280
const LOCK_H = 800

async function lockWindowSize() {
  const { getCurrentWindow } = await import("@tauri-apps/api/window")
  const { LogicalSize } = await import("@tauri-apps/api/dpi")
  const win = getCurrentWindow()
  const size = new LogicalSize(LOCK_W, LOCK_H)
  try {
    await win.setMaximizable(false)
  } catch {
    /* older runtime */
  }
  try {
    await win.setResizable(false)
  } catch {
    /* ignore */
  }
  try {
    await win.setFullscreen(false)
  } catch {
    /* ignore */
  }
  try {
    if (await win.isMaximized()) await win.unmaximize()
  } catch {
    /* ignore */
  }
  try {
    await win.setSize(size)
  } catch {
    /* ignore */
  }
}

/**
 * Frameless custom chrome. Fixed 1280×800 — no maximize / fullscreen.
 */
export function TitleBar({ className }: { className?: string }) {
  const { active } = useProjects()
  const tauri = isTauri()

  useEffect(() => {
    if (!tauri) return
    let unlisten: (() => void) | undefined
    let cancelled = false

    void (async () => {
      await lockWindowSize()
      if (cancelled) return
      const { getCurrentWindow } = await import("@tauri-apps/api/window")
      const win = getCurrentWindow()
      unlisten = await win.onResized(() => {
        void lockWindowSize()
      })
    })()

    const onKey = (e: KeyboardEvent) => {
      if (e.key === "F11") {
        e.preventDefault()
        e.stopPropagation()
        void lockWindowSize()
      }
    }
    window.addEventListener("keydown", onKey, true)

    return () => {
      cancelled = true
      unlisten?.()
      window.removeEventListener("keydown", onKey, true)
    }
  }, [tauri])

  const minimize = async () => {
    if (!tauri) return
    const { getCurrentWindow } = await import("@tauri-apps/api/window")
    await getCurrentWindow().minimize()
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
