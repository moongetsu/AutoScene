import { useEffect, useState } from "react"
import { useRouterState } from "@tanstack/react-router"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { FolderInput } from "lucide-react"
import { toast } from "sonner"
import { commands, errorMessage } from "@/lib/ipc"
import { isTauri } from "@/lib/platform"
import { useNewProject } from "@/state/new-project"

/**
 * Files and folders dropped anywhere on the window: inside a project they are
 * added to it, elsewhere they start a new project (or fill the open New
 * Project dialog).
 */
export function DropZone() {
  const [over, setOver] = useState(false)
  const path = useRouterState({ select: (s) => s.location.pathname })
  const pid = path.match(/^\/p\/([^/]+)/)?.[1]

  useEffect(() => {
    if (!isTauri) return
    const un = getCurrentWebview().onDragDropEvent(({ payload }) => {
      if (payload.type === "enter" || payload.type === "over") setOver(true)
      else if (payload.type === "leave") setOver(false)
      else if (payload.type === "drop") {
        setOver(false)
        const paths = payload.paths
        if (!paths.length) return
        const np = useNewProject.getState()
        if (np.open) np.show([...np.seed, ...paths])
        else if (pid) {
          commands
            .mediaAdd(pid, paths)
            .then((n) => toast(n === 0 ? "Nothing new to add" : `Added ${n} ${n === 1 ? "file" : "files"}`))
            .catch((e) => toast.error("Couldn't add files", { description: errorMessage(e) }))
        } else np.show(paths)
      }
    })
    return () => void un.then((f) => f())
  }, [pid])

  if (!over) return null
  return (
    <div className="pointer-events-none fixed inset-0 top-[var(--titlebar-h)] z-[60] p-3">
      <div className="flex size-full flex-col items-center justify-center rounded-xl border-2 border-dashed border-accent-line bg-black/55 animate-fade-in">
        <FolderInput className="size-8 text-accent" />
        <p className="mt-3 text-lg font-semibold text-fg-1">{pid ? "Add to this project" : "Create a project"}</p>
        <p className="mt-1 text-sm text-fg-2">Drop a season folder or video files</p>
      </div>
    </div>
  )
}
