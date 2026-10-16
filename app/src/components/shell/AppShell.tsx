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

  // The browser's own menu (Save as, Print, Inspect…) has no place in an app.
  useEffect(() => {
    const onContextMenu = (e: MouseEvent) => {
      if (!wantsNativeMenu(e.target)) e.preventDefault()
    }
    window.addEventListener("contextmenu", onContextMenu)
    return () => window.removeEventListener("contextmenu", onContextMenu)
  }, [])

  return (
    <ContextMenu>
      <ContextMenuTrigger
        asChild
        // Text keeps the system menu: stop before any of our menus takes the click.
        onContextMenuCapture={(e) => wantsNativeMenu(e.target) && e.stopPropagation()}
      >
        <div className="flex h-full">
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
        <ContextMenuItem onSelect={() => window.history.back()}>
          <ArrowLeft /> Back
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => window.location.reload()}>
          <ArrowCounterClockwise /> Reload
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={actions.openPalette} shortcut={shortcutKeys("palette")}>
          <MagnifyingGlass /> Command palette…
        </ContextMenuItem>
        <ContextMenuItem onSelect={actions.newProject} shortcut={shortcutKeys("newProject")}>
          <Plus /> New project…
        </ContextMenuItem>
        <ContextMenuItem onSelect={actions.goLibrary} shortcut={shortcutKeys("library")}>
          <SquaresFour /> Library
        </ContextMenuItem>
        <ContextMenuItem onSelect={actions.toggleActivity} shortcut={shortcutKeys("activity")}>
          <Queue /> Activity
        </ContextMenuItem>
        <ContextMenuSeparator />
        <ContextMenuItem onSelect={() => actions.goSettings("shortcuts")}>
          <Keyboard /> Shortcuts
        </ContextMenuItem>
        <ContextMenuItem onSelect={() => actions.goSettings("general")} shortcut={shortcutKeys("settings")}>
          <GearSix /> Settings
        </ContextMenuItem>
      </ContextMenuContent>
    </ContextMenu>
  )
}

const TEXT_INPUTS = new Set(["text", "search", "url", "email", "password", "tel", "number"])

/** Text fields and selected text keep the system menu: cut, copy and paste. */
function wantsNativeMenu(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false
  const field = target.closest("input, textarea")
  if (field instanceof HTMLTextAreaElement || (field instanceof HTMLInputElement && TEXT_INPUTS.has(field.type))) return true
  if (target instanceof HTMLElement && target.isContentEditable) return true
  return target.closest("[data-selectable], code, pre") != null && !!window.getSelection()?.toString()
}
