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
}

const SNOOZE_KEY = "autoscene.update.snoozeUntil"
const SKIP_KEY = "autoscene.update.skipVersion"
const SNOOZE_MS = 24 * 60 * 60 * 1000

function suppressed(version: string | null): boolean {
  if (!version) return true
  if (localStorage.getItem(SKIP_KEY) === version) return true
  const until = Number(localStorage.getItem(SNOOZE_KEY) ?? 0)
  return Date.now() < until
}

export const useUpdates = create<UpdatesState>((set, get) => ({
  phase: "idle",
  status: null,
  downloaded: 0,
  total: null,
  error: null,
  dialogOpen: false,
  lastChecked: null,

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
}))

const LAUNCH_DELAY_MS = 4_000
const INTERVAL_MS = 6 * 60 * 60 * 1000

/** Start the background update schedule once per app session. */
export function startUpdateSchedule(): () => void {
  const first = window.setTimeout(() => void useUpdates.getState().check(false), LAUNCH_DELAY_MS)
  const every = window.setInterval(() => void useUpdates.getState().check(false), INTERVAL_MS)
  return () => {
    window.clearTimeout(first)
    window.clearInterval(every)
  }
}
