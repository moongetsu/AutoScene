import { motion } from "motion/react"
import { ArrowRight, Loader2, RefreshCw } from "lucide-react"
import { Button as AnimatedButton } from "@/components/ui/animated-button"
import { Button } from "@/components/ui/button"
import { Progress } from "@/components/ui/progress"
import type { Project } from "@/lib/api"
import { friendlyError, friendlyProjectStatus, sanitizeStage } from "@/lib/plainCopy"

type ScanProgressPanelProps = {
  project: Project
  onOpenGallery: () => void
  onRetry: () => void
}

export function ScanProgressPanel({ project, onOpenGallery, onRetry }: ScanProgressPanelProps) {
  const cutting = project.status === "cutting"
  const scanning = project.status === "scanning"
  const busy = cutting || scanning
  const scanPct = Math.round((project.progress || 0) * 100)
  const framesDone = project.frames_done ?? 0
  const framesTotal = project.frames_total ?? 0
  const frameLabel =
    framesTotal > 0
      ? `${framesDone}/${framesTotal}${project.frames_approx ? "~" : ""}`
      : "—"
  const cutInfer =
    project.progress_phase === "cuts_infer" ||
    project.progress_phase === "cuts_finalize"
  const phase = cutting
    ? cutInfer
      ? `${project.cuts_found ?? 0} scenes found`
      : "Finding scene breaks…"
    : sanitizeStage(project.progress_phase || project.stage || project.status)
  const shotCount = project.shot_count ?? project.shots?.length ?? 0
  const cutsFound = project.cuts_found ?? shotCount

  return (
    <motion.div
      initial={{ opacity: 0, y: 16 }}
      animate={{ opacity: 1, y: 0 }}
      className="panel-surface overflow-hidden border-[rgba(242,237,230,0.10)] bg-[#1C1A17]/80"
    >
      <div className="border-b border-[rgba(242,237,230,0.10)] bg-gradient-to-r from-[#C4A35A]/12 via-transparent to-transparent px-5 py-4">
        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <p className="text-[10px] uppercase tracking-[0.2em] text-[#9A948A]">
              {cutting ? "Finding scenes" : scanning ? "Scanning" : friendlyProjectStatus(project.status)}
            </p>
            <p className="truncate font-heading text-xl text-[#F2EDE6]">{project.name}</p>
            <p className="mt-1 text-sm text-[#9A948A]">
              {phase}
              {project.error
                ? ` · ${friendlyError(project.error, "Scan failed. Try again.")}`
                : ""}
            </p>
          </div>
          {busy ? <Loader2 className="size-5 shrink-0 animate-spin text-[#C4A35A]" /> : null}
        </div>
      </div>

      <div className="space-y-4 p-5">
        <div>
          <div className="mb-2 flex items-center justify-between text-xs text-[#9A948A]">
            <span>Progress</span>
            <span className="font-mono font-medium tabular-nums text-[#F2EDE6]">{scanPct}%</span>
          </div>
          <Progress value={scanPct} className="h-2 overflow-hidden bg-muted/60" />
        </div>

        <div className="grid grid-cols-3 gap-2 text-center">
          <Stat
            label={cutting ? "Scenes found" : "Shots"}
            value={
              cutting
                ? cutInfer
                  ? String(cutsFound)
                  : "—"
                : shotCount > 0
                  ? String(shotCount)
                  : "—"
            }
          />
          <Stat label="Frames" value={frameLabel} />
          <Stat label="Cast" value={String(project.people.length)} />
        </div>

        {project.status === "ready" ? (
          <AnimatedButton className="w-full gap-2" onClick={onOpenGallery}>
            Open gallery
            <ArrowRight className="size-4" />
          </AnimatedButton>
        ) : null}

        {project.status === "error" ? (
          <Button variant="outline" className="w-full gap-2" onClick={onRetry}>
            <RefreshCw className="size-4" />
            Retry scan
          </Button>
        ) : null}
      </div>
    </motion.div>
  )
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="panel-inset px-2 py-2">
      <p className="text-[10px] uppercase tracking-wider text-muted-foreground">{label}</p>
      <p className="timecode mt-0.5 text-sm font-medium capitalize">{value}</p>
    </div>
  )
}
