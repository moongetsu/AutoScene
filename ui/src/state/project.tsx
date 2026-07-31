import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from "react"
import { api, type GpuInfo, type Project } from "@/lib/api"
import type { IngestFileMeta } from "@/components/import/IngestTracker"

const ACTIVE_ID_KEY = "autoscene.activeId"
/** localStorage so Ctrl+R / shell reload keep Import tracker + ingest meta. */
const IMPORT_UI_KEY = "autoscene.importUi"

export type ImportMode = "upload" | "load"

export type ImportUiState = {
  mode: ImportMode
  tracking: boolean
  ingestFile: IngestFileMeta | null
  backend: string
  model: string
  /** Last live-face cache bust rev so Import remount can paint immediately */
  liveFaceRev: number
  liveFaceProjectId: string | null
}

const defaultImportUi: ImportUiState = {
  mode: "upload",
  tracking: false,
  ingestFile: null,
  backend: "insightface",
  model: "buffalo_l",
  liveFaceRev: 0,
  liveFaceProjectId: null,
}

const BUSY_STATUSES = new Set([
  "cutting",
  "scanning",
  "paused",
  "exporting",
])

const TRACKER_STATUSES = new Set([
  "cutting",
  "scanning",
  "paused",
  "error",
])

function isBusyStatus(status: string | undefined): boolean {
  return Boolean(status && BUSY_STATUSES.has(status))
}

function isTrackerStatus(status: string | undefined): boolean {
  return Boolean(status && TRACKER_STATUSES.has(status))
}

function readActiveId(): string | null {
  try {
    return localStorage.getItem(ACTIVE_ID_KEY)
  } catch {
    return null
  }
}

function writeActiveId(id: string | null) {
  try {
    if (id) localStorage.setItem(ACTIVE_ID_KEY, id)
    else localStorage.removeItem(ACTIVE_ID_KEY)
  } catch {
    /* ignore quota / private mode */
  }
}

function readImportUi(): ImportUiState {
  try {
    // Prefer localStorage; migrate leftover sessionStorage once.
    const raw =
      localStorage.getItem(IMPORT_UI_KEY) ?? sessionStorage.getItem(IMPORT_UI_KEY)
    if (!raw) return defaultImportUi
    const parsed = JSON.parse(raw) as Partial<ImportUiState>
    const next: ImportUiState = {
      mode: parsed.mode === "load" ? "load" : "upload",
      tracking: Boolean(parsed.tracking),
      ingestFile: parsed.ingestFile ?? null,
      backend: parsed.backend || defaultImportUi.backend,
      model: parsed.model || defaultImportUi.model,
      liveFaceRev: Number(parsed.liveFaceRev) || 0,
      liveFaceProjectId: parsed.liveFaceProjectId ?? null,
    }
    try {
      localStorage.setItem(IMPORT_UI_KEY, JSON.stringify(next))
      sessionStorage.removeItem(IMPORT_UI_KEY)
    } catch {
      /* ignore */
    }
    return next
  } catch {
    return defaultImportUi
  }
}

function writeImportUi(next: ImportUiState) {
  try {
    localStorage.setItem(IMPORT_UI_KEY, JSON.stringify(next))
  } catch {
    /* ignore */
  }
}

function ingestFromProject(p: Project, prev: IngestFileMeta | null): IngestFileMeta {
  if (prev) {
    return {
      ...prev,
      name: prev.name || p.name,
      size:
        prev.size > 0 ? prev.size : (p.video_bytes ?? 0),
      path: prev.path || p.video_path,
      type: prev.type || "video",
    }
  }
  return {
    name: p.name,
    size: p.video_bytes ?? 0,
    type: "video",
    path: p.video_path,
  }
}

function pathBasename(path: string): string {
  const norm = path.replace(/\\/g, "/").trim()
  const leaf = norm.split("/").pop() || ""
  return leaf.toLowerCase()
}

function stripExt(name: string): string {
  return name.replace(/\.[a-z0-9]{2,5}$/i, "").trim().toLowerCase()
}

/** True when pinned ingest file is clearly not this project (new upload mid-flight). */
function ingestMismatch(file: IngestFileMeta | null, p: Project | undefined): boolean {
  if (!file || !p) return false
  const fileName = (file.name || "").trim().toLowerCase()
  const fileStem = stripExt(fileName)
  const pStem = stripExt(p.name || "")
  const pPath = (p.video_path || "").replace(/\\/g, "/").toLowerCase()
  const filePath = (file.path || "").replace(/\\/g, "/").toLowerCase()
  const pLeaf = pathBasename(p.video_path || "")
  const pLeafStem = stripExt(pLeaf)

  // Same path / same file leaf → same project.
  if (filePath && pPath && filePath === pPath) return false
  if (fileName && (pLeaf === fileName || pLeafStem === fileStem)) return false
  if (
    fileStem &&
    pStem &&
    (fileStem === pStem || pStem.startsWith(fileStem) || fileStem.startsWith(pStem))
  ) {
    return false
  }
  if (filePath && pPath && filePath !== pPath) return true
  if (fileStem && pStem && fileStem !== pStem) return true
  return false
}

function pickBusyProject(list: Project[]): Project | undefined {
  const busy = list.filter((p) => isBusyStatus(p.status))
  if (busy.length === 0) return undefined
  if (busy.length === 1) return busy[0]
  return busy.slice().sort((a, b) => {
    const aTs = Date.parse(a.updated_at || "") || 0
    const bTs = Date.parse(b.updated_at || "") || 0
    return bTs - aTs
  })[0]
}

/** List payload is full project JSON — paint immediately on reload. */
function mergeProjectRow(prev: Project | null, row: Project): Project {
  if (!prev || prev.id !== row.id) return row
  // Prefer fresher updated_at / progress from either side.
  const prevTs = Date.parse(prev.updated_at || "") || 0
  const rowTs = Date.parse(row.updated_at || "") || 0
  const base =
    rowTs >= prevTs
      ? { ...prev, ...row }
      : {
          ...row,
          ...prev,
          // Keep live counters if prev is ahead mid-poll.
          progress: Math.max(prev.progress ?? 0, row.progress ?? 0),
          frames_done: Math.max(prev.frames_done ?? 0, row.frames_done ?? 0),
          detect_hits: Math.max(prev.detect_hits ?? 0, row.detect_hits ?? 0),
        }
  // Never let a thin list/detail poll wipe a richer cast already in memory.
  // Empty people/segments on Export was a common false "Nothing to export".
  if ((base.people?.length ?? 0) === 0 && (prev.people?.length ?? 0) > 0) {
    base.people = prev.people
  }
  if ((base.segments?.length ?? 0) === 0 && (prev.segments?.length ?? 0) > 0) {
    base.segments = prev.segments
  }
  if ((base.scenes?.length ?? 0) === 0 && (prev.scenes?.length ?? 0) > 0) {
    base.scenes = prev.scenes
  }
  if (
    (base.excluded_segments?.length ?? 0) === 0 &&
    (prev.excluded_segments?.length ?? 0) > 0
  ) {
    base.excluded_segments = prev.excluded_segments
  }
  return base
}

type Ctx = {
  projects: Project[]
  /** False until first /api/projects response (success or fail). */
  projectsReady: boolean
  /** True while bootstrapping known activeId before first paintable row. */
  hydrating: boolean
  activeId: string | null
  active: Project | null
  gpu: GpuInfo | null
  /** False until first /api/gpu response. */
  gpuReady: boolean
  selectedPersonId: number | null
  importUi: ImportUiState
  setActiveId: (id: string | null) => void
  setSelectedPersonId: (id: number | null) => void
  setImportUi: (
    patch: Partial<ImportUiState> | ((prev: ImportUiState) => ImportUiState),
  ) => void
  resetImportUi: () => void
  refreshProjects: () => Promise<void>
  refreshActive: () => Promise<void>
  refreshGpu: () => Promise<void>
  removeProjectLocal: (id: string) => void
}

const ProjectContext = createContext<Ctx | null>(null)

export function ProjectProvider({ children }: { children: React.ReactNode }) {
  const [projects, setProjects] = useState<Project[]>([])
  const [projectsReady, setProjectsReady] = useState(false)
  const [activeId, setActiveIdState] = useState<string | null>(() => readActiveId())
  const [active, setActive] = useState<Project | null>(null)
  const [gpu, setGpu] = useState<GpuInfo | null>(null)
  const [gpuReady, setGpuReady] = useState(false)
  const [selectedPersonId, setSelectedPersonId] = useState<number | null>(null)
  const [importUi, setImportUiState] = useState<ImportUiState>(() => readImportUi())

  const activeIdRef = useRef(activeId)
  activeIdRef.current = activeId
  const importUiRef = useRef(importUi)
  importUiRef.current = importUi
  const activeRef = useRef(active)
  activeRef.current = active
  const projectsRef = useRef(projects)
  projectsRef.current = projects

  const setActiveId = useCallback((id: string | null) => {
    if (activeIdRef.current !== id) {
      setSelectedPersonId(null)
      // Drop live-face pin from previous project so Import never shows stale cast.
      setImportUiState((prev) => {
        if (
          !prev.liveFaceProjectId ||
          (id != null && prev.liveFaceProjectId === id)
        ) {
          return prev
        }
        const next = { ...prev, liveFaceProjectId: null, liveFaceRev: 0 }
        importUiRef.current = next
        writeImportUi(next)
        return next
      })
    }
    activeIdRef.current = id
    setActiveIdState(id)
    writeActiveId(id)
  }, [])

  const setImportUi = useCallback(
    (
      patch: Partial<ImportUiState> | ((prev: ImportUiState) => ImportUiState),
    ) => {
      setImportUiState((prev) => {
        const next =
          typeof patch === "function" ? patch(prev) : { ...prev, ...patch }
        importUiRef.current = next
        writeImportUi(next)
        return next
      })
    },
    [],
  )

  const resetImportUi = useCallback(() => {
    const prev = importUiRef.current
    const next: ImportUiState = {
      ...defaultImportUi,
      backend: prev.backend,
      model: prev.model,
    }
    importUiRef.current = next
    setImportUiState(next)
    writeImportUi(next)
  }, [])

  const pinTrackerFromProject = useCallback(
    (p: Project) => {
      if (!isTrackerStatus(p.status)) return
      const hits = p.detect_hits ?? 0
      setImportUi((prev) => {
        // Never glue a finished project's name onto a newer ingest pin.
        const keepIngest =
          prev.ingestFile && ingestMismatch(prev.ingestFile, p)
            ? prev.ingestFile
            : ingestFromProject(p, prev.ingestFile)
        return {
          ...prev,
          tracking: true,
          ingestFile: keepIngest,
          liveFaceProjectId: hits > 0 ? p.id : null,
          liveFaceRev: hits > 0 ? hits : 0,
        }
      })
    },
    [setImportUi],
  )

  const refreshProjects = useCallback(async () => {
    try {
      const list = await api.projects()
      setProjects(list)

      const cur = activeIdRef.current
      const inList = cur ? list.find((p) => p.id === cur) : undefined
      const busy = pickBusyProject(list)
      const ui = importUiRef.current

      // Live cut/scan/export always wins — finished activeId must not hide it.
      if (busy) {
        if (cur !== busy.id) setActiveId(busy.id)
        setActive((prev) =>
          mergeProjectRow(prev?.id === busy.id ? prev : null, busy),
        )
        pinTrackerFromProject(busy)
        return
      }

      if (inList) {
        setActive((prev) => mergeProjectRow(prev, inList))
        // Finished/error: drop Import tracker pin so Export/poll never fight it.
        if (inList.status === "ready" || inList.status === "error") {
          if (ui.tracking) {
            setImportUi((prev) =>
              prev.tracking
                ? { ...prev, tracking: false, liveFaceProjectId: null, liveFaceRev: 0 }
                : prev,
            )
          }
        } else {
          pinTrackerFromProject(inList)
        }
        return
      }

      // Confirmed success + id not on server → clear. Never clear on empty
      // when we still have an id and list is empty only if truly no projects.
      if (cur) {
        setActive(null)
        setActiveId(null)
        setImportUi((prev) =>
          prev.tracking ? { ...prev, tracking: false } : prev,
        )
      }
    } catch {
      // Transient API failure: keep activeId / active / tracking untouched.
    } finally {
      setProjectsReady(true)
    }
  }, [pinTrackerFromProject, setActiveId, setImportUi])

  const refreshActive = useCallback(async () => {
    // Prefer list-busy over stale activeId before hitting detail.
    const listedBusy = pickBusyProject(projectsRef.current)
    if (listedBusy && activeIdRef.current !== listedBusy.id) {
      setActiveId(listedBusy.id)
      setActive((prev) =>
        mergeProjectRow(prev?.id === listedBusy.id ? prev : null, listedBusy),
      )
      pinTrackerFromProject(listedBusy)
    }

    const id = activeIdRef.current
    if (!id) {
      setActive(null)
      return
    }
    try {
      const p = await api.project(id)
      if (activeIdRef.current !== id) return
      setActive((prev) => mergeProjectRow(prev, p))
      setProjects((prev) => {
        const idx = prev.findIndex((row) => row.id === p.id)
        if (idx < 0) return prev
        const next = prev.slice()
        next[idx] = mergeProjectRow(prev[idx], p)
        return next
      })
      if (p.status === "ready" || p.status === "error") {
        const ui = importUiRef.current
        if (ui.tracking) {
          setImportUi((prev) =>
            prev.tracking
              ? { ...prev, tracking: false, liveFaceProjectId: null, liveFaceRev: 0 }
              : prev,
          )
        }
        return
      }
      // Don't re-pin tracker from idle detail while ingest tracks another file.
      const ui = importUiRef.current
      if (
        ui.tracking &&
        ui.ingestFile &&
        !isBusyStatus(p.status) &&
        ingestMismatch(ui.ingestFile, p)
      ) {
        return
      }
      pinTrackerFromProject(p)
    } catch (e) {
      const msg = e instanceof Error ? e.message : String(e)
      const gone =
        /\bHTTP\s*404\b/i.test(msg) ||
        /\b404\b/i.test(msg) ||
        /not found/i.test(msg) ||
        /project not found/i.test(msg)
      if (gone && activeIdRef.current === id) {
        // Avoid wipe if list still shows the row (transient detail 404).
        if (projectsRef.current.some((row) => row.id === id)) return
        setActive(null)
        setActiveId(null)
        setImportUi((prev) =>
          prev.tracking ? { ...prev, tracking: false } : prev,
        )
        return
      }
      // Transient blip: keep last active snapshot.
    }
  }, [pinTrackerFromProject, setActiveId, setImportUi])

  const refreshGpu = useCallback(async () => {
    try {
      setGpu(await api.gpu())
    } catch {
      // Offline / boot race — keep last known GPU info.
    } finally {
      setGpuReady(true)
    }
  }, [])

  const removeProjectLocal = useCallback(
    (id: string) => {
      setProjects((prev) => prev.filter((p) => p.id !== id))
      if (activeIdRef.current === id) {
        setActive(null)
        setActiveId(null)
      }
    },
    [setActiveId],
  )

  // Boot: wait for API gate, then token + list + detail.
  useEffect(() => {
    let cancelled = false
    ;(async () => {
      const {
        ensureApiToken,
        waitForApiReady,
        markApiReady,
        seedApiToken,
      } = await import("@/lib/api")
      const { apiBase } = await import("@/lib/desktop")
      let ready = await waitForApiReady(8_000)
      // HMR / remount can clear the in-memory ready flag after splash already ran.
      if (!ready) {
        try {
          const res = await fetch(`${apiBase()}/api/health`, { cache: "no-store" })
          if (res.ok) {
            const body = (await res.json()) as {
              ready?: boolean
              token?: string
            }
            if (body.ready !== false) {
              if (typeof body.token === "string" && body.token) {
                seedApiToken(body.token)
              }
              markApiReady()
              ready = true
            }
          }
        } catch {
          /* still try list below */
        }
      }
      if (cancelled) return
      if (ready) await ensureApiToken()
      if (cancelled) return
      await refreshProjects()
      if (cancelled) return
      await refreshActive()
      if (cancelled) return
      await refreshGpu()
    })()
    return () => {
      cancelled = true
    }
    // intentionally once on mount
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  // When activeId changes after boot (user open / pin), fetch detail.
  const bootDoneRef = useRef(false)
  useEffect(() => {
    if (!projectsReady) return
    if (!bootDoneRef.current) {
      bootDoneRef.current = true
      return
    }
    void refreshActive()
  }, [activeId, projectsReady, refreshActive])

  const listBusy = projects.some((p) => isBusyStatus(p.status))
  const activeBusy = isBusyStatus(active?.status)
  const ghostTracker = importUi.tracking && !activeId

  // Poll while busy; soft-poll when idle so a lost pin rediscovers live scans.
  useEffect(() => {
    if (!projectsReady) return
    let cancelled = false
    const tick = async () => {
      await refreshProjects()
      if (cancelled) return
      await refreshActive()
    }
    void tick()
    const ms = listBusy || activeBusy || ghostTracker ? 1200 : 4500
    const t = window.setInterval(() => {
      void tick()
    }, ms)
    return () => {
      cancelled = true
      window.clearInterval(t)
    }
  }, [projectsReady, listBusy, activeBusy, ghostTracker, refreshProjects, refreshActive])

  useEffect(() => {
    if (active?.status === "ready") {
      void refreshProjects()
    }
  }, [active?.status, refreshProjects])

  const hydrating =
    !projectsReady ||
    (Boolean(activeId) && !active && projectsReady && !projects.some((p) => p.id === activeId))

  const value = useMemo(
    () => ({
      projects,
      projectsReady,
      hydrating,
      activeId,
      active,
      gpu,
      gpuReady,
      selectedPersonId,
      importUi,
      setActiveId,
      setSelectedPersonId,
      setImportUi,
      resetImportUi,
      refreshProjects,
      refreshActive,
      refreshGpu,
      removeProjectLocal,
    }),
    [
      projects,
      projectsReady,
      hydrating,
      activeId,
      active,
      gpu,
      gpuReady,
      selectedPersonId,
      importUi,
      setActiveId,
      setSelectedPersonId,
      setImportUi,
      resetImportUi,
      refreshProjects,
      refreshActive,
      refreshGpu,
      removeProjectLocal,
    ],
  )

  return <ProjectContext.Provider value={value}>{children}</ProjectContext.Provider>
}

export function useProjects() {
  const ctx = useContext(ProjectContext)
  if (!ctx) throw new Error("useProjects outside provider")
  return ctx
}

export function projectIsBusy(status: string | undefined): boolean {
  return isBusyStatus(status)
}

export function projectShowsTracker(status: string | undefined): boolean {
  return isTrackerStatus(status)
}
