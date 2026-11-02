import { useEffect, useMemo } from "react"
import { useNavigate } from "@tanstack/react-router"
import { toast } from "sonner"
import { useNewProject } from "@/state/new-project"
import { useUi } from "@/state/ui"
import { useUpdates } from "@/state/updates"
import { commands, errorMessage, events } from "./ipc"
import { menuAnswers } from "./keymap"
import { layerOpen } from "./layers"
import { pickVideoFiles } from "./media"
import { isTauri } from "./platform"
import { GLOBAL_SHORTCUTS, type GlobalId, useShortcuts } from "./shortcuts"

/** App-level actions shared by the command palette, keyboard shortcuts and the macOS menu. */
export function useAppActions() {
  const navigate = useNavigate()
  return useMemo(
    () => ({
      openPalette: () => useUi.getState().setPaletteOpen(true),
      toggleActivity: () => useUi.getState().setActivityOpen(!useUi.getState().activityOpen),
      toggleSidebar: () => useUi.getState().toggleSidebar(),
      goLibrary: () => void navigate({ to: "/" }),
      goExtensions: () => void navigate({ to: "/extensions" }),
      newProject: () => useNewProject.getState().show(),
      goProject: (pid: string, page: "overview" | "episodes" | "cast" | "exports" = "overview") =>
        void navigate({ to: page === "overview" ? "/p/$pid" : `/p/$pid/${page}`, params: { pid } }),
      goSettings: (section: SettingsSection = "general") =>
        void navigate({ to: "/settings/$section", params: { section } }),
      checkUpdates: () => void useUpdates.getState().check(true, true),
      openLogs: () => commands.openLogsDir().catch((e) => toast.error("Couldn't open logs", { description: errorMessage(e) })),
    }),
    [navigate],
  )
}

/** DOM event asking the current screen to open its export dialog (macOS menu ⌘E). */
export const EXPORT_REQUEST = "autoscene:export"
/** macOS menu File → Reveal in Finder: the player reveals its source file. */
export const REVEAL_REQUEST = "autoscene:reveal"
/** macOS menu Edit → Copy Timecodes: the player copies its in/out (or current) timecode. */
export const COPY_TIMECODES_REQUEST = "autoscene:copy-timecodes"

/** The project the current route is about, if any. */
function currentProjectId(): string | undefined {
  const m = window.location.hash.match(/#\/p\/([^/]+)/) ?? window.location.pathname.match(/^\/p\/([^/]+)/)
  return m?.[1]
}

export const SETTINGS_SECTIONS = ["general", "shortcuts", "updates", "about", "contributors"] as const
export type SettingsSection = (typeof SETTINGS_SECTIONS)[number]

/** Wire global keyboard shortcuts (non-macOS-native) and macOS menu events to actions. */
export function useGlobalBindings() {
  const a = useAppActions()

  // On macOS desktop the native menu owns these accelerators and emits menu
  // events instead — except for a key the person chose that it can't carry.
  const page =
    (id: GlobalId, run: () => void) =>
    (): boolean | void => {
      if (menuAnswers(`global.${id}`)) return false
      run()
    }
  useShortcuts(
    GLOBAL_SHORTCUTS,
    {
      palette: page("palette", a.openPalette),
      activity: page("activity", a.toggleActivity),
      sidebar: page("sidebar", a.toggleSidebar),
      settings: page("settings", () => a.goSettings()),
      library: page("library", a.goLibrary),
      newProject: page("newProject", a.newProject),
    },
    { anywhere: ["palette", "activity"] },
  )

  useEffect(() => {
    if (!isTauri) return
    // Like their keys elsewhere: nothing moves away under an open dialog or menu.
    const calm =
      (run: () => void) =>
      () => {
        if (!layerOpen()) run()
      }
    const byId: Record<string, () => void> = {
      "app:check-updates": a.checkUpdates,
      "app:settings": calm(() => a.goSettings()),
      "view:command-palette": a.openPalette,
      "view:library": calm(a.goLibrary),
      "file:new-project": calm(a.newProject),
      // The native ⌘E accelerator swallows the keystroke; the player listens for this instead.
      "file:export": () => window.dispatchEvent(new CustomEvent(EXPORT_REQUEST)),
      "view:activity": a.toggleActivity,
      "view:toggle-sidebar": calm(a.toggleSidebar),
      "file:reveal": () => window.dispatchEvent(new CustomEvent(REVEAL_REQUEST)),
      "edit:copy-timecodes": () => window.dispatchEvent(new CustomEvent(COPY_TIMECODES_REQUEST)),
      "file:add-media": calm(() => {
        const pid = currentProjectId()
        if (!pid) return a.newProject()
        void pickVideoFiles().then((files) => {
          if (!files.length) return
          commands
            .mediaAdd(pid, files)
            .then((n) => toast(n === 0 ? "Nothing new to add" : `Added ${n} ${n === 1 ? "file" : "files"}`))
            .catch((e) => toast.error("Couldn't add files", { description: errorMessage(e) }))
        })
      }),
      "view:episodes": calm(() => {
        const pid = currentProjectId()
        if (pid) a.goProject(pid, "episodes")
      }),
      "view:exports": calm(() => {
        const pid = currentProjectId()
        if (pid) a.goProject(pid, "exports")
      }),
    }
    const p = events.menuAction.listen(({ payload }) => byId[payload]?.())
    return () => void p.then((un) => un())
  }, [a])
}
