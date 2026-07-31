import { useCallback, useEffect, useRef, useState } from "react"
import { useNavigate } from "react-router-dom"
import { Cpu, FolderOpen, Upload } from "lucide-react"
import { motion } from "motion/react"
import { toast } from "sonner"
import { Badge } from "@/components/ui/badge"
import { PageHeader } from "@/components/app/PageHeader"
import { ImportDropzone } from "@/components/import/ImportDropzone"
import {
  IngestTracker,
  type IngestFileMeta,
} from "@/components/import/IngestTracker"
import { ProjectLoader } from "@/components/import/ProjectLoader"
import { ScanProfileRack } from "@/components/import/ScanProfileRack"
import type { UploadProgressState } from "@/components/import/UploadProgressBar"
import {
  ANIMECV_MODEL_FALLBACK,
  api,
  normalizeSettings,
  type AnimeCVModelInfo,
  type Project,
} from "@/lib/api"
import { pickLocalVideoPath } from "@/lib/desktop"
import { mediaLabel } from "@/lib/format"
import { resolveDeepfaceModel } from "@/lib/modelFamilies"
import { useProjects } from "@/state/project"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { friendlyError, friendlyGpuLine, friendlyProjectStatus } from "@/lib/plainCopy"
import { cn } from "@/lib/utils"

export function ImportPage() {
  const nav = useNavigate()
  const {
    active,
    activeId,
    projects,
    projectsReady,
    hydrating,
    setActiveId,
    refreshProjects,
    refreshActive,
    gpu,
    importUi,
    setImportUi,
    resetImportUi,
    removeProjectLocal,
  } = useProjects()
  const { mode, tracking, ingestFile, backend, model } = importUi
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  const [upload, setUpload] = useState<UploadProgressState | null>(null)
  const uploadStartedAt = useRef<number | null>(null)
  const uploadEmaBps = useRef(0)
  const uploadLastSample = useRef<{ t: number; loaded: number } | null>(null)

  useEffect(() => {
    if (mode === "load") {
      void refreshProjects()
    }
  }, [mode, refreshProjects])

  const [animeModels, setAnimeModels] = useState<AnimeCVModelInfo[]>(
    ANIMECV_MODEL_FALLBACK,
  )

  useEffect(() => {
    void api.settings().then((raw) => {
      const s = normalizeSettings(raw)
      if (s.animecv_models?.length) setAnimeModels(s.animecv_models)
      // Never clobber an in-flight tracker restore — only seed rack defaults
      // when Import is idle.
      if (importUi.tracking) return
      // Production defaults follow content preset stack. DeepFace is lab-only
      // (user can still pick it on the rack); never seed as Import default.
      const b = (s.default_backend || "insightface").toLowerCase()
      if (b === "animecv" || b === "anime") {
        setImportUi({
          backend: "animecv",
          model: s.default_model || "cel_canon",
        })
      } else {
        setImportUi({
          backend: "insightface",
          model:
            b === "insightface" && s.default_model
              ? s.default_model
              : "buffalo_l",
        })
      }
    })
    // eslint-disable-next-line react-hooks/exhaustive-deps -- boot seed only
  }, [setImportUi])

  const pinIngestFile = useCallback(
    (meta: IngestFileMeta) => {
      setImportUi({
        tracking: true,
        ingestFile: meta,
        liveFaceProjectId: null,
        liveFaceRev: 0,
      })
      // Detach finished/idle project so tracker cannot show old cast as "Finished".
      if (
        active &&
        active.status !== "cutting" &&
        active.status !== "scanning" &&
        active.status !== "paused" &&
        active.status !== "exporting"
      ) {
        setActiveId(null)
      }
    },
    [active, setActiveId, setImportUi],
  )

  const startFromPath = useCallback(
    async (videoPath: string) => {
      const name = mediaLabel(videoPath)
      setBusy(true)
      setError(null)
      pinIngestFile({
        name,
        size: 0,
        type: "video",
        path: videoPath,
      })
      try {
        const project = await api.createPath({
          name,
          video_path: videoPath,
          backend,
          model: backend === "deepface" ? resolveDeepfaceModel(model) : model,
        })
        setImportUi({
          tracking: true,
          liveFaceProjectId: null,
          liveFaceRev: 0,
          ingestFile: {
            name,
            size: project.video_bytes ?? 0,
            type: "video",
            path: project.video_path || videoPath,
          },
        })
        setActiveId(project.id)
        await api.scan(project.id)
        await refreshProjects()
        await refreshActive()
        toast.success("Local path linked — scan running")
      } catch (e) {
        const msg = friendlyError(e)
        setError(msg)
        toast.error(msg)
        setImportUi({ tracking: false })
      } finally {
        setBusy(false)
      }
    },
    [
      backend,
      model,
      pinIngestFile,
      refreshActive,
      refreshProjects,
      setActiveId,
      setImportUi,
    ],
  )

  const onPickLocalPath = useCallback(async () => {
    const path = await pickLocalVideoPath()
    if (!path) return
    await startFromPath(path)
  }, [startFromPath])

  const onFile = useCallback(
    async (file: File | null) => {
      if (!file) return
      uploadStartedAt.current = Date.now()
      uploadEmaBps.current = 0
      uploadLastSample.current = { t: Date.now(), loaded: 0 }
      pinIngestFile({
        name: file.name,
        size: file.size,
        type: file.type || "video",
      })
      setUpload({
        file,
        loaded: 0,
        total: file.size,
        percent: 0,
        bytesPerSecond: 0,
        etaSeconds: file.size > 0 ? null : 0,
        phase: "transfer",
      })
      setBusy(true)
      setError(null)
      try {
        const project = await api.upload(file, {
          backend,
          model: backend === "deepface" ? resolveDeepfaceModel(model) : model,
          autostart: true,
          onProgress: (event) => {
            const now = Date.now()
            const last = uploadLastSample.current
            let ema = uploadEmaBps.current
            if (last && now > last.t) {
              const dt = (now - last.t) / 1000
              const dBytes = event.loaded - last.loaded
              if (dt > 0.05 && dBytes >= 0) {
                const inst = dBytes / dt
                ema = ema > 0 ? 0.25 * inst + 0.75 * ema : inst
                uploadEmaBps.current = ema
              }
            }
            uploadLastSample.current = { t: now, loaded: event.loaded }
            const remaining = Math.max(0, event.total - event.loaded)
            const etaSeconds = ema > 0 && remaining > 0 ? remaining / ema : null
            const percent = event.total > 0 ? (event.loaded / event.total) * 100 : 0
            setUpload({
              file,
              loaded: event.loaded,
              total: event.total,
              percent,
              bytesPerSecond: ema,
              etaSeconds,
              phase: percent >= 99.95 && remaining === 0 ? "finalizing" : "transfer",
            })
          },
        })
        setUpload((prev) =>
          prev
            ? {
                ...prev,
                loaded: prev.total,
                percent: 100,
                etaSeconds: 0,
                phase: "done",
              }
            : null,
        )
        setImportUi({
          tracking: true,
          liveFaceProjectId: null,
          liveFaceRev: 0,
          ingestFile: {
            name: file.name,
            size: file.size,
            type: file.type || "video",
            path: project.video_path,
          },
        })
        setActiveId(project.id)
        await refreshProjects()
        await refreshActive()
        toast.success("Upload complete — scan running")
        window.setTimeout(() => setUpload(null), 600)
      } catch (e) {
        const msg = friendlyError(e)
        setError(msg)
        toast.error(msg)
        setImportUi({ tracking: false })
        setUpload(null)
      } finally {
        setBusy(false)
        uploadStartedAt.current = null
        uploadLastSample.current = null
      }
    },
    [
      backend,
      model,
      pinIngestFile,
      refreshActive,
      refreshProjects,
      setActiveId,
      setImportUi,
    ],
  )

  const openProject = async (project: Project) => {
    setBusy(true)
    setError(null)
    try {
      setActiveId(project.id)
      setImportUi({
        tracking: project.status !== "ready",
        ingestFile: {
          name: mediaLabel(project.video_path, project.name),
          size: project.video_bytes ?? 0,
          type: "video",
          path: project.video_path,
        },
        mode: project.status === "ready" ? mode : "upload",
      })
      await refreshActive()
      await refreshProjects()
      const fresh = await api.project(project.id)
      if ((fresh.video_bytes ?? 0) > 0) {
        setImportUi((prev) => ({
          ...prev,
          ingestFile: prev.ingestFile
            ? { ...prev.ingestFile, size: fresh.video_bytes ?? prev.ingestFile.size }
            : prev.ingestFile,
        }))
      }

      if (project.status === "ready") {
        toast.success(`Loaded ${mediaLabel(project.video_path, project.name)}`)
        nav("/gallery")
        return
      }

      setImportUi({ mode: "upload", tracking: true })
      toast.message(
        project.status === "cutting" ||
          project.status === "scanning" ||
          project.status === "paused"
          ? "Scene cuts / scan in progress. Tracker open."
          : `Loaded · ${friendlyProjectStatus(project.status)}`,
      )
    } catch (e) {
      const msg = friendlyError(e)
      setError(msg)
      toast.error(msg)
    } finally {
      setBusy(false)
    }
  }

  const isBusyRow = (p: Project) =>
    p.status === "cutting" ||
    p.status === "scanning" ||
    p.status === "paused" ||
    p.status === "exporting"

  const busyProject =
    projects
      .filter(isBusyRow)
      .sort((a, b) => {
        const aTs = Date.parse(a.updated_at || "") || 0
        const bTs = Date.parse(b.updated_at || "") || 0
        return bTs - aTs
      })[0] ?? null

  // Prefer live job over finished active (Frankenstein: new filename + old cast).
  const trackerProject = busyProject ?? (upload ? null : active)

  const scanBusy =
    trackerProject?.status === "cutting" ||
    trackerProject?.status === "scanning" ||
    trackerProject?.status === "paused"

  const anyListBusy = projects.some(
    (p) => isBusyRow(p) || p.status === "error",
  )

  const listPinnedBusy = projects.some(
    (p) =>
      p.id === activeId &&
      (p.status === "cutting" ||
        p.status === "scanning" ||
        p.status === "paused" ||
        p.status === "error"),
  )

  const showTracker =
    tracking ||
    Boolean(upload) ||
    Boolean(trackerProject && (scanBusy || trackerProject.status === "error")) ||
    listPinnedBusy ||
    anyListBusy ||
    (hydrating && (Boolean(activeId) || tracking))

  const showReadyTracker =
    tracking &&
    !upload &&
    !busyProject &&
    trackerProject?.status === "ready"

  const monitorWall = showTracker || showReadyTracker

  const gpuLabel = gpu ? friendlyGpuLine(gpu.available, gpu.name, gpu.detail) : undefined

  const onBackend = (v: string) => {
    if (v === "deepface") {
      setImportUi({ backend: v, model: resolveDeepfaceModel(model) })
      return
    }
    if (v === "animecv") {
      setImportUi({ backend: v, model: "cel_canon" })
      return
    }
    setImportUi({ backend: "insightface", model: "buffalo_l" })
  }

  const modeGate = (
    <div className="inline-flex rounded-lg border border-border/35 bg-[oklch(0.09_0.014_280/0.85)] p-0.5">
      {(
        [
          { id: "upload" as const, label: "Upload", icon: Upload },
          { id: "load" as const, label: "Load", icon: FolderOpen },
        ] as const
      ).map(({ id, label, icon: Icon }) => (
        <button
          key={id}
          type="button"
          disabled={busy || Boolean(scanBusy)}
          onClick={() => setImportUi({ mode: id })}
          className={cn(
            "flex items-center gap-2 rounded-md px-3.5 py-2 text-xs font-medium transition-colors",
            mode === id
              ? "bg-primary/15 text-primary"
              : "text-muted-foreground hover:bg-muted/30 hover:text-foreground",
            (busy || scanBusy) && "opacity-50",
          )}
        >
          <Icon className="size-3.5" />
          {label}
        </button>
      ))}
    </div>
  )

  const tracker = (
    <IngestTracker
      project={trackerProject}
      file={ingestFile}
      upload={upload}
      onOpenGallery={() => nav("/gallery")}
      onRetry={async () => {
        const id = trackerProject?.id ?? active?.id
        if (!id) return
        await api.scan(id)
        await refreshActive()
        setImportUi({ tracking: true })
        toast.message("Rescan queued")
      }}
      onPauseScan={async () => {
        const id = trackerProject?.id ?? active?.id
        if (!id) return
        try {
          await api.pauseScan(id)
          await refreshActive()
          toast.message("Scan paused")
        } catch (e) {
          toast.error(friendlyError(e))
        }
      }}
      onResumeScan={async () => {
        const id = trackerProject?.id ?? active?.id
        if (!id) return
        try {
          await api.resumeScan(id)
          await refreshActive()
          toast.message("Scan resumed")
        } catch (e) {
          toast.error(friendlyError(e))
        }
      }}
      onStopScan={async () => {
        const id = trackerProject?.id ?? active?.id
        if (!id) return
        try {
          await api.stopScan(id)
          await refreshActive()
          toast.message("Stopping scan…")
        } catch (e) {
          toast.error(friendlyError(e))
        }
      }}
      onImportAnother={resetImportUi}
    />
  )

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-auto">
      <PageHeader
        eyebrow="Import"
        title="Import"
        description={
          monitorWall
            ? "Scan in progress. You can pause, resume, or stop anytime."
            : "Upload a video or load an existing project. Link local path for large files."
        }
        className="border-border/40"
        actions={
          <div className="flex flex-wrap items-center gap-2">
            {!monitorWall ? modeGate : null}
            {gpu?.available ? (
              <Badge variant="secondary" className="gap-1.5 border-primary/20">
                <Cpu className="size-3.5 text-primary" />
                Graphics ready
              </Badge>
            ) : null}
          </div>
        }
      />

      <div className="flex flex-1 flex-col gap-4 p-4 sm:p-6 lg:p-8 animate-fade-up">
        {monitorWall ? (
          <div className="mx-auto flex w-full max-w-[1520px] flex-col gap-3">
            {hydrating && !active ? (
              <div className="panel-surface p-6">
                <LoadingBlock
                  label="Restoring active scan…"
                  rows={4}
                  dense
                />
              </div>
            ) : (
              tracker
            )}
            {error ? (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive"
              >
                {error}
              </motion.div>
            ) : null}
          </div>
        ) : mode === "upload" ? (
          <div className="mx-auto grid w-full max-w-[1520px] gap-4 lg:grid-cols-[minmax(0,1fr)_minmax(270px,300px)] lg:items-start lg:gap-5">
            <div className="order-1 min-w-0 space-y-4">
              <ImportDropzone
                busy={busy}
                gpuLabel={gpuLabel}
                upload={upload}
                onFile={(f) => void onFile(f)}
                onPickLocalPath={() => void onPickLocalPath()}
              />
              {error ? (
                <motion.div
                  initial={{ opacity: 0 }}
                  animate={{ opacity: 1 }}
                  className="rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive"
                >
                  {error}
                </motion.div>
              ) : null}
            </div>
            <aside className="order-2 lg:sticky lg:top-4">
              <ScanProfileRack
                variant="full"
                backend={backend}
                model={model}
                animeModels={animeModels}
                gpuLabel={gpuLabel}
                disabled={busy || Boolean(scanBusy)}
                onBackend={onBackend}
                onModel={(m) => setImportUi({ model: m })}
              />
            </aside>
          </div>
        ) : (
          <div className="mx-auto w-full max-w-[1520px] space-y-4">
            <div className="flex flex-wrap items-center justify-between gap-3 lg:hidden">
              {modeGate}
            </div>
            <ProjectLoader
              projects={projects}
              loading={!projectsReady}
              activeId={activeId}
              disabled={busy}
              onOpen={(p) => void openProject(p)}
              onDeleted={async (id) => {
                removeProjectLocal(id)
                if (activeId === id) {
                  resetImportUi()
                  setUpload(null)
                  setError(null)
                }
                await refreshProjects()
              }}
            />
            {error ? (
              <motion.div
                initial={{ opacity: 0 }}
                animate={{ opacity: 1 }}
                className="rounded-xl border border-destructive/40 bg-destructive/10 px-4 py-3 text-sm text-destructive"
              >
                {error}
              </motion.div>
            ) : null}
          </div>
        )}
      </div>
    </div>
  )
}
