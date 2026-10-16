import { useEffect } from "react"
import { Outlet } from "@tanstack/react-router"
import {
  ArrowCounterClockwise,
  ArrowLeft,
  GearSix,
  Keyboard,
  MagnifyingGlass,
  Plus,
  Queue,
  SquaresFour,
} from "@/components/icons"
import {
  ContextMenu,
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuSeparator,
  ContextMenuTrigger,
} from "@/components/ui/context-menu"
import { NewProjectDialog } from "@/features/library/NewProjectDialog"
import { RelinkDialog } from "@/features/project/RelinkDialog"
import { useAppActions, useGlobalBindings } from "@/lib/actions"
import { startEventBridge } from "@/lib/events"
import { useLayerSafetyNet } from "@/lib/layers"
import { isTauri, isWindows } from "@/lib/platform"
import { shortcutKeys } from "@/lib/shortcuts"
import { startUpdateSchedule } from "@/state/updates"
import { useUi } from "@/state/ui"
import { ActivityDrawer } from "./ActivityDrawer"
import { CommandPalette } from "./CommandPalette"
import { DropZone } from "./DropZone"
import { Rail } from "./Rail"
import { UpdateDialog } from "./UpdateDialog"
import { WindowControls } from "./WindowControls"
import { WindowTitle } from "./WindowTitle"

/**
 * The window: the rail on the left (on the native material), the view on
 * the rest, its top bar floating over it. The Windows caption buttons float
 * over the top-right corner so every view has them.
 */
export function AppShell() {
  const actions = useAppActions()
  useGlobalBindings()
  useLayerSafetyNet()
  useEffect(() => startUpdateSchedule(), [])
  useEffect(() => startEventBridge(), [])
  const railHidden = useUi((s) => s.sidebarCollapsed)

  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      const target = e.target as HTMLElement | null
      // Let text inputs and textareas keep native context menu for cut/copy/paste
      if (target?.tagName === "INPUT" || target?.tagName === "TEXTAREA" || target?.isContentEditable) {
        return
      }
      // Prevent the default browser context menu everywhere else
      e.preventDefault()
    }
    window.addEventListener("contextmenu", onContextMenu)
    return () => window.removeEventListener("contextmenu", onContextMenu)
  }, [])

  return (
    <ContextMenu>
      <ContextMenuTrigger asChild>
        <div className="flex h-full select-none">
          {!railHidden && <Rail />}
          <main className="relative min-w-0 flex-1 overflow-hidden bg-canvas">
            <Outlet />
          </main>
          {isWindows && isTauri && <WindowControls />}
          <WindowTitle />
          <CommandPalette />
          <ActivityDrawer />
          <UpdateDialog />
          <NewProjectDialog />
          <RelinkDialog />
          <DropZone />
        </div>
      </ContextMenuTrigger>
      <ContextMenuContent>
        <ContextMenuItem onSelect={() => window.history.back()} shortcut="Alt+←">
          <ArrowLeft /> Back
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => window.location.reload()} shortcut="Ctrl+R">
          <ArrowCounterClockwise /> Reload
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={actions.openPalette} shortcut={shortcutKeys("palette").join("+")}>
          <MagnifyingGlass /> Command palette…
        </ContextMenuItem>
        <ContextMenuItem onSelect={actions.newProject} shortcut={shortcutKeys("newProject").join("+")}>
          <Plus /> New project…
        </ContextMenuItem>
        <ContextMenuItem onSelect={actions.goLibrary} shortcut={shortcutKeys("library").join("")}>
          <SquaresFour /> Library
        </ContextMenuItem>
        <ContextMenuItem onSelect={actions.toggleActivity} shortcut={shortcutKeys("activity").join("+")}>
          <Queue /> Activity
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => actions.goSettings("shortcuts")} shortcut="?">
          <Keyboard /> Shortcuts
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => actions.goSettings("general")} shortcut={shortcutKeys("settings").join("")}>
          <GearSix /> Settings
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

