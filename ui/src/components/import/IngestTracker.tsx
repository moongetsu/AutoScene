import { useEffect, useState } from "react"
import { motion } from "motion/react"
import {
  ArrowRight,
  Clapperboard,
  Film,
  FolderOpen,
  Loader2,
  Pause,
  Play,
  RefreshCw,
  ScanFace,
  Square,
} from "lucide-react"
import { Button as AnimatedButton } from "@/components/ui/animated-button"
import { Button } from "@/components/ui/button"
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/components/ui/alert-dialog"
import { Progress } from "@/components/ui/progress"
import { api, formatDuration, type Project } from "@/lib/api"
import { formatBytes, formatEta, formatTransferRate } from "@/lib/format"
import {
  friendlyBackend,
  friendlyError,
  friendlyModel,
  sanitizeStage,
} from "@/lib/plainCopy"
import { cn } from "@/lib/utils"
import type { UploadProgressState } from "@/components/import/UploadProgressBar"
import { useProjects } from "@/state/project"

export type IngestFileMeta = {
  name: string
  size: number
  type: string
  path?: string
}

type IngestTrackerProps = {
  project: Project | null
  file: IngestFileMeta | null
  upload: UploadProgressState | null
  onOpenGallery: () => void
  onRetry: () => void
  onImportAnother: () => void
  onPauseScan?: () => void
  onResumeScan?: () => void
  onStopScan?: () => void
  className?: string
}

const SCAN_STEPS = [
  { id: "cuts", label: "Scenes" },
  { id: "model", label: "Prepare" },
  { id: "detect", label: "Find faces" },
  { id: "cluster", label: "Group" },
  { id: "thumbs", label: "Portraits" },
  { id: "done", label: "Done" },
] as const

function stepIndex(phase: string | undefined, status?: string): number {
  if (status === "cutting" || phase === "cuts" || phase?.startsWith("cuts")) return 0
  if (phase === "probe") return 1
  const i = SCAN_STEPS.findIndex((s) => s.id === phase)
  return i >= 0 ? i : status === "scanning" ? 1 : 0
}

/** True while cut pass is actively processing frames (AI or AdaptiveDetector). */
function isCutInferPhase(phase: string | undefined): boolean {
  return (
    phase === "cuts_infer" ||
    phase === "cuts_finalize" ||
    phase === "cuts_extract"
  )
}

export function IngestTracker({
  project,
  file,
  upload,
  onOpenGallery,
  onRetry,
  onImportAnother,
  onPauseScan,
  onResumeScan,
  onStopScan,
  className,
}: IngestTrackerProps) {
  const uploading = Boolean(
    upload && (upload.phase === "transfer" || upload.phase === "finalizing"),
  )
  // While a new file uploads, ignore finished project status/cast (stale activeId).
  const cutting = !uploading && project?.status === "cutting"
  const scanning = !uploading && project?.status === "scanning"
  const paused = !uploading && project?.status === "paused"
  const scanActive = cutting || scanning || paused
  const ready = !uploading && project?.status === "ready"
  const errored = !uploading && project?.status === "error"

  const displayName = file?.name || project?.name || "Untitled video"
  const displayPath = file?.path || project?.video_path || null
  const [resolvedBytes, setResolvedBytes] = useState<number | null>(null)
  // Path-linked imports often have file.size === 0; prefer API filesystem stat / HEAD.
  const displaySize =
    (file?.size && file.size > 0 ? file.size : null) ??
    (project?.video_bytes && project.video_bytes > 0 ? project.video_bytes : null) ??
    resolvedBytes

  useEffect(() => {
    setResolvedBytes(null)
    if (!project?.id) return
    if ((file?.size && file.size > 0) || (project.video_bytes && project.video_bytes > 0)) {
      return
    }
    let cancelled = false
    void api
      .videoMeta(project.id)
      .then((meta) => {
        if (!cancelled && meta.video_bytes > 0) setResolvedBytes(meta.video_bytes)
      })
      .catch(() => {
        /* ignore — size stays hidden until available */
      })
    return () => {
      cancelled = true
    }
  }, [project?.id, project?.video_bytes, file?.size])

  const phaseLabel = uploading
    ? upload?.phase === "finalizing"
      ? "Finalizing"
      : "Uploading"
    : paused
      ? "Paused"
      : cutting
        ? "Finding scenes"
        : scanning
          ? "Scanning"
          : ready
            ? "Finished"
            : errored
              ? "Error"
              : "Queued"

  const scanPct = (project?.progress || 0) * 100
  const uploadPct = upload?.percent ?? 0
  const progressValue = uploading
    ? uploadPct
    : scanActive
      ? scanPct
      : ready
        ? 100
        : 0

  const framesDone = project?.frames_done ?? 0
  const framesTotal = project?.frames_total ?? 0
  const framesApprox = Boolean(project?.frames_approx)
  const detectHits = project?.detect_hits ?? 0
  const scanEta = project?.eta_sec ?? null
  const activeStep = stepIndex(project?.progress_phase, project?.status)
  const shotCount = project?.shot_count ?? project?.shots?.length ?? 0
  const cutsFound = project?.cuts_found ?? shotCount
  const cutLive =
    cutting &&
    (isCutInferPhase(project?.progress_phase) ||
      (project?.frames_total ?? 0) > 0 ||
      (project?.cuts_found ?? 0) > 0)
  const cutFramesDone = cutting ? (project?.frames_done ?? 0) : framesDone
  const cutFramesTotal = cutting ? (project?.frames_total ?? 0) : framesTotal

  const { importUi } = useProjects()
  const [stopOpen, setStopOpen] = useState(false)

  // Last good crop for this scan only. detect_hits === 0 ⇒ no face UI (new scan / none yet).
  const [liveFaceSrc, setLiveFaceSrc] = useState<string | null>(() => {
    if (
      project &&
      (project.detect_hits ?? 0) > 0 &&
      importUi.liveFaceProjectId === project.id &&
      importUi.liveFaceRev > 0
    ) {
      return api.liveFaceUrl(project.id, importUi.liveFaceRev)
    }
    return null
  })
  const [liveFaceHits, setLiveFaceHits] = useState(() => {
    if (
      project &&
      (project.detect_hits ?? 0) > 0 &&
      importUi.liveFaceProjectId === project.id
    ) {
      return importUi.liveFaceRev
    }
    return 0
  })

  const projectId = project?.id ?? null

  useEffect(() => {
    if (!projectId) {
      setLiveFaceSrc(null)
      setLiveFaceHits(0)
      return
    }
    if (detectHits > 0) {
      // Keep showing latest crop even on frames with no new face (hits stay > 0).
      setLiveFaceSrc(api.liveFaceUrl(projectId, detectHits))
      setLiveFaceHits(detectHits)
      return
    }
    // Zero hits: waiting state. Do not reuse a previous scan's crop.
    setLiveFaceSrc(null)
    setLiveFaceHits(0)
  }, [projectId, detectHits])

  const fallbackCastSrc = (() => {
    if (!project?.people?.length) return null
    const top = [...project.people].sort(
      (a, b) =>
        (b.detection_count || 0) - (a.detection_count || 0) ||
        (b.screen_time_sec || 0) - (a.screen_time_sec || 0),
    )[0]
    return top?.thumbnail_path
      ? api.mediaUrl(project.id, top.thumbnail_path)
      : null
  })()

  const faceStillSrc = liveFaceSrc || fallbackCastSrc
  const showFaceStill = Boolean(
    project && faceStillSrc && (scanActive || ready || errored) && (detectHits > 0 || fallbackCastSrc),
  )

  const progressPct = progressValue.toFixed(1)

  return (
    <motion.div
      initial={{ opacity: 0, y: 14 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{ type: "spring", stiffness: 260, damping: 28 }}
      className={cn(
        "relative flex min-h-[min(78dvh,740px)] flex-col overflow-hidden rounded-[1.35rem] border border-border/40 bg-[oklch(0.07_0.012_280)]",
        className,
      )}
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-0 bg-[radial-gradient(80%_55%_at_20%_0%,oklch(0.78_0.14_65/0.1),transparent_55%)]"
      />

      <div className="relative z-10 flex items-center justify-between gap-3 border-b border-border/35 px-5 py-3">
        <div className="flex min-w-0 flex-wrap items-center gap-3">
          <div className="flex items-center gap-2">
            <span
              className={cn(
                "size-1.5 rounded-full",
                uploading || scanning || cutting
                  ? "bg-[#C4A35A] animate-pulse"
                  : paused
                    ? "bg-amber-400"
                    : ready
                      ? "bg-emerald-400"
                      : errored
                        ? "bg-destructive"
                        : "bg-muted-foreground/50",
              )}
            />
            <p className="text-[10px] uppercase tracking-[0.22em] text-[#9A948A]">
              Ingest bay
            </p>
          </div>
          <span
            className={cn(
              "inline-flex items-center gap-1.5 rounded-md border px-2 py-1 text-[11px] font-medium",
              uploading || scanning || cutting
                ? "border-[#C4A35A]/35 bg-[#C4A35A]/10 text-[#C4A35A]"
                : paused
                  ? "border-amber-500/30 bg-amber-500/10 text-amber-400"
                  : ready
                    ? "border-emerald-500/30 bg-emerald-500/10 text-emerald-400"
                    : errored
                      ? "border-destructive/30 bg-destructive/10 text-destructive"
                      : "border-border/50 bg-muted/40 text-muted-foreground",
            )}
          >
            {(uploading || scanning || cutting) && (
              <Loader2 className="size-3 animate-spin" />
            )}
            {paused ? <Pause className="size-3" /> : null}
            {phaseLabel}
          </span>
          <span className="timecode text-[11px] tabular-nums text-muted-foreground">
            {progressPct}%
          </span>
        </div>
        <Button type="button" variant="ghost" size="sm" onClick={onImportAnother}>
          Import another
        </Button>
      </div>

      <div className="relative z-10 grid min-h-0 flex-1 gap-0 xl:grid-cols-[minmax(0,1.15fr)_minmax(320px,0.85fr)]">
        <div className="flex min-h-0 flex-col border-b border-border/35 xl:border-b-0 xl:border-r xl:border-border/35">
          {/* Program frame — one square size (w+max-h together broke aspect). */}
          <div className="relative m-3 mx-auto aspect-square w-[min(100%,42dvh,360px)] shrink-0 overflow-hidden rounded-lg border border-border/30 bg-background sm:w-[min(100%,46dvh,400px)] xl:m-4 xl:mx-auto">
            {/* Corner ticks */}
            <span aria-hidden className="pointer-events-none absolute left-2 top-2 z-[1] h-3 w-px bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute left-2 top-2 z-[1] h-px w-3 bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute right-2 top-2 z-[1] h-3 w-px bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute right-2 top-2 z-[1] h-px w-3 bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute bottom-2 left-2 z-[1] h-3 w-px bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute bottom-2 left-2 z-[1] h-px w-3 bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute bottom-2 right-2 z-[1] h-3 w-px bg-primary/50" />
            <span aria-hidden className="pointer-events-none absolute bottom-2 right-2 z-[1] h-px w-3 bg-primary/50" />
            {showFaceStill && faceStillSrc ? (
              <img
                src={faceStillSrc}
                alt={ready ? "Last detected face" : "Latest detected face"}
                className="absolute inset-0 size-full object-cover object-center"
                draggable={false}
              />
            ) : scanActive && project ? (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-4 text-center text-[#9A948A]">
                {cutting ? (
                  <Film className="size-12 opacity-40" />
                ) : (
                  <ScanFace className="size-12 opacity-40" />
                )}
                {paused ? (
                  <p className="text-sm text-[#F2EDE6]/80">
                    Paused. Resume to continue detecting.
                  </p>
                ) : cutting && !cutLive ? (
                  <p className="text-sm text-[#F2EDE6]/80">Finding scene breaks…</p>
                ) : !cutting ? (
                  <p className="text-sm text-[#F2EDE6]/80">Waiting for first face…</p>
                ) : null}
                {cutting && cutLive ? (
                  <div className="mt-1 flex flex-col items-center gap-2">
                    <p className="font-mono text-2xl tabular-nums tracking-tight text-[#C4A35A]">
                      {cutsFound}
                      <span className="ml-2 text-xs uppercase tracking-[0.18em] text-[#9A948A]">
                        scenes found
                      </span>
                    </p>
                    <p className="font-mono text-[11px] tabular-nums text-[#9A948A]">
                      {cutFramesTotal > 0
                        ? `Frame ${cutFramesDone}/${cutFramesTotal}`
                        : "…"}
                      {" · "}
                      {Math.round(scanPct)}%
                    </p>
                    <div className="h-1.5 w-48 overflow-hidden rounded-full bg-[#141210]">
                      <motion.div
                        className="h-full rounded-full bg-[#C4A35A]"
                        initial={false}
                        animate={{ width: `${Math.min(100, Math.max(2, scanPct))}%` }}
                        transition={{ type: "spring", stiffness: 100, damping: 20 }}
                      />
                    </div>
                  </div>
                ) : null}
              </div>
            ) : project && (ready || errored) ? (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-4 text-center text-muted-foreground">
                <ScanFace className="size-12 opacity-40" />
                <p className="text-sm">
                  {errored ? "Scan failed" : "No face preview saved for this scan"}
                </p>
              </div>
            ) : (
              <div className="absolute inset-0 flex flex-col items-center justify-center gap-3 px-4 text-center text-muted-foreground">
                <Film className="size-12 opacity-40" />
                <p className="text-sm">Holding your footage while work runs</p>
              </div>
            )}
            {uploading && (
              <div className="pointer-events-none absolute inset-0 bg-gradient-to-t from-background/80 via-transparent to-transparent" />
            )}
            {showFaceStill ? (
              <div className="pointer-events-none absolute bottom-3 left-3 z-[2] rounded-md border border-primary/30 bg-background/85 px-2 py-1 text-[10px] uppercase tracking-[0.18em] text-primary backdrop-blur-sm">
                {scanActive
                  ? `Live face · ${liveFaceHits || detectHits} found`
                  : "Last face"}
              </div>
            ) : null}
          </div>

          <div className="flex items-start gap-3 px-5 pb-4 pt-1">
            <div className="flex size-10 shrink-0 items-center justify-center rounded-lg border border-primary/20 bg-primary/10 text-primary">
              <Clapperboard className="size-4" />
            </div>
            <div className="min-w-0 flex-1">
              <p className="truncate font-heading text-lg leading-tight tracking-tight">
                {displayName}
              </p>
              {displayPath ? (
                <p className="mt-1 flex items-start gap-1.5 break-all font-mono text-[11px] text-muted-foreground">
                  <FolderOpen className="mt-0.5 size-3 shrink-0" />
                  {displayPath}
                </p>
              ) : null}
              <p className="timecode mt-1 text-[11px] text-muted-foreground">
                {displaySize != null ? formatBytes(displaySize) : null}
                {displaySize != null && file?.type ? " · " : null}
                {file?.type || "video"}
                {project && project.duration_sec > 0
                  ? ` · ${formatDuration(project.duration_sec)}`
                  : null}
              </p>
            </div>
          </div>
        </div>

        <div className="flex flex-col justify-between gap-5 p-5 xl:p-6">
          <div className="space-y-5">
            <div>
              <p className="text-[10px] uppercase tracking-[0.22em] text-muted-foreground">
                Progress
              </p>
              <p className="font-heading mt-1.5 text-2xl tracking-tight">
                {uploading
                  ? upload?.phase === "finalizing"
                    ? "Finishing upload"
                    : "Uploading"
                  : paused
                    ? "Paused"
                    : cutting
                      ? sanitizeStage(project?.stage) || "Finding scenes"
                      : scanning
                        ? sanitizeStage(project?.stage) || "Scanning faces"
                        : ready
                          ? "Finished"
                          : errored
                            ? "Error"
                            : "Preparing"}
              </p>
              {errored && project?.error ? (
                <p className="mt-2 text-sm text-destructive">
                  {friendlyError(project.error, "Scan failed. Try again.")}
                </p>
              ) : (
                <p className="mt-2 text-sm text-muted-foreground">
                  {uploading
                    ? "Upload progress. Time left updates as speed settles."
                    : paused
                      ? "Paused. Resume or stop when ready."
                      : cutting
                        ? sanitizeStage(project?.stage) || "Finding scene breaks…"
                        : scanning
                          ? framesTotal > 0
                            ? `Progress ${framesApprox ? "~" : ""}${framesDone}/${framesTotal}`
                            : "Preparing…"
                          : ready
                            ? "Cast is ready. Open the gallery to review characters."
                            : "Waiting for scan to start."}
                </p>
              )}
            </div>

            {/* Dual rail: upload always shown when present; scan when project active */}
            <div className="space-y-3">
              {upload ? (
                <Meter
                  label="Upload"
                  value={upload.percent}
                  exact={`${upload.percent.toFixed(1)}%`}
                  detail={
                    upload.phase === "finalizing"
                      ? "Finishing upload…"
                      : `${formatBytes(upload.loaded)} / ${formatBytes(upload.total)} · ${formatBytes(Math.max(0, upload.total - upload.loaded))} left · ${
                          upload.bytesPerSecond > 0
                            ? formatTransferRate(upload.bytesPerSecond)
                            : "…"
                        }`
                  }
                  eta={
                    upload.phase === "finalizing"
                      ? "finishing"
                      : formatEta(upload.etaSeconds)
                  }
                  active={uploading}
                />
              ) : null}

              {(scanActive || ready || (!uploading && project)) && (
                <Meter
                  label="Scan"
                  value={ready ? 100 : scanPct}
                  exact={`${(ready ? 100 : scanPct).toFixed(1)}%`}
                  detail={
                    cutting && cutFramesTotal > 0
                      ? `Frame ${cutFramesDone}/${cutFramesTotal} · ${cutsFound} scenes`
                      : scanActive && framesTotal > 0
                        ? `Progress ${framesApprox ? "~" : ""}${framesDone}/${framesTotal} · ${detectHits} faces`
                        : cutting
                          ? sanitizeStage(project?.stage) || "Finding scenes"
                          : scanning
                            ? sanitizeStage(project?.progress_phase) || "Working"
                            : paused
                              ? "paused"
                              : ready
                                ? `${project?.people.length ?? 0} cast · ${project?.segments.length ?? 0} scenes`
                                : "—"
                  }
                  eta={
                    cutting
                      ? "…"
                      : scanning
                        ? formatEta(scanEta)
                        : paused
                          ? "held"
                          : ready
                            ? "done"
                            : "—"
                  }
                  active={scanning || cutting}
                />
              )}
            </div>

            {scanActive || ready ? (
              <ol className="grid grid-cols-7 gap-1">
                {SCAN_STEPS.map((step, i) => {
                  const done = ready || i < activeStep
                  const current = !ready && i === activeStep
                  return (
                    <li key={step.id} className="text-center">
                      <div
                        className={cn(
                          "mx-auto h-1 rounded-full transition-colors",
                          done || current ? "bg-[#C4A35A]" : "bg-muted",
                          current && "animate-pulse",
                        )}
                      />
                      <p
                        className={cn(
                          "mt-1.5 text-[9px] uppercase tracking-wider",
                          current
                            ? "text-[#C4A35A]"
                            : done
                              ? "text-foreground/70"
                              : "text-muted-foreground/60",
                        )}
                      >
                        {step.label}
                      </p>
                    </li>
                  )
                })}
              </ol>
            ) : null}

            <dl className="grid grid-cols-2 gap-2">
              <Meta
                label="Footage type"
                value={friendlyBackend(project?.backend)}
              />
              <Meta
                label="Quality pack"
                value={friendlyModel(project?.backend, project?.model)}
              />
              <Meta
                label="Scenes"
                value={
                  cutting
                    ? String(cutsFound)
                    : cutsFound > 0 || shotCount > 0
                      ? String(cutsFound > 0 ? cutsFound : shotCount)
                      : "—"
                }
              />
              <Meta
                label="Shots"
                value={
                  shotCount > 0
                    ? String(shotCount)
                    : cutting
                      ? String(cutsFound || "…")
                      : "—"
                }
              />
              <Meta
                label="Frames"
                value={
                  cutting && cutFramesTotal > 0
                    ? `${cutFramesDone}/${cutFramesTotal}`
                    : framesTotal > 0
                      ? `${framesApprox ? "~" : ""}${framesDone}/${framesTotal}`
                      : "—"
                }
              />
              <Meta label="Faces" value={detectHits > 0 ? String(detectHits) : "—"} />
            </dl>
          </div>

          <div className="flex flex-col gap-2">
            {scanActive ? (
              <div className="grid grid-cols-2 gap-2">
                {cutting ? (
                  <Button variant="outline" className="h-10 gap-2" disabled>
                    <Film className="size-4" />
                    Cutting scenes…
                  </Button>
                ) : scanning ? (
                  <Button
                    variant="outline"
                    className="h-10 gap-2"
                    onClick={onPauseScan}
                    disabled={!onPauseScan}
                  >
                    <Pause className="size-4" />
                    Pause
                  </Button>
                ) : (
                  <Button
                    variant="outline"
                    className="h-10 gap-2 border-amber-500/40 text-amber-400 hover:bg-amber-500/10"
                    onClick={onResumeScan}
                    disabled={!onResumeScan}
                  >
                    <Play className="size-4" />
                    Resume
                  </Button>
                )}
                <Button
                  variant="outline"
                  className="h-10 gap-2 border-destructive/40 text-destructive hover:bg-destructive/10"
                  onClick={() => setStopOpen(true)}
                  disabled={!onStopScan}
                >
                  <Square className="size-3.5 fill-current" />
                  Stop
                </Button>
              </div>
            ) : null}
            {ready ? (
              <AnimatedButton className="h-10 w-full gap-2" onClick={onOpenGallery}>
                Open gallery
                <ArrowRight className="size-4" />
              </AnimatedButton>
            ) : null}
            {errored ? (
              <Button variant="outline" className="h-10 w-full gap-2" onClick={onRetry}>
                <RefreshCw className="size-4" />
                Retry scan
              </Button>
            ) : null}
            {(uploading || scanActive) && (
              <p className="text-center text-[11px] text-muted-foreground">
                {progressValue.toFixed(1)}% · you can leave; work keeps running.
              </p>
            )}
          </div>
        </div>
      </div>

      <AlertDialog open={stopOpen} onOpenChange={setStopOpen}>
        <AlertDialogContent size="default" className="max-w-md">
          <AlertDialogHeader>
            <AlertDialogTitle>Stop scan?</AlertDialogTitle>
            <AlertDialogDescription>
              Progress is kept but the scan ends.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel>Keep scanning</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              onClick={(e) => {
                e.preventDefault()
                setStopOpen(false)
                onStopScan?.()
              }}
            >
              Stop scan
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </motion.div>
  )
}

function Meter({
  label,
  value,
  exact,
  detail,
  eta,
  active,
}: {
  label: string
  value: number
  exact: string
  detail: string
  eta: string
  active: boolean
}) {
  return (
    <div
      className={cn(
        "rounded-xl border px-3 py-3",
        active ? "border-primary/35 bg-primary/5" : "border-border/40 bg-background/30",
      )}
    >
      <div className="mb-2 flex items-center justify-between gap-2">
        <span className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">
          {label}
        </span>
        <span className="timecode text-sm font-medium tabular-nums text-primary">{exact}</span>
      </div>
      <Progress value={Math.min(100, Math.max(0, value))} className="h-2 bg-muted/70" />
      <div className="mt-2 flex items-start justify-between gap-3 text-[11px] text-muted-foreground">
        <span className="timecode min-w-0 truncate tabular-nums">{detail}</span>
        <span className="timecode shrink-0 tabular-nums">{eta}</span>
      </div>
    </div>
  )
}

function Meta({ label, value }: { label: string; value: string }) {
  return (
    <div className="rounded-lg border border-border/40 bg-background/40 px-3 py-2">
      <dt className="text-[10px] uppercase tracking-wider text-muted-foreground">{label}</dt>
      <dd className="timecode mt-0.5 truncate text-sm font-medium">{value}</dd>
    </div>
  )
}
