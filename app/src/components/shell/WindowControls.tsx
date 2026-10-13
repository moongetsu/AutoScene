import { useEffect, useState } from "react"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { cn } from "@/lib/utils"

/** Windows caption buttons for the frameless window (Segoe Fluent-style glyphs). */
export function WindowControls() {
  const [maximized, setMaximized] = useState(false)

  useEffect(() => {
    const win = getCurrentWindow()
    void win.isMaximized().then(setMaximized)
    const un = win.onResized(() => void win.isMaximized().then(setMaximized))
    return () => void un.then((f) => f())
  }, [])

  const win = () => getCurrentWindow()
  const base = "flex h-full w-[46px] items-center justify-center text-fg-2 transition-colors duration-[var(--dur-fast)]"

  return (
    <div className="flex h-full">
      <button aria-label="Minimize" className={cn(base, "hover:bg-hover hover:text-fg-1")} onClick={() => void win().minimize()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M0 5h10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
      <button
        aria-label={maximized ? "Restore" : "Maximize"}
        className={cn(base, "hover:bg-hover hover:text-fg-1")}
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
      <button aria-label="Close" className={cn(base, "hover:bg-[#c42b1c] hover:text-white")} onClick={() => void win().close()}>
        <svg width="10" height="10" viewBox="0 0 10 10" aria-hidden>
          <path d="M0 0l10 10M10 0L0 10" stroke="currentColor" strokeWidth="1" />
        </svg>
      </button>
    </div>
  )
}
