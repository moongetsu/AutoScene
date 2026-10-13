import { useEffect, useMemo } from "react"
import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"
import { useUi } from "@/state/ui"
import { useUpdates } from "@/state/updates"
import { commands, errorMessage, events } from "./ipc"
import { isMac, isTauri } from "./platform"
import { matchShortcut, type ShortcutId } from "./shortcuts"

/** App-level actions shared by the command palette, keyboard shortcuts and the macOS menu. */
export function useAppActions() {
  const navigate = useNavigate()
  return useMemo(
    () => ({
      openPalette: () => useUi.getState().setPaletteOpen(true),
      toggleActivity: () => useUi.getState().setActivityOpen(!useUi.getState().activityOpen),
      toggleSidebar: () => useUi.getState().toggleSidebar(),
      goLibrary: () => void navigate({ to: "/" }),
      goSettings: (section: SettingsSection = "general") =>
        void navigate({ to: "/settings/$section", params: { section } }),
      checkUpdates: () => void useUpdates.getState().check(true),
      openLogs: () => commands.openLogsDir().catch((e) => toast.error("Couldn't open logs", { description: errorMessage(e) })),
    }),
    [navigate],
  )
}

export const SETTINGS_SECTIONS = ["general", "shortcuts", "updates", "about"] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

/** Wire global keyboard shortcuts (non-macOS-native) and macOS menu events to actions. */
export function useGlobalBindings() {
  const a = useAppActions()

  useEffect(() => {
    const run: Record<ShortcutId, () => void> = {
      palette: a.openPalette,
      activity: a.toggleActivity,
      sidebar: a.toggleSidebar,
      settings: () => a.goSettings(),
      library: a.goLibrary,
    }
    // On macOS desktop the native menu owns accelerators and emits menu events instead.
    if (isMac && isTauri) return
    const onKey = (e: KeyboardEvent) => {
      const id = matchShortcut(e)
      if (!id) return
      e.preventDefault()
      run[id]()
    }
    window.addEventListener("keydown", onKey)
    return () => window.removeEventListener("keydown", onKey)
  }, [a])

  useEffect(() => {
    if (!isTauri) return
    const byId: Record<string, () => void> = {
      "app:check-updates": a.checkUpdates,
      "app:settings": () => a.goSettings(),
      "view:command-palette": a.openPalette,
      "view:library": a.goLibrary,
      "view:activity": a.toggleActivity,
      "view:toggle-sidebar": a.toggleSidebar,
    }
    const p = events.menuAction.listen(({ payload }) => byId[payload]?.())
    return () => void p.then((un) => un())
  }, [a])
}
