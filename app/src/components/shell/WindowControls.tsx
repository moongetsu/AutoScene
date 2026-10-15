import { useEffect, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { cn } from "@/lib/utils"

/**
 * Windows caption buttons for the frameless window (Segoe Fluent-style
 * glyphs), over the top-right corner of every view (`--caption-inset`).
 */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    const win = getCurrentWindow()
    void win.isMaximized().then(setMaximized)
    const un = win.onResized(() => void win.isMaximized().then(setMaximized))
    return () => void un.then((f) => f())
  }, [])

  const win = () => getCurrentWindow()
  const base =
    "flex h-full w-[46px] items-center justify-center text-fg-1/80 transition-colors duration-[var(--dur-fast)] [filter:drop-shadow(0_1px_2px_rgb(0_0_0/0.6))]"

  return (
    <div className="fixed top-0 right-0 z-30 flex h-[var(--topbar-h)]">
      <button aria-label="Minimize" className={cn(base, "hover:bg-white/10 hover:text-white")} onClick={() => void win().minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
      <button
        aria-label={maximized ? "Restore" : "Maximize"}
        className={cn(base, "hover:bg-white/10 hover:text-white")}
        onClick={() => void win().toggleMaximize()}
      >
        {maximized ? (
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
            <path d="M2.5 2.5V.5h7v7h-2" stroke="currentColor" />
            <rect x=".5" y="2.5" width="7" height="7" stroke="currentColor" />
          </svg>
        ) : (
          <svg width="10" height="10" viewBox="0 0 10 10" fill="none" aria-hidden>
            <rect x=".5" y=".5" width="9" height="9" stroke="currentColor" />
          </svg>
        )}
      </button>
      {/* Windows' own close-hover red: a platform convention, not a brand accent. */}
      <button aria-label="Close" className={cn(base, "hover:bg-win-close hover:text-on-win-close")} onClick={() => void win().close()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
    </div>
  )
}
