import { FileVideo, FolderOpen } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { revealInFolder } from "@/lib/desktop"
import { friendlyError } from "@/lib/plainCopy"
import { CopyButton } from "@/components/shared/CopyButton"

type MergedExportResult = {
  output_path: string
  scene_count: number
  person_label: string
}

export function MergedExportResultPanel({ result }: { result: MergedExportResult }) {
  return (
    <div className="panel-surface animate-fade-up space-y-4 p-5">
      <div className="flex items-start gap-3">
        <div className="rounded-lg bg-primary/10 p-2.5 text-primary">
          <FileVideo className="size-5" />
        </div>
        <div className="min-w-0 flex-1">
          <p className="text-sm font-medium text-primary">
            Merged {result.scene_count} scene{result.scene_count === 1 ? "" : "s"} for{" "}
            {result.person_label}
          </p>
          <p className="mt-1 break-all font-mono text-xs text-muted-foreground">
            {result.output_path}
          </p>
        </div>
      </div>

      <div className="flex flex-wrap gap-2">
        <CopyButton text={result.output_path} label="Copy path" toastLabel="File path copied" />
        <Button
          variant="secondary"
          size="sm"
          className="gap-1.5"
          onClick={async () => {
            try {
              await revealInFolder(result.output_path)
              toast.success("Opened in file manager")
            } catch (e) {
              toast.error(friendlyError(e, "Could not open folder"))
            }
          }}
        >
          <FolderOpen className="size-3.5" />
          Reveal file
        </Button>
      </div>
    </div>
  )
}
