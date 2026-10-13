import { useEffect } from "react"
import { Outlet } from "@tanstack/react-router"
import { NewProjectDialog } from "@/features/library/NewProjectDialog"
import { useGlobalBindings } from "@/lib/actions"
import { startEventBridge } from "@/lib/events"
import { startUpdateSchedule } from "@/state/updates"
import { ActivityDrawer } from "./ActivityDrawer"
import { CommandPalette } from "./CommandPalette"
import { DropZone } from "./DropZone"
import { Sidebar } from "./Sidebar"
import { TitleBar } from "./TitleBar"
import { UpdateDialog } from "./UpdateDialog"

export function AppShell() {
  useGlobalBindings()
  useEffect(() => startUpdateSchedule(), [])
  useEffect(() => startEventBridge(), [])

  return (
    <div className="flex h-full flex-col">
      <TitleBar />
      <div className="flex min-h-0 flex-1">
        <Sidebar />
        <main className="relative min-w-0 flex-1 overflow-hidden bg-canvas">
          <Outlet />
        </main>
      </div>
      <CommandPalette />
      <ActivityDrawer />
      <UpdateDialog />
      <NewProjectDialog />
      <DropZone />
    </div>
  )
}
