import { FolderOpen } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { revealInFolder } from "@/lib/desktop"
import { formatClipsForCopy } from "@/lib/clipboard"
import { friendlyError } from "@/lib/plainCopy"
import { CopyButton } from "@/components/shared/CopyButton"

type ExportResult = {
  out_dir: string
  count: number
  clips: string[]
}

export function ExportResultPanel({ result }: { result: ExportResult }) {
  return (
    <div className="panel-surface animate-fade-up space-y-4 p-5">
      <div>
        <p className="text-sm font-medium text-primary">
          Exported {result.count} clip{result.count === 1 ? "" : "s"}
        </p>
        <p className="mt-1 break-all font-mono text-xs text-muted-foreground">{result.out_dir}</p>
      </div>

      <div className="flex flex-wrap gap-2">
        <CopyButton text={result.out_dir} label="Copy path" toastLabel="Folder path copied" />
        <CopyButton
          text={formatClipsForCopy(result.clips)}
          label="Copy clip list"
          toastLabel="Clip paths copied"
        />
        <Button
          variant="secondary"
          size="sm"
          className="gap-1.5"
          onClick={async () => {
            try {
              await revealInFolder(result.out_dir)
              toast.success("Opened in file manager")
            } catch (e) {
              toast.error(friendlyError(e, "Could not open folder"))
            }
          }}
        >
          <FolderOpen className="size-3.5" />
          Reveal folder
        </Button>
      </div>

      {result.clips.length > 0 ? (
        <div className="panel-inset max-h-48 overflow-auto p-3">
          <p className="mb-2 text-[10px] uppercase tracking-wider text-muted-foreground">
            Output files
          </p>
          <ul className="space-y-1.5">
            {result.clips.map((clip) => (
              <li
                key={clip}
                className="flex items-center justify-between gap-2 font-mono text-[11px] text-muted-foreground"
              >
                <span className="truncate">{clip.split(/[/\\]/).pop()}</span>
                <CopyButton
                  text={clip}
                  variant="ghost"
                  size="icon-sm"
                  toastLabel="Clip path copied"
                  className="shrink-0"
                />
              </li>
            ))}
          </ul>
        </div>
      ) : null}
    </div>
  )
}
