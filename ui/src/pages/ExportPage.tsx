import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useNavigate } from "react-router-dom"
import {
  Clapperboard,
  Files,
  LayoutGrid,
  ListPlus,
  Loader2,
  RotateCcw,
  ScanFace,
  Square,
} from "lucide-react"
import { toast } from "sonner"
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
import { Button as AnimatedButton } from "@/components/ui/animated-button"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Label } from "@/components/ui/label"
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select"
import { PageHeader } from "@/components/app/PageHeader"
import { EmptyState } from "@/components/app/EmptyState"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { SceneClipGrid } from "@/components/export/SceneClipGrid"
import { MergedExportResultPanel } from "@/components/export/MergedExportResultPanel"
import { ExportResultPanel } from "@/components/export/ExportResultPanel"
import { ExportTargetsPanel } from "@/components/export/ExportTargetsPanel"
import { FacePortrait } from "@/components/shared/FacePortrait"
import {
  api,
  formatDuration,
  isSegmentExcluded,
  type ExportQueueSnapshot,
  type MergedExport,
  type Segment,
} from "@/lib/api"
import { pickMergedVideoPath } from "@/lib/desktop"
import {
  buildSharedScenesClient,
  segmentPreviewRange,
  segmentStillTime,
} from "@/lib/scenes"
import { markAuthoritativeWrite, useProjects } from "@/state/project"
import { useMenuAction } from "@/state/menu-actions"
import { friendlyError, sanitizeStage } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

type MergedExportResult = {
  output_path: string
  scene_count: number
  person_label: string
}

/** Shape returned by POST /api/projects/{id}/export (one file per scene). */
type ClipExportResult = {
  out_dir: string
  count: number
  clips: string[]
}

type ExportMode = "quality" | "lossless" | "copy"
type VideoCodec =
  | "h264_nvenc"
  | "hevc_nvenc"
  | "av1_nvenc"
  | "h264_videotoolbox"
  | "hevc_videotoolbox"

function safeFilename(label: string) {
  return label.replace(/[<>:"/\\|?*]+/g, "-").trim() || "character"
}

export function ExportPage() {
  const nav = useNavigate()
  const {
    active,
    activeId,
    projects,
    projectsReady,
    selectedPersonId,
    setSelectedPersonId,
    setActiveId,
    refreshActive,
  } = useProjects()

  // Sidebar "Export" does not pin a project — restore the newest ready cast
  // so clicking Export after a scan does not land on a blank empty state.
  useEffect(() => {
    if (!projectsReady || activeId) return
    const candidates = projects
      .filter(
        (p) =>
          (p.status === "ready" || p.status === "exporting") &&
          (p.people?.length ?? 0) > 0,
      )
      .sort(
        (a, b) =>
          (Date.parse(b.updated_at || "") || 0) -
          (Date.parse(a.updated_at || "") || 0),
      )
    if (candidates[0]) setActiveId(candidates[0].id)
  }, [projectsReady, activeId, projects, setActiveId])
  const [exportMode, setExportMode] = useState<ExportMode>("quality")
  const [videoCodec, setVideoCodec] = useState<VideoCodec>("h264_nvenc")
  const [enqueueBusy, setEnqueueBusy] = useState(false)
  const [forceReencode, setForceReencode] = useState(false)
  const [codecCaps, setCodecCaps] = useState<Record<string, boolean>>({})
  // The UI offers friendly H.264/HEVC labels; the concrete encoder depends on
  // which hardware family this machine reported.
  const h264Codec: VideoCodec = codecCaps.h264_videotoolbox
    ? "h264_videotoolbox"
    : "h264_nvenc"
  const hevcCodec: VideoCodec = codecCaps.hevc_videotoolbox
    ? "hevc_videotoolbox"
    : "hevc_nvenc"
  const hwEncoderName = codecCaps.h264_videotoolbox
    ? "GPU VideoToolbox"
    : "GPU NVENC"
  const [toggleBusy, setToggleBusy] = useState(false)
  const [queue, setQueue] = useState<ExportQueueSnapshot | null>(null)
  const [exportProgress, setExportProgress] = useState(0)
  const [exportStage, setExportStage] = useState("")
  const [result, setResult] = useState<MergedExportResult | null>(null)
  const [clipsBusy, setClipsBusy] = useState(false)
  const [clipsResult, setClipsResult] = useState<ClipExportResult | null>(null)
  const [error, setError] = useState<string | null>(null)
  const [cancelOpen, setCancelOpen] = useState(false)
  const seenQueueJobs = useRef<Set<string>>(new Set())
  const queueSeeded = useRef(false)

  const queueActive = Boolean(queue?.active || queue?.global_busy)

  useEffect(() => {
    queueSeeded.current = false
    seenQueueJobs.current = new Set()
  }, [active?.id])

  // Full-cast still + hover-clip prefetch (GPU when available).
  useEffect(() => {
    const project = active
    if (!project?.id || project.status !== "ready") return
    const segs = project.segments ?? []
    if (!segs.length) return

    let cancelled = false
    const times = [
      ...new Set(segs.map((s) => Number(segmentStillTime(s).toFixed(3)))),
    ]
    const ranges = segs.map((s) => {
      const p = segmentPreviewRange(s)
      return [p.startSec, p.endSec] as [number, number]
    })
    const urls = times.map((t) => api.sceneFrameUrl(project.id, t))

    const warmBrowser = () => {
      for (const src of urls) {
        if (cancelled) return
        const img = new Image()
        img.decoding = "async"
        img.src = src
      }
    }

    void (async () => {
      try {
        await Promise.all([
          api.prefetchSceneFrames(project.id, times),
          api.prefetchHoverClips(project.id, ranges),
        ])
      } catch {
        /* tiles self-warm on view/hover */
      }
      if (cancelled) return
      warmBrowser()
      window.setTimeout(() => {
        if (!cancelled) warmBrowser()
      }, 4000)
    })()

    return () => {
      cancelled = true
    }
  }, [active?.id, active?.status, active?.segments?.length])

  useEffect(() => {
    if (!active?.id) return
    let cancelled = false
    const tick = async () => {
      try {
        const q = await api.exportQueue(active.id)
        if (cancelled) return
        setQueue(q)
        const cur = q.current
        if (cur) {
          setExportProgress(Math.round(Math.min(100, Math.max(0, (cur.progress || 0) * 100))))
          setExportStage(sanitizeStage(cur.stage) || "Exporting…")
        } else if (!q.pending_count) {
          setExportProgress(0)
          setExportStage("")
        }
        if (!queueSeeded.current) {
          for (const j of q.recent) seenQueueJobs.current.add(j.id)
          queueSeeded.current = true
        } else {
          for (const j of q.recent) {
            if (seenQueueJobs.current.has(j.id)) continue
            seenQueueJobs.current.add(j.id)
            if (j.status === "done") {
              setResult({
                output_path: j.output_path,
                scene_count: j.scene_count,
                person_label: j.person_label,
              })
              toast.success(`Exported ${j.person_label}`)
              void refreshActive()
            } else if (j.status === "failed") {
              toast.error(friendlyError(j.error, "Export failed"))
            }
          }
        }
      } catch {
        /* ignore */
      }
    }
    void tick()
    const id = window.setInterval(() => void tick(), queueActive ? 500 : 2000)
    return () => {
      cancelled = true
      window.clearInterval(id)
    }
  }, [active?.id, queueActive, refreshActive])

  useEffect(() => {
    let cancelled = false
    void (async () => {
      try {
        const caps = await api.exportCapabilities()
        if (cancelled) return
        setCodecCaps(caps.codecs ?? {})
        // Backend knows which hardware family this machine has (NVENC vs
        // VideoToolbox); without this the default stays NVENC-only on macOS.
        if (caps.default_codec) setVideoCodec(caps.default_codec as VideoCodec)
      } catch {
        /* optional */
      }
    })()
    return () => {
      cancelled = true
    }
  }, [])

  useEffect(() => {
    if (!active?.id) return
    let cancelled = false
    void (async () => {
      try {
        const st = await api.videoProxyStatus(active.id)
        if (cancelled) return
        const must =
          Boolean(st.needs_transcode) ||
          (!st.native_ok && !st.proxy_ready) ||
          (st.source_video_codec != null &&
            !["h264", "vp8", "vp9", "av1"].includes(st.source_video_codec))
        setForceReencode(must)
        if (must) {
          setExportMode((m) => (m === "copy" ? "quality" : m))
        }
      } catch {
        /* keep manual */
      }
    })()
    return () => {
      cancelled = true
    }
  }, [active?.id])

  // Prefer detail snapshot; fall back to list row so polls never flash LOADING.
  const exportProject =
    active ??
    (activeId ? (projects.find((p) => p.id === activeId) ?? null) : null)

  const person =
    selectedPersonId != null
      ? (exportProject?.people.find((p) => p.person_id === selectedPersonId) ?? null)
      : null

  /**
   * Manual ranges tagged to a person, shaped as segments.
   *
   * The server folds these into the pack in api._active_segments, so the
   * grid has to show them too or Export would under-report what it ships.
   */
  const manualAsSegments = useCallback(
    (personId: number): Segment[] =>
      (exportProject?.manual_segments ?? [])
        .filter((m) => m.person_ids.includes(personId))
        .map((m) => ({
          person_id: personId,
          start_sec: m.start_sec,
          end_sec: m.end_sec,
        })),
    [exportProject?.manual_segments],
  )

  const segments = useMemo(() => {
    if (!exportProject || !person) return []
    return [
      ...exportProject.segments.filter((s) => s.person_id === person.person_id),
      ...manualAsSegments(person.person_id),
    ].sort((a, b) => a.start_sec - b.start_sec)
  }, [exportProject, person, manualAsSegments])

  const scenes = useMemo(
    () =>
      exportProject?.scenes?.length
        ? exportProject.scenes
        : buildSharedScenesClient(exportProject?.segments ?? []),
    [exportProject?.scenes, exportProject?.segments],
  )

  const exclusions = exportProject?.excluded_segments ?? []

  const personExclusions = useMemo(
    () => exclusions.filter((ex) => ex.person_id === person?.person_id),
    [exclusions, person?.person_id],
  )

  const activeScenes = useMemo(
    () => segments.filter((s) => !isSegmentExcluded(exclusions, s)),
    [segments, exclusions],
  )

  const activeDuration = useMemo(
    () => activeScenes.reduce((sum, s) => sum + (s.end_sec - s.start_sec), 0),
    [activeScenes],
  )

  const castPacks = useMemo(() => {
    if (!exportProject?.people.length) return []
    const ex = exportProject.excluded_segments ?? []
    return exportProject.people
      .map((p) => {
        const segs = [
          ...exportProject.segments.filter((s) => s.person_id === p.person_id),
          ...manualAsSegments(p.person_id),
        ]
        const activeSegs = segs.filter((s) => !isSegmentExcluded(ex, s))
        const dur = activeSegs.reduce((sum, s) => sum + (s.end_sec - s.start_sec), 0)
        return { person: p, sceneCount: segs.length, activeDuration: dur }
      })
      .filter((row) => row.sceneCount > 0)
      .sort((a, b) => b.activeDuration - a.activeDuration || b.sceneCount - a.sceneCount)
  }, [exportProject, manualAsSegments])

  const finishedFiles = useMemo(() => {
    const fromProject = exportProject?.merged_exports ?? []
    const fromQueue: MergedExport[] = (queue?.recent ?? [])
      .filter((j) => j.status === "done")
      .map((j) => ({
        id: j.id,
        person_id: j.person_id,
        person_label: j.person_label,
        output_path: j.output_path,
        scene_count: j.scene_count,
        finished_at: "",
        export_mode: j.export_mode,
        video_codec: j.video_codec,
      }))
    const byPath = new Map<string, MergedExport>()
    for (const f of [...fromQueue, ...fromProject]) {
      if (!f.output_path) continue
      if (!byPath.has(f.output_path)) byPath.set(f.output_path, f)
    }
    return [...byPath.values()]
  }, [exportProject?.merged_exports, queue?.recent])

  const hasCast = Boolean(exportProject?.people?.length)
  const scanLive =
    exportProject?.status === "cutting" ||
    exportProject?.status === "scanning" ||
    exportProject?.status === "paused"
  const exportable =
    hasCast &&
    (exportProject?.status === "ready" || exportProject?.status === "exporting")

  // File ▸ Export / Cancel Export. These must be registered above the early
  // returns below, but the handlers are defined further down — the refs bridge
  // that gap and are assigned during the same render.
  const exportMergedRef = useRef<(() => void | Promise<void>) | null>(null)
  const cancelQueueRef = useRef<(() => void | Promise<void>) | null>(null)
  useMenuAction(
    "file:export",
    exportable ? () => exportMergedRef.current?.() : null,
  )
  useMenuAction(
    "file:cancel-export",
    exportable && queueActive ? () => cancelQueueRef.current?.() : null,
  )

  if (!projectsReady || (Boolean(activeId) && !exportProject)) {
    return (
      <div className="flex min-h-0 flex-1 flex-col items-center justify-center p-8">
        <div className="w-full max-w-sm">
          <LoadingBlock label="Loading export…" rows={4} dense />
        </div>
      </div>
    )
  }

  if (!exportProject) {
    return (
      <EmptyState
        title="No project selected"
        description="Open a scanned project from the dashboard, then come back to Export."
        actionLabel="Open dashboard"
        onAction={() => nav("/")}
        icon={<Clapperboard className="size-6" />}
      />
    )
  }

  if (scanLive) {
    return (
      <EmptyState
        title="Scan still running"
        description={
          hasCast
            ? `Cast is building (${exportProject.people.length} so far). Export unlocks when the scan finishes.`
            : "Finish the scan first — Export needs a completed cast."
        }
        actionLabel={hasCast ? "Open gallery" : "Open import"}
        onAction={() => nav(hasCast ? "/gallery" : "/import")}
        icon={<ScanFace className="size-6" />}
      />
    )
  }

  if (!exportable) {
    return (
      <EmptyState
        title="Nothing to export"
        description={
          exportProject.status === "error"
            ? "This scan failed. Re-scan from Import, then export packs."
            : "No cast on this project yet. Scan footage and build characters in the gallery."
        }
        actionLabel={exportProject.status === "error" ? "Open import" : "Open gallery"}
        onAction={() =>
          nav(exportProject.status === "error" ? "/import" : "/gallery")
        }
        icon={<ScanFace className="size-6" />}
      />
    )
  }

  if (!person) {
    return (
      <ExportTargetsPanel
        projectId={exportProject.id}
        packs={castPacks}
        files={finishedFiles}
        onSelectPack={(id) => {
          setResult(null)
          setClipsResult(null)
          setSelectedPersonId(id)
        }}
        onSelectFile={(file) => {
          setResult({
            output_path: file.output_path,
            scene_count: file.scene_count,
            person_label: file.person_label,
          })
          setSelectedPersonId(file.person_id)
        }}
      />
    )
  }

  const toggleSegment = async (segment: Segment, excluded: boolean) => {
    setToggleBusy(true)
    setError(null)
    try {
      await api.toggleSegmentExclusion(exportProject.id, {
        person_id: segment.person_id,
        start_sec: segment.start_sec,
        end_sec: segment.end_sec,
        excluded,
      })
      // Empty excluded_segments after re-include must win over poll merge.
      markAuthoritativeWrite()
      await refreshActive()
    } catch (e) {
      const msg = friendlyError(e)
      setError(msg)
      toast.error(msg)
    } finally {
      setToggleBusy(false)
    }
  }

  const restoreAll = async () => {
    if (!personExclusions.length) return
    setToggleBusy(true)
    setError(null)
    try {
      for (const ex of personExclusions) {
        await api.toggleSegmentExclusion(exportProject.id, {
          person_id: ex.person_id,
          start_sec: ex.start_sec,
          end_sec: ex.end_sec,
          excluded: false,
        })
      }
      markAuthoritativeWrite()
      await refreshActive()
      toast.success("All scenes restored")
    } catch (e) {
      const msg = friendlyError(e)
      setError(msg)
      toast.error(msg)
    } finally {
      setToggleBusy(false)
    }
  }

  const exportMerged = async () => {
    if (!activeScenes.length) {
      toast.error("Select at least one scene to export")
      return
    }

    const mode: ExportMode =
      forceReencode && exportMode === "copy" ? "quality" : exportMode
    // Mirror the backend's prefer_output_path: only NVENC HEVC/AV1 need mkv.
    // VideoToolbox HEVC stays mp4 (tagged hvc1 so QuickTime plays it).
    const defaultExt =
      videoCodec === "hevc_nvenc" || videoCodec === "av1_nvenc" ? "mkv" : "mp4"
    const defaultName = `${safeFilename(person.label)}_scenes.${defaultExt}`
    const outputPath = await pickMergedVideoPath(defaultName)
    if (!outputPath) return

    setEnqueueBusy(true)
    setError(null)
    try {
      const res = await api.exportMerged(exportProject.id, {
        person_id: person.person_id,
        output_path: outputPath,
        export_mode: mode,
        video_codec: videoCodec,
        reencode: mode !== "copy",
      })
      const q = res.queue ?? (await api.exportQueue(exportProject.id))
      setQueue(q)
      if (q.current || (q.pending_count ?? 0) > 0) {
        toast.success(
          q.current && q.current.person_id === person.person_id && q.pending_count === 0
            ? `Exporting ${res.person_label} (${res.scene_count} scenes)`
            : `Queued ${res.person_label} (${res.scene_count} scenes)`,
        )
      }
      void refreshActive()
    } catch (e) {
      const msg = friendlyError(e)
      setError(msg)
      toast.error(msg)
    } finally {
      setEnqueueBusy(false)
    }
  }

  /**
   * Per-clip export: one file per scene into a folder.
   *
   * Unlike /export/merged this endpoint runs synchronously server-side, so
   * there is no queue to poll — hold a busy state for the round trip.
   */
  const exportClips = async () => {
    if (!activeScenes.length) {
      toast.error("Select at least one scene to export")
      return
    }
    setClipsBusy(true)
    setError(null)
    setClipsResult(null)
    try {
      const res = await api.exportPerson(
        exportProject.id,
        person.person_id,
        forceReencode || exportMode !== "copy",
      )
      setClipsResult(res)
      toast.success(`Exported ${res.count} clip${res.count === 1 ? "" : "s"}`)
      void refreshActive()
    } catch (e) {
      const msg = friendlyError(e)
      setError(msg)
      toast.error(msg)
    } finally {
      setClipsBusy(false)
    }
  }

  const cancelQueue = async () => {
    setEnqueueBusy(true)
    try {
      const res = await api.exportQueueCancel(exportProject.id)
      setQueue(res.queue ?? (await api.exportQueue(exportProject.id)))
      setExportProgress(0)
      setExportStage("")
      toast.message("Export cancelled · queue cleared")
      void refreshActive()
      setCancelOpen(false)
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setEnqueueBusy(false)
    }
  }

  // Hand the freshly-closed-over handlers to the menu registrations above.
  exportMergedRef.current = exportMerged
  cancelQueueRef.current = cancelQueue

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <PageHeader
        eyebrow={exportProject.name}
        title="Export bay"
        description="Hover plays each character window. Export merges those same cuts. Toggle Off to drop a plate before merge."
        actions={
          <div className="flex items-center gap-2">
            <button
              type="button"
              onClick={() => {
                setSelectedPersonId(null)
                setResult(null)
                setClipsResult(null)
              }}
              className="inline-flex items-center gap-1.5 rounded-md border border-[rgba(242,237,230,0.10)] px-2.5 py-1.5 text-xs text-[#9A948A] transition hover:border-[#C4A35A]/40 hover:text-[#F2EDE6]"
            >
              <LayoutGrid className="size-3.5" />
              All exports
            </button>
            <Badge variant="secondary" className="font-mono tabular-nums">
              {activeScenes.length}/{segments.length} active
            </Badge>
          </div>
        }
      />

      <div className="cast-bay grid min-h-0 flex-1 bg-[#141210]/40 lg:grid-cols-[minmax(0,1fr)_minmax(280px,320px)]">
        <ScrollArea className="min-h-0">
          <div className="space-y-4 p-5 lg:p-6">
            <div>
              <p className="text-[10px] uppercase tracking-[0.2em] text-[#9A948A]">
                Cast rail
              </p>
              <div className="filmstrip-fade mt-2">
                <div className="filmstrip-scroll flex gap-1.5 pb-0.5">
                  {exportProject.people.map((p) => {
                    const activeChip = p.person_id === person.person_id
                    return (
                      <button
                        key={p.person_id}
                        type="button"
                        onClick={() => setSelectedPersonId(p.person_id)}
                        className={cn(
                          "inline-flex shrink-0 items-center gap-2 rounded-md border px-2 py-1.5 text-left text-sm transition",
                          activeChip
                            ? "border-[#C4A35A]/50 bg-[#C4A35A]/10 text-[#F2EDE6]"
                            : "border-[rgba(242,237,230,0.10)] bg-[#1C1A17] text-[#9A948A] hover:border-[#C4A35A]/30 hover:text-[#F2EDE6]",
                        )}
                      >
                        <FacePortrait
                          src={
                            p.thumbnail_path
                              ? api.mediaUrl(exportProject.id, p.thumbnail_path)
                              : null
                          }
                          size="sm"
                          fit="cover"
                          className="size-7 rounded-md border-border/40"
                        />
                        <span className="max-w-[8rem] truncate font-medium">{p.label}</span>
                      </button>
                    )
                  })}
                </div>
              </div>
            </div>

            <div>
              <p className="mb-2 text-[10px] uppercase tracking-[0.2em] text-[#9A948A]">
                Scene reel · character windows · scene-cut safe
              </p>
              <SceneClipGrid
                projectId={exportProject.id}
                segments={segments}
                allSegments={exportProject.segments}
                exclusions={exclusions}
                people={exportProject.people}
                scenes={scenes}
                disabled={toggleBusy}
                onToggle={toggleSegment}
              />
            </div>
          </div>
        </ScrollArea>

        <aside className="flex min-h-0 flex-col border-t border-[rgba(242,237,230,0.10)] bg-[#1C1A17]/90 lg:border-l lg:border-t-0">
          <ScrollArea className="min-h-0 flex-1">
            <div className="space-y-4 p-5 pb-3">
            <div className="flex items-start gap-3">
              <FacePortrait
                src={
                  person.thumbnail_path
                    ? api.mediaUrl(exportProject.id, person.thumbnail_path)
                    : null
                }
                alt={person.label}
                size="md"
                fit="contain"
              />
              <div className="min-w-0 pt-0.5">
                <p className="text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
                  Scene review
                </p>
                <h2 className="font-heading mt-1 truncate text-xl tracking-tight">
                  {person.label}
                </h2>
                <p className="mt-1 text-xs leading-relaxed text-muted-foreground">
                  Gray scenes stay off the merge. Click again to restore.
                </p>
              </div>
            </div>

            <dl className="grid grid-cols-2 gap-px overflow-hidden rounded-lg border border-border/40 bg-border/40">
              <Stat label="Total" value={String(segments.length)} />
              <Stat label="Active" value={String(activeScenes.length)} />
              <Stat label="Excluded" value={String(personExclusions.length)} />
              <Stat label="Runtime" value={formatDuration(activeDuration)} />
            </dl>

            {personExclusions.length > 0 ? (
              <button
                type="button"
                disabled={toggleBusy}
                onClick={() => void restoreAll()}
                className="inline-flex items-center gap-1.5 text-xs text-muted-foreground transition hover:text-foreground"
              >
                <RotateCcw className="size-3.5" />
                Restore all excluded scenes
              </button>
            ) : null}

            <div className="space-y-3 rounded-lg border border-border/50 px-4 py-3">
              <div>
                <p className="text-[10px] uppercase tracking-[0.16em] text-muted-foreground">
                  Encode rack
                </p>
                <p className="mt-1 text-sm font-medium tracking-tight">Delivery</p>
                <p className="mt-0.5 text-xs leading-relaxed text-muted-foreground">
                  {forceReencode
                    ? "Source needs re-encode. Quality = fast GPU. Archive = master look, size near source (GPU)."
                    : "Quality = fast GPU. Archive = keep source look without mush or giant files. Stream copy when H.264-safe."}
                </p>
              </div>

              <div className="space-y-1.5">
                <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                  Mode
                </Label>
                <Select
                  value={forceReencode && exportMode === "copy" ? "quality" : exportMode}
                  onValueChange={(v) => {
                    if (!v) return
                    if (forceReencode && v === "copy") return
                    setExportMode(v as ExportMode)
                  }}
                  disabled={enqueueBusy}
                >
                  <SelectTrigger className="h-9 bg-background/40">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem value="quality">Quality</SelectItem>
                    <SelectItem value="lossless">Archive</SelectItem>
                    <SelectItem value="copy" disabled={forceReencode}>
                      Stream copy
                    </SelectItem>
                  </SelectContent>
                </Select>
              </div>

              <div className="space-y-1.5">
                <Label className="text-[11px] uppercase tracking-[0.14em] text-muted-foreground">
                  Codec
                </Label>
                <Select
                  value={videoCodec}
                  onValueChange={(v) => v && setVideoCodec(v as VideoCodec)}
                  disabled={enqueueBusy || exportMode === "copy"}
                >
                  <SelectTrigger className="h-9 bg-background/40">
                    <SelectValue />
                  </SelectTrigger>
                  <SelectContent>
                    <SelectItem
                      value={h264Codec}
                      disabled={codecCaps[h264Codec] === false}
                    >
                      H.264
                    </SelectItem>
                    <SelectItem
                      value={hevcCodec}
                      disabled={codecCaps[hevcCodec] === false}
                    >
                      HEVC
                    </SelectItem>
                    <SelectItem
                      value="av1_nvenc"
                      disabled={codecCaps.av1_nvenc !== true}
                    >
                      AV1
                    </SelectItem>
                  </SelectContent>
                </Select>
                <p className="text-[11px] text-muted-foreground">
                  {exportMode === "lossless"
                    ? `Archive uses ${hwEncoderName} near source bitrate (keeps 10-bit HEVC when useful).`
                    : exportMode === "copy"
                      ? "Codec unused for stream copy."
                      : `${hwEncoderName} only. No CPU encode fallback.`}
                </p>
              </div>
            </div>

            {error ? (
              <div className="rounded-lg border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm">
                {error}
              </div>
            ) : null}

            {result ? <MergedExportResultPanel result={result} /> : null}
            {clipsResult ? <ExportResultPanel result={clipsResult} /> : null}
            </div>
          </ScrollArea>

          <div className="shrink-0 space-y-3 border-t border-border/40 bg-card/40 px-5 py-4">
            <AnimatedButton
              className="h-10 w-full gap-2"
              disabled={enqueueBusy || toggleBusy || clipsBusy || !activeScenes.length}
              onClick={() => void exportMerged()}
            >
              {enqueueBusy ? (
                <>
                  <Loader2 className="size-4 animate-spin" />
                  Adding…
                </>
              ) : queueActive ? (
                <>
                  <ListPlus className="size-4" />
                  Add to export queue
                </>
              ) : (
                <>
                  <Clapperboard className="size-4" />
                  Export merged video
                </>
              )}
            </AnimatedButton>

            <Button
              variant="secondary"
              className="h-9 w-full gap-2"
              disabled={enqueueBusy || toggleBusy || clipsBusy || !activeScenes.length}
              onClick={() => void exportClips()}
            >
              {clipsBusy ? (
                <>
                  <Loader2 className="size-4 animate-spin" />
                  Exporting {activeScenes.length} clip
                  {activeScenes.length === 1 ? "" : "s"}…
                </>
              ) : (
                <>
                  <Files className="size-4" />
                  Export separate clips
                </>
              )}
            </Button>

            {queueActive ? (
              <div className="space-y-2">
                <div className="flex items-center justify-between gap-2 text-xs tabular-nums text-muted-foreground">
                  <span className="min-w-0 truncate">{exportStage || "Exporting…"}</span>
                  <span className="shrink-0 font-medium text-foreground">{exportProgress}%</span>
                </div>
                <Progress value={exportProgress} className="h-1.5" />
                {queue?.pending_count ? (
                  <p className="text-[11px] text-muted-foreground">
                    Queue · {queue.pending_count} waiting
                    {queue.pending.slice(0, 3).map((j) => (
                      <span key={j.id}> · {j.person_label}</span>
                    ))}
                  </p>
                ) : (
                  <p className="text-[11px] text-muted-foreground">
                    Switch cast anytime · previews stay live
                  </p>
                )}
                <button
                  type="button"
                  disabled={enqueueBusy}
                  onClick={() => setCancelOpen(true)}
                  className="inline-flex items-center gap-1.5 text-xs text-muted-foreground transition hover:text-destructive"
                >
                  <Square className="size-3.5" />
                  Cancel export · clear queue
                </button>
              </div>
            ) : null}
          </div>
        </aside>
      </div>

      <AlertDialog open={cancelOpen} onOpenChange={setCancelOpen}>
        <AlertDialogContent size="default" className="max-w-md">
          <AlertDialogHeader>
            <AlertDialogTitle>Cancel export queue?</AlertDialogTitle>
            <AlertDialogDescription>
              Cancel all exports in the queue?
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel disabled={enqueueBusy}>Keep exporting</AlertDialogCancel>
            <AlertDialogAction
              variant="destructive"
              disabled={enqueueBusy}
              onClick={(e) => {
                e.preventDefault()
                void cancelQueue()
              }}
            >
              {enqueueBusy ? (
                <>
                  <Loader2 className="size-3.5 animate-spin" />
                  Cancelling…
                </>
              ) : (
                "Cancel queue"
              )}
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  )
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-card/80 px-3 py-2.5 text-center">
      <dd className="timecode text-sm font-medium tabular-nums text-primary">{value}</dd>
      <dt className="mt-0.5 text-[9px] uppercase tracking-[0.12em] text-muted-foreground">
        {label}
      </dt>
    </div>
  )
}
