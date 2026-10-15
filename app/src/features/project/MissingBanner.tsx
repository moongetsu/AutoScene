import { FolderOpen, Warning } from "@/components/icons"
import type { MediaItem } from "@/bindings"
import { Button } from "@/components/ui/button"
import { plural } from "@/features/cast/cast"
import { cn } from "@/lib/utils"
import { locateFolder } from "./relink"

/**
 * Episodes whose files can't be found (moved, renamed, on a drive that
 * isn't connected): one place to show AutoScene where they are now. Their
 * analysis and the decisions about them wait meanwhile.
 */
export function MissingBanner({
  pid,
  media,
  movie,
  className,
}: {
  pid: string
  media: MediaItem[] | undefined
  movie?: boolean
  className?: string
}) {
  const missing = media?.filter((m) => m.status === "missing").length ?? 0
  if (missing === 0) return null
  const what = movie ? plural(missing, "file") : plural(missing, "episode")
  return (
    <div
      role="status"
      className={cn(
        "flex flex-wrap items-center gap-x-4 gap-y-3 rounded-2xl border border-accent-line/50 bg-accent-subtle px-5 py-4",
        className,
      )}
    >
      <Warning weight="fill" className="size-5 shrink-0 text-accent" />
      <div className="min-w-0 flex-1">
        <p className="text-md font-medium text-fg-1">
          {what} can't be found
        </p>
        <p className="text-sm text-fg-2">
          Moved, renamed or on a drive that isn't connected? Their analysis and your decisions wait for them.
        </p>
      </div>
      <Button variant="secondary" onClick={() => void locateFolder(pid, missing)}>
        <FolderOpen /> Locate folder…
      </Button>
    </div>
  )
}
