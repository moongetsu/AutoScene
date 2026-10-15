import { FolderOpen } from "@/components/icons"
import { toast } from "sonner"
import type { MediaItem, Rational } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Select } from "@/components/ui/select"
import { commands, errorMessage } from "@/lib/ipc"
import { audioOption, codecLabel, currentAudio, formatFlags, releaseTitle } from "@/lib/media"
import { formatFps } from "@/lib/timecode"
import { Card } from "./Card"

/** The file: its name, picture, flags, audio track and folder. */
export function SourceCard({ pid, media: m, fps }: { pid: string; media: MediaItem; fps: Rational }) {
  const audio = m.info?.audio ?? []
  const flags = formatFlags(m)
  return (
    <Card title="Source">
      <p className="text-md font-medium text-fg-1">{releaseTitle(m.fileName)}</p>
      <p data-selectable className="mt-0.5 text-xs [overflow-wrap:anywhere] text-fg-3">
        {m.fileName}
      </p>
      {m.info && (
        <p className="timecode mt-2 text-xs text-fg-2">
          {m.info.video.width}×{m.info.video.height} · {formatFps(fps)} fps · {codecLabel(m.info.video.codec)}
        </p>
      )}
      {flags.length > 0 && (
        <div className="mt-2.5 flex flex-wrap gap-1.5">
          {flags.map((f) => (
            <Badge key={f} tone="outline" className="h-5 px-2 text-2xs">
              {f}
            </Badge>
          ))}
        </div>
      )}
      {audio.length > 1 && (
        <div className="mt-3">
          <Select
            size="sm"
            aria-label="Audio track"
            className="w-full"
            value={String(currentAudio(m))}
            onChange={(v) =>
              void commands
                .mediaSetAudio(pid, m.id, Number(v))
                .catch((e) => toast.error("Couldn't change the audio track", { description: errorMessage(e) }))
            }
            options={audio.map(audioOption)}
          />
        </div>
      )}
      <Button size="sm" variant="ghost" className="mt-3 -ml-2" onClick={() => void commands.revealPath(m.path)}>
        <FolderOpen /> Show in folder
      </Button>
    </Card>
  )
}
