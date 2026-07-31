import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  type ReactNode,
} from "react"

import { isMacDesktop } from "@/lib/desktop"

/**
 * Registry letting the native macOS menu reach page-local handlers.
 *
 * Menu events arrive globally, but actions like "Export Merged Video…" live
 * inside ExportPage and close over its state. Pages register while mounted;
 * the bridge looks handlers up by id. All state lives in refs, so registering
 * never re-renders consumers.
 */

export type MenuActionId =
  | "file:open-video"
  | "file:export"
  | "file:cancel-export"
  | "file:reveal"
  | "edit:copy-timecodes"

type Handler = () => void | Promise<void>

type MenuActionsCtx = {
  register: (id: MenuActionId, fn: Handler) => () => void
  /** Runs the handler for `id`. Returns false when no page has registered one. */
  run: (id: MenuActionId) => boolean
  /** Queue `id` to fire as soon as a page registers it. Expires after 4s. */
  defer: (id: MenuActionId) => void
}

const Ctx = createContext<MenuActionsCtx | null>(null)

/** How long a deferred action waits for its page to mount after navigation. */
const DEFER_TTL_MS = 4000

export function MenuActionsProvider({ children }: { children: ReactNode }) {
  const handlers = useRef(new Map<MenuActionId, Handler>())
  const pending = useRef<{ id: MenuActionId; at: number } | null>(null)

  const register = useCallback((id: MenuActionId, fn: Handler) => {
    handlers.current.set(id, fn)

    // A menu item fired before this page existed (e.g. Cmd+O from Dashboard,
    // which navigates to /import first). Run it now that the page is here.
    const queued = pending.current
    if (queued?.id === id && Date.now() - queued.at < DEFER_TTL_MS) {
      pending.current = null
      queueMicrotask(() => {
        void fn()
      })
    }

    return () => {
      // Only clear if this exact handler is still the registered one — guards
      // against a remount ordering race clobbering the newer registration.
      if (handlers.current.get(id) === fn) handlers.current.delete(id)
    }
  }, [])

  const run = useCallback((id: MenuActionId) => {
    const fn = handlers.current.get(id)
    if (!fn) return false
    void fn()
    return true
  }, [])

  const defer = useCallback((id: MenuActionId) => {
    pending.current = { id, at: Date.now() }
  }, [])

  const value = useMemo(
    () => ({ register, run, defer }),
    [register, run, defer],
  )

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>
}

export function useMenuActions(): MenuActionsCtx {
  const ctx = useContext(Ctx)
  if (!ctx) throw new Error("useMenuActions must be used within MenuActionsProvider")
  return ctx
}

/**
 * Expose a page action to the native menu. Pass null when the action is
 * currently unavailable (nothing selected, export already running, …).
 *
 * No-op off macOS desktop, so Windows/Linux and browser dev are untouched.
 */
export function useMenuAction(id: MenuActionId, handler: Handler | null) {
  const { register } = useMenuActions()
  const ref = useRef(handler)
  ref.current = handler

  // Registration tracks availability, not identity: a null handler stays
  // unregistered so run() reports false and the bridge can navigate and defer
  // until the page is actually ready to serve the action.
  const available = handler !== null

  useEffect(() => {
    if (!isMacDesktop() || !available) return
    // The ref keeps the handler fresh without re-registering every render.
    return register(id, () => {
      void ref.current?.()
    })
  }, [id, register, available])
}
