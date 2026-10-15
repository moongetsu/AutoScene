import type * as React from "react"
import { useEffect, useRef } from "react"
import { useMatches } from "@tanstack/react-router"
import { getCurrentWindow } from "@tauri-apps/api/window"
import { isTauri } from "@/lib/platform"

/** Route-provided names: each route may set `staticData.crumb`. */
function useCrumbs(): { id: string; label: React.ReactNode }[] {
  const matches = useMatches()
  return matches.flatMap((m) => {
    const { crumb } = m.staticData
    if (crumb == null) return []
    const label = typeof crumb === "function" ? crumb(m.params as Record<string, string>) : crumb
    return [{ id: m.id, label }]
  })
}

/**
 * The native window title (taskbar, Alt+Tab, Mission Control) follows the
 * view, most specific first: "Cast · Walter — Dark — AutoScene". Crumbs are
 * components that follow renames live, so they render hidden and the title
 * is read from their text.
 */
export function WindowTitle() {
  const crumbs = useCrumbs()
  const ref = useRef<HTMLSpanElement>(null)
  useEffect(() => {
    const el = ref.current
    if (!el) return
    let last = ""
    const apply = () => {
      const parts = Array.from(el.children, (c) => c.textContent?.trim() ?? "").filter(Boolean)
      const title = [...parts.reverse(), "AutoScene"].join(" — ")
      if (title === last) return
      last = title
      document.title = title
      if (isTauri)
        void getCurrentWindow()
          .setTitle(title)
          .catch(() => {})
    }
    apply()
    const watch = new MutationObserver(apply)
    watch.observe(el, { subtree: true, childList: true, characterData: true })
    return () => watch.disconnect()
  }, [])
  return (
    <span ref={ref} hidden>
      {crumbs.map((c) => (
        <span key={c.id}>{c.label}</span>
      ))}
    </span>
  )
}
