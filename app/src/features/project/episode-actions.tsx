import { ArrowCounterClockwise, Copy, FolderOpen, MagnifyingGlass, Play, Scissors, Trash, UserFocus, X } from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem } from "@/bindings"
import type { MenuKit } from "@/components/ui/menu-actions"
import { commands, errorMessage } from "@/lib/ipc"
import { copyText, reveal } from "@/lib/os"
import { useMediaJob } from "@/state/activity"
import { locateFile } from "./relink"

/**
 * What an episode (or a movie's file) offers wherever it is shown — its row,
 * its card, their ⋯ menus: open it, its file, analyse it again, remove it.
 */
export function EpisodeItems({
  kit,
  projectId,
  media: m,
  faces,
  onOpen,
  onRemove,
  openShortcut,
}: {
  kit: MenuKit
  projectId: string
  media: MediaItem
  /** The project finds characters (live action). */
  faces: boolean
  onOpen: () => void
  onRemove: () => void
  /** The key that opens it where it is shown (Enter on a focused row). */
  openShortcut?: string[]
}) {
  const { Item, Separator } = kit
  const job = useMediaJob(m.id)
  const isWorking = job?.state === "running" || job?.state === "queued"
  const ready = m.status === "ready"
  const redo = (stage: "shots" | "faces") =>
    void commands
      .mediaAnalyze(projectId, m.id, stage)
      .catch((e) => toast.error("Couldn't start the analysis", { description: errorMessage(e) }))
  const cancelJob = () => {
    if (!job) return
    commands
      .jobCancel(job.id)
      .catch((e) => toast.error("Couldn't cancel the job", { description: errorMessage(e) }))
  }
  return (
    <>
      <Item disabled={!ready} onSelect={onOpen} shortcut={openShortcut}>
        <Play /> Open
      </Item>
      <Item onSelect={() => reveal(m.path)}>
        <FolderOpen /> Show in folder
      </Item>
      <Item onSelect={() => void copyText(m.path, "file path")}>
        <Copy /> Copy file path
      </Item>
      {m.status === "missing" && (
        <Item onSelect={() => void locateFile(projectId, m)}>
          <MagnifyingGlass /> Locate file…
        </Item>
      )}
      {m.status === "failed" && !isWorking && (
        <Item
          onSelect={() =>
            void commands
              .mediaRetry(projectId, m.id)
              .catch((e) => toast.error("Couldn't index it again", { description: errorMessage(e) }))
          }
        >
          <ArrowCounterClockwise /> Index again
        </Item>
      )}
      {isWorking && (
        <Item onSelect={cancelJob}>
          <X /> Cancel analysis
        </Item>
      )}
      {ready && !isWorking && (
        <Item onSelect={() => redo("shots")}>
          <Scissors /> Find shots again
        </Item>
      )}
      {ready && faces && m.shots === "ready" && !isWorking && (
        <Item onSelect={() => redo("faces")}>
          <UserFocus /> Find characters again
        </Item>
      )}
      <Separator />
      <Item destructive disabled={isWorking} onSelect={onRemove}>
        <Trash /> Remove from project…
      </Item>
    </>
  )
}
