import { motion } from "motion/react"
import { Clapperboard } from "lucide-react"
import { Progress } from "@/components/ui/progress"
import { formatBytes, formatEta, formatTransferRate } from "@/lib/format"
import { cn } from "@/lib/utils"

export type UploadPhase = "transfer" | "finalizing" | "done"

export type UploadProgressState = {
  file: File
  loaded: number
  total: number
  percent: number
  bytesPerSecond: number
  etaSeconds: number | null
  phase: UploadPhase
}

type UploadProgressBarProps = {
  upload: UploadProgressState
  className?: string
}

export function UploadProgressBar({ upload, className }: UploadProgressBarProps) {
  const pctLabel = upload.percent.toFixed(1)
  const hasRate = upload.bytesPerSecond > 0
  const remaining = Math.max(0, upload.total - upload.loaded)

  return (
    <div className={cn("border-t border-border/50 bg-background/50 px-5 py-4 backdrop-blur-sm", className)}>
      <div className="flex items-start gap-3 text-left">
        <div className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-primary/10 text-primary">
          <Clapperboard className="size-5" />
        </div>

        <div className="min-w-0 flex-1 space-y-3">
          <div className="flex items-start justify-between gap-3">
            <div className="min-w-0">
              <p className="truncate text-sm font-medium">{upload.file.name}</p>
              <p className="text-xs text-muted-foreground">
                {upload.phase === "finalizing"
                  ? "Bytes complete · waiting for server…"
                  : `${formatBytes(upload.loaded)} / ${formatBytes(upload.total)} · ${formatBytes(remaining)} left`}
              </p>
            </div>
            <span className="timecode shrink-0 text-sm font-medium tabular-nums text-primary">
              {pctLabel}%
            </span>
          </div>

          <div className="space-y-2">
            <Progress value={upload.percent} className="h-1.5 bg-muted/80" />
            <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-1 text-[11px] text-muted-foreground">
              <span className="timecode tabular-nums">
                {upload.phase === "finalizing"
                  ? "finalizing"
                  : hasRate
                    ? formatTransferRate(upload.bytesPerSecond)
                    : "measuring speed…"}
              </span>
              <span className="timecode tabular-nums">
                {upload.phase === "finalizing"
                  ? "almost there"
                  : hasRate
                    ? formatEta(upload.etaSeconds)
                    : "—"}
              </span>
            </div>
          </div>

          <motion.div
            className="h-px origin-left bg-primary/40"
            initial={{ scaleX: 0 }}
            animate={{ scaleX: upload.percent / 100 }}
            transition={{ type: "spring", stiffness: 120, damping: 22 }}
          />
        </div>
      </div>
    </div>
  )
}
