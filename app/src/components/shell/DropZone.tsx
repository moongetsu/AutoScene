import { useEffect, useState } from "react"
import { useRouterState } from "@tanstack/react-router"
import { getCurrentWebview } from "@tauri-apps/api/webview"
import { FolderPlus } from "@/components/icons"
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
      // Only files from outside: text or a link dragged inside the window
      // (no paths) is no drop for a project. "over" keeps what "enter" said.
      if (payload.type === "enter") setOver(payload.paths.length > 0)
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
    <div className="pointer-events-none fixed inset-0 z-[60] p-4">
      <div className="flex size-full animate-fade-in flex-col items-center justify-center rounded-[28px] border-2 border-dashed border-accent-line bg-black/60 backdrop-blur-xl">
        <span className="flex size-20 items-center justify-center rounded-full bg-accent-subtle text-accent shadow-[0_0_60px_rgb(224_180_92/0.3)] [&_svg]:size-9">
          <FolderPlus />
        </span>
        <p className="display mt-6 text-4xl text-fg-1">{pid ? "Add to this project" : "Create a project"}</p>
        <p className="mt-2 text-md text-fg-2">Drop a season folder or video files</p>
      </div>
    </div>
  )
}
