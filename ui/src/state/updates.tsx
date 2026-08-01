import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactNode,
} from "react"

import { isTauri } from "@/lib/desktop"
import {
  FIRST_CHECK_DELAY_MS,
  RECHECK_INTERVAL_MS,
  autoCheckEnabled,
  checkForUpdate,
  clearDeferrals,
  installUpdate,
  lastChecked,
  markChecked,
  reconcileDeferrals,
  relaunchApp,
  remindIn,
  setAutoCheckEnabled,
  shouldPrompt,
  skipVersion,
  type UpdateProgress,
  type UpdateStatus,
} from "@/lib/updates"

/**
 * One update check shared by everything that needs it: the automatic check on
 * launch, the macOS "Check for Updates…" menu item, and the Settings button.
 *
 * Centralising it means a manual check while a download is running reuses the
 * open dialog rather than starting a second install.
 */

/** Where the flow currently is. Drives which face the dialog shows. */
export type UpdatePhase =
  | "idle"
  /** Downloading + installing. */
  | "working"
  /** Staged; the app needs a restart to run it. */
  | "staged"
  /** Simulated install finished — nothing was really written. */
  | "simulated"
  | "error"

type UpdatesCtx = {
  status: UpdateStatus | null
  phase: UpdatePhase
  progress: UpdateProgress | null
  /** Why the last check or install failed. */
  error: string | null
  checking: boolean
  dialogOpen: boolean
  /** True while the currently open dialog came from a user-initiated check. */
  manual: boolean
  lastCheckedAt: number | null
  autoCheck: boolean
  supported: boolean
  setAutoCheck: (on: boolean) => void
  /** Run a check. `manual` also opens the dialog when already up to date. */
  check: (manual: boolean) => Promise<UpdateStatus | null>
  /** Download + install now. */
  updateNow: () => Promise<void>
  /** Stop offering this version; it stays reachable from Check for Updates. */
  updateLater: () => void
  /** Hide the prompt and raise it again in 24 hours. */
  remindMeLater: () => void
  restart: () => Promise<void>
  closeDialog: () => void
}

const Ctx = createContext<UpdatesCtx | null>(null)

export function UpdatesProvider({ children }: { children: ReactNode }) {
  const supported = isTauri()

  const [status, setStatus] = useState<UpdateStatus | null>(null)
  const [phase, setPhase] = useState<UpdatePhase>("idle")
  const [progress, setProgress] = useState<UpdateProgress | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [checking, setChecking] = useState(false)
  const [dialogOpen, setDialogOpen] = useState(false)
  const [manual, setManual] = useState(false)
  const [lastCheckedAt, setLastCheckedAt] = useState<number | null>(() =>
    supported ? lastChecked() : null,
  )
  const [autoCheck, setAutoCheckState] = useState(() =>
    supported ? autoCheckEnabled() : false,
  )

  // A check already in flight — a menu click mid-check should join it, not
  // fire a second request.
  const inFlight = useRef<Promise<UpdateStatus | null> | null>(null)
  // Never interrupt a download with an automatic re-check.
  const busy = useRef(false)
  busy.current = phase === "working" || phase === "staged"

  const check = useCallback(
    async (isManual: boolean): Promise<UpdateStatus | null> => {
      if (!supported) return null
      if (busy.current && !isManual) return null
      if (inFlight.current) {
        if (isManual) setManual(true)
        return inFlight.current
      }

      setChecking(true)
      if (isManual) {
        setManual(true)
        setError(null)
        // Open straight away so a menu click has visible feedback, rather than
        // nothing at all until the request comes back.
        if (!busy.current) setPhase("idle")
        setDialogOpen(true)
      }

      const run = (async () => {
        try {
          const next = await checkForUpdate()
          setStatus(next)
          setError(null)
          markChecked()
          setLastCheckedAt(Date.now())

          // A version newer than one already waved off re-opens the prompt.
          reconcileDeferrals(next)

          if (isManual || shouldPrompt(next)) {
            setPhase((p) => (p === "working" || p === "staged" ? p : "idle"))
            setDialogOpen(true)
          }
          return next
        } catch (e) {
          const msg = e instanceof Error ? e.message : String(e)
          setError(msg)
          // A failed automatic check stays silent; only a manual one reports.
          if (isManual) {
            setStatus(null)
            setPhase("error")
            setDialogOpen(true)
          }
          return null
        } finally {
          setChecking(false)
          inFlight.current = null
        }
      })()

      inFlight.current = run
      return run
    },
    [supported],
  )

  const updateNow = useCallback(async () => {
    setPhase("working")
    setProgress({ downloaded: 0, total: null })
    setError(null)
    try {
      await installUpdate((p) => setProgress(p))
      // Simulate mode installs nothing, so it gets its own end state rather
      // than pretending a restart would change anything.
      setPhase(status?.simulated ? "simulated" : "staged")
      clearDeferrals()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setPhase("error")
    }
  }, [status?.simulated])

  const updateLater = useCallback(() => {
    if (status?.version) skipVersion(status.version)
    setDialogOpen(false)
  }, [status?.version])

  const remindMeLater = useCallback(() => {
    remindIn()
    setDialogOpen(false)
  }, [])

  const restart = useCallback(async () => {
    try {
      const relaunched = await relaunchApp()
      // Simulate mode declines to restart; close up instead of hanging on a
      // "Restarting…" state that will never resolve.
      if (!relaunched) setDialogOpen(false)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setPhase("error")
    }
  }, [])

  const closeDialog = useCallback(() => setDialogOpen(false), [])

  const setAutoCheck = useCallback((on: boolean) => {
    setAutoCheckEnabled(on)
    setAutoCheckState(on)
  }, [])

  // Automatic check on launch, then periodically for long-running sessions.
  const checkRef = useRef(check)
  checkRef.current = check
  useEffect(() => {
    if (!supported || !autoCheck) return
    const first = window.setTimeout(() => {
      void checkRef.current(false)
    }, FIRST_CHECK_DELAY_MS)
    const repeat = window.setInterval(() => {
      void checkRef.current(false)
    }, RECHECK_INTERVAL_MS)
    return () => {
      window.clearTimeout(first)
      window.clearInterval(repeat)
    }
  }, [supported, autoCheck])

  const value = useMemo<UpdatesCtx>(
    () => ({
      status,
      phase,
      progress,
      error,
      checking,
      dialogOpen,
      manual,
      lastCheckedAt,
      autoCheck,
      supported,
      setAutoCheck,
      check,
      updateNow,
      updateLater,
      remindMeLater,
      restart,
      closeDialog,
    }),
    [
      status,
      phase,
      progress,
      error,
      checking,
      dialogOpen,
      manual,
      lastCheckedAt,
      autoCheck,
      supported,
      setAutoCheck,
      check,
      updateNow,
      updateLater,
      remindMeLater,
      restart,
      closeDialog,
    ],
  )

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>
}

export function useUpdates(): UpdatesCtx {
  const ctx = useContext(Ctx)
  if (!ctx) throw new Error("useUpdates must be used within UpdatesProvider")
  return ctx
}
