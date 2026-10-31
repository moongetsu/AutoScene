import { toast } from "sonner"
import { create } from "zustand"
import { commands, errorMessage, events, type UpdateStatus } from "@/lib/ipc"

type Phase = "idle" | "checking" | "current" | "available" | "downloading" | "installing" | "ready" | "error"

type UpdatesState = {
  phase: Phase
  status: UpdateStatus | null
  downloaded: number
  total: number | null
  error: string | null
  dialogOpen: boolean
  lastChecked: number | null
  /** Check at launch and every few hours (off: only when asked). */
  auto: boolean
  /**
   * `manual` checks always surface the result; launch checks respect snooze/skip.
   * `announce` also toasts "up to date" / failures, for checks started where
   * no settings page shows the answer (palette, menu).
   */
  check: (manual: boolean, announce?: boolean) => Promise<void>
  install: () => Promise<void>
  relaunch: () => Promise<void>
  remindLater: () => void
  skipVersion: () => void
  setDialogOpen: (open: boolean) => void
  setAuto: (on: boolean) => void
}

const SNOOZE_KEY = "autoscene.update.snoozeUntil"
const SKIP_KEY = "autoscene.update.skipVersion"
const AUTO_KEY = "autoscene.update.auto"
const SNOOZE_MS = 24 * 60 * 60 * 1000

function suppressed(version: string | null): boolean {
  if (!version) return true
  if (localStorage.getItem(SKIP_KEY) === version) return true
  const until = Number(localStorage.getItem(SNOOZE_KEY) ?? 0)
  return Date.now() < until
}

/** Automatic checks are on unless turned off (storage may refuse: then on). */
function autoChecks(): boolean {
  try {
    return localStorage.getItem(AUTO_KEY) !== "off"
  } catch {
    return true
  }
}

export const useUpdates = create<UpdatesState>((set, get) => ({
  phase: "idle",
  status: null,
  downloaded: 0,
  total: null,
  error: null,
  dialogOpen: false,
  lastChecked: null,
  auto: autoChecks(),

  async check(manual, announce = false) {
    const busy = ["checking", "downloading", "installing"].includes(get().phase)
    if (busy) return
    set({ phase: "checking", error: null })
    try {
      const status = await commands.updateCheck()
      const available = status.state === "available"
      set({
        status,
        phase: available ? "available" : "current",
        lastChecked: Date.now(),
        dialogOpen: available && (manual || !suppressed(status.version)),
      })
      // A manual check from the palette or menu has no settings page to show its answer.
      if (announce && !available) toast("AutoScene is up to date", { description: `Version ${status.currentVersion}` })
    } catch (e) {
      // Launch checks fail quietly (offline is normal); manual ones explain.
      set({ phase: manual ? "error" : "idle", error: errorMessage(e), lastChecked: Date.now() })
      if (announce) toast.error("Couldn't check for updates", { description: errorMessage(e) })
    }
  },

  async install() {
    set({ phase: "downloading", downloaded: 0, total: null, error: null })
    const unlisten = await events.updateProgress.listen(({ payload }) => {
      if (payload.downloaded === 0 && payload.total === null) set({ phase: "installing" })
      else set({ downloaded: payload.downloaded, total: payload.total })
    })
    try {
      await commands.updateInstall()
      set({ phase: "ready" })
    } catch (e) {
      set({ phase: "error", error: errorMessage(e) })
    } finally {
      unlisten()
    }
  },

  async relaunch() {
    try {
      const restarted = await commands.updateRelaunch()
      // Simulated installs don't restart; close the dialog and reset.
      if (!restarted) set({ dialogOpen: false, phase: "current" })
    } catch (e) {
      set({ phase: "error", error: errorMessage(e) })
    }
  },

  remindLater() {
    localStorage.setItem(SNOOZE_KEY, String(Date.now() + SNOOZE_MS))
    set({ dialogOpen: false })
  },

  skipVersion() {
    const v = get().status?.version
    if (v) localStorage.setItem(SKIP_KEY, v)
    set({ dialogOpen: false })
  },

  setDialogOpen: (dialogOpen) => set({ dialogOpen }),

  setAuto(auto) {
    try {
      if (auto) localStorage.removeItem(AUTO_KEY)
      else localStorage.setItem(AUTO_KEY, "off")
    } catch {
      // Kept for this session only.
    }
    set({ auto })
  },
}))

/** A version's release page (tags are "v" + the version, e.g. v1.0.3-BETA). */
export function releaseNotesUrl(version: string): string {
  return `https://github.com/tcmmi/AutoScene/releases/tag/v${encodeURIComponent(version)}`
}

const LAUNCH_DELAY_MS = 4_000
const INTERVAL_MS = 6 * 60 * 60 * 1000

/** Start the background update schedule once per app session. Each check
 *  asks the setting then, so turning it on or off applies without a restart. */
export function startUpdateSchedule(): () => void {
  const tick = () => {
    if (useUpdates.getState().auto) void useUpdates.getState().check(false)
  }
  const first = window.setTimeout(tick, LAUNCH_DELAY_MS)
  const every = window.setInterval(tick, INTERVAL_MS)
  return () => {
    window.clearTimeout(first)
    window.clearInterval(every)
  }
}
