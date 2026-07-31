import { useEffect, useRef } from "react"
import { useNavigate } from "react-router-dom"

import { useSidebar } from "@/components/ui/sidebar"
import { isMacDesktop } from "@/lib/desktop"
import { useMenuActions, type MenuActionId } from "@/state/menu-actions"

/**
 * Bridges native macOS menu events to router navigation and page actions.
 *
 * Renders nothing. Must live inside both BrowserRouter (useNavigate) and
 * SidebarProvider (useSidebar). Rust emits a single "menu:action" event whose
 * payload is the item id, so this subscribes once instead of per-item.
 */
export function MacMenuBridge() {
  const navigate = useNavigate()
  const { toggleSidebar } = useSidebar()
  const { run, defer } = useMenuActions()

  // Refs keep the listener stable — it subscribes once and never resubscribes
  // on navigation, which would drop events mid-flight.
  const navRef = useRef(navigate)
  navRef.current = navigate
  const toggleRef = useRef(toggleSidebar)
  toggleRef.current = toggleSidebar
  const runRef = useRef(run)
  runRef.current = run
  const deferRef = useRef(defer)
  deferRef.current = defer

  useEffect(() => {
    if (!isMacDesktop()) return

    /** Run a page action, or navigate to its page and run it once mounted. */
    const runOrNavigate = (id: MenuActionId, route: string) => {
      if (runRef.current(id)) return
      deferRef.current(id)
      navRef.current(route)
    }

    const handle = (id: string) => {
      if (id.startsWith("view:/")) {
        navRef.current(id.slice("view:".length))
        return
      }
      switch (id) {
        case "app:settings":
          navRef.current("/settings")
          break
        case "file:new-project":
          navRef.current("/import")
          break
        case "file:open-video":
          runOrNavigate("file:open-video", "/import")
          break
        case "file:export":
          runOrNavigate("file:export", "/export")
          break
        case "edit:copy-timecodes":
          runOrNavigate("edit:copy-timecodes", "/timeline")
          break
        case "file:cancel-export":
          runRef.current("file:cancel-export")
          break
        case "file:reveal":
          runRef.current("file:reveal")
          break
        case "view:toggle-sidebar":
          toggleRef.current()
          break
        default:
          break
      }
    }

    let unlisten: (() => void) | undefined
    let cancelled = false

    void (async () => {
      const { listen } = await import("@tauri-apps/api/event")
      const un = await listen<string>("menu:action", (e) => handle(e.payload))
      if (cancelled) un()
      else unlisten = un
    })()

    return () => {
      cancelled = true
      unlisten?.()
    }
  }, [])

  return null
}
