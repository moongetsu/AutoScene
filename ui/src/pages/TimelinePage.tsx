import { useCallback, useEffect, useMemo, useRef, useState } from "react"
import { useNavigate } from "react-router-dom"
import { Clapperboard, Layers, Scissors, Trash2, User } from "lucide-react"
import { toast } from "sonner"
import { Button } from "@/components/ui/button"
import { Input } from "@/components/ui/input"
import { ScrollArea } from "@/components/ui/scroll-area"
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar"
import { PageHeader } from "@/components/app/PageHeader"
import { EmptyState } from "@/components/app/EmptyState"
import { LoadingBlock } from "@/components/shared/LoadingBlock"
import { PersonDossier } from "@/components/gallery/PersonDossier"
import {
  TimelineCanvas,
  type TimelineBlock,
  type TimelineLane,
  type TimelineSelection,
  type TimelineView,
} from "@/components/timeline/TimelineCanvas"
import { VideoPlayer, type PlayerSegment } from "@/components/player/VideoPlayer"
import { useVideoFps } from "@/components/player/useVideoFps"
import { CopyButton } from "@/components/shared/CopyButton"
import {
  api,
  formatDuration,
  isSegmentExcluded,
  type ManualSegment,
  type PackClips,
  type PackContextMode,
  type Person,
  type Segment,
} from "@/lib/api"
import { PackContextBar } from "@/components/timeline/PackContextBar"
import {
  clipName,
  copyText,
  formatSegmentsForCopy,
  type CopyFormat,
} from "@/lib/clipboard"
import { formatTimecode, supportsDropFrame } from "@/lib/timecode"
import { buildSharedScenesClient, segmentFaceRange } from "@/lib/scenes"
import { friendlyError } from "@/lib/plainCopy"
import { useProjects, markAuthoritativeWrite } from "@/state/project"
import { useMenuAction } from "@/state/menu-actions"
import { cn } from "@/lib/utils"

type ViewMode = "person" | "full"

const COPY_PREFS_KEY = "autoscene.timelineCopy"

type CopyPrefs = { format: CopyFormat; prefix: string; dropFrame: boolean }

function loadCopyPrefs(): CopyPrefs {
  try {
    const raw = localStorage.getItem(COPY_PREFS_KEY)
    if (raw) {
      const p = JSON.parse(raw) as Partial<CopyPrefs>
      return {
        format: p.format ?? "timecode",
        prefix: p.prefix ?? "",
        dropFrame: Boolean(p.dropFrame),
      }
    }
  } catch {
    /* ignore malformed prefs */
  }
  return { format: "timecode", prefix: "", dropFrame: false }
}

export function TimelinePage() {
  const nav = useNavigate()
  const {
    active,
    activeId,
    projectsReady,
    selectedPersonId,
    setSelectedPersonId,
    refreshActive,
  } = useProjects()

  const [activeSegment, setActiveSegment] = useState<number | null>(0)
  const [viewMode, setViewMode] = useState<ViewMode>("person")
  const [view, setView] = useState<TimelineView>({ startSec: 0, endSec: 1 })
  const [playheadSec, setPlayheadSec] = useState(0)
  const [loopSegment, setLoopSegment] = useState(false)
  const [selection, setSelection] = useState<TimelineSelection | null>(null)
  const [selectionTags, setSelectionTags] = useState<number[]>([])
  const [savingRange, setSavingRange] = useState(false)
  const [copyPrefs, setCopyPrefs] = useState<CopyPrefs>(loadCopyPrefs)
  const [followPlayhead, setFollowPlayhead] = useState(true)

  const { fps } = useVideoFps(active?.id)
  const duration = active?.duration_sec ?? 0

  const person =
    active?.people.find((p) => p.person_id === selectedPersonId) ??
    active?.people[0] ??
    null

  const exclusions = useMemo(() => active?.excluded_segments ?? [], [active])
  const manualSegments = useMemo<ManualSegment[]>(
    () => active?.manual_segments ?? [],
    [active],
  )

  // --- pack context ---------------------------------------------------------
  // The pack is assembled server-side (api._active_segments), so the timeline
  // shows exactly what export will ship rather than re-deriving it here.
  const [pack, setPack] = useState<PackClips | null>(null)
  const [packBusy, setPackBusy] = useState(false)
  const projectId = active?.id ?? null
  const personId = person?.person_id ?? null
  const shots = useMemo(() => active?.shots ?? [], [active])
  const sceneGroups = useMemo(() => active?.scene_groups ?? [], [active])

  const loadPack = useCallback(async () => {
    if (!projectId || personId == null) {
      setPack(null)
      return
    }
    try {
      setPack(await api.packClips(projectId, personId))
    } catch (e) {
      toast.error(friendlyError(e))
    }
  }, [projectId, personId])

  useEffect(() => {
    void loadPack()
  }, [loadPack])

  /** Run a pack mutation, keeping the returned clips as the source of truth. */
  const mutatePack = useCallback(
    async (run: () => Promise<{ clips: PackClips }>, done?: string) => {
      setPackBusy(true)
      try {
        const res = await run()
        markAuthoritativeWrite()
        setPack(res.clips)
        if (done) toast.success(done)
        await refreshActive()
      } catch (e) {
        toast.error(friendlyError(e))
      } finally {
        setPackBusy(false)
      }
    },
    [refreshActive],
  )

  const setContextMode = useCallback(
    (mode: PackContextMode) => {
      if (!projectId || personId == null) return
      void mutatePack(() =>
        api.updatePackSettings(projectId, personId, { context_mode: mode }),
      )
    },
    [projectId, personId, mutatePack],
  )

  const setPad = useCallback(
    (patch: { lead_shots?: number; tail_shots?: number }) => {
      if (!projectId || personId == null) return
      void mutatePack(() => api.updatePackSettings(projectId, personId, patch))
    },
    [projectId, personId, mutatePack],
  )

  const setShotOverride = useCallback(
    (shotIndex: number, action: "include" | "exclude" | "clear") => {
      if (!projectId || personId == null) return
      const verb =
        action === "include"
          ? "Shot added to pack"
          : action === "exclude"
            ? "Shot removed from pack"
            : "Shot reset"
      void mutatePack(
        () => api.setClipOverride(projectId, personId, { shot_index: shotIndex, action }),
        verb,
      )
    },
    [projectId, personId, mutatePack],
  )

  const rebuildScenes = useCallback(async () => {
    if (!projectId) return
    setPackBusy(true)
    try {
      await api.rebuildSceneGroups(projectId)
      markAuthoritativeWrite()
      await refreshActive()
      await loadPack()
      toast.success("Scenes regrouped")
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setPackBusy(false)
    }
  }, [projectId, refreshActive, loadPack])

  /** Cut boundaries, for snapping drawn ranges and drawing the Shots lane. */
  const snapPoints = useMemo(() => {
    const pts = new Set<number>()
    for (const s of shots) {
      pts.add(s.start_sec)
      pts.add(s.end_sec)
    }
    return [...pts].sort((a, b) => a - b)
  }, [shots])

  /** Which shot each timestamp falls in — used to map clicks back to a shot. */
  const shotIndexAt = useCallback(
    (sec: number) => shots.findIndex((s) => s.start_sec <= sec && sec < s.end_sec),
    [shots],
  )

  const segments = useMemo(() => {
    if (!active || !person) return []
    return active.segments
      .filter((s) => s.person_id === person.person_id)
      .sort((a, b) => a.start_sec - b.start_sec)
  }, [active, person])

  /**
   * Exactly what this pack exports.
   *
   * Served by /packs/{id}/clips, which routes through the same helper as
   * /export — context widening, manual ranges and per-shot overrides are all
   * already applied, so the timeline cannot drift from the delivered files.
   * Falls back to raw plates only while the first fetch is in flight.
   */
  const personClips = useMemo(() => {
    if (pack) {
      const manualKeys = new Set(
        manualSegments
          .filter((m) => person && m.person_ids.includes(person.person_id))
          .map((m) => `${m.start_sec.toFixed(3)}:${m.end_sec.toFixed(3)}`),
      )
      return pack.clips.map((c) => {
        const key = `${c.start_sec.toFixed(3)}:${c.end_sec.toFixed(3)}`
        const manual = manualKeys.has(key)
        return {
          start_sec: c.start_sec,
          end_sec: c.end_sec,
          manual,
          id: manual
            ? (manualSegments.find(
                (m) =>
                  `${m.start_sec.toFixed(3)}:${m.end_sec.toFixed(3)}` === key,
              )?.id ?? null)
            : null,
        }
      })
    }
    return segments
      .map((s) => ({
        start_sec: s.start_sec,
        end_sec: s.end_sec,
        manual: false as const,
        id: null,
      }))
      .sort((a, b) => a.start_sec - b.start_sec)
  }, [pack, segments, manualSegments, person])

  /** What Copy Timecodes and Export actually ship — exclusions applied. */
  const includedClips = useMemo(
    () =>
      personClips.filter(
        (c) =>
          !person ||
          !isSegmentExcluded(exclusions, {
            person_id: person.person_id,
            start_sec: c.start_sec,
            end_sec: c.end_sec,
          } as Segment),
      ),
    [personClips, exclusions, person],
  )
  const excludedCount = personClips.length - includedClips.length

  // --- fit the view when the project changes -------------------------------
  useEffect(() => {
    if (duration > 0) setView({ startSec: 0, endSec: duration })
  }, [active?.id, duration])

  // Follow the playhead during playback, but never fight an active pan.
  useEffect(() => {
    if (!followPlayhead) return
    const span = view.endSec - view.startSec
    if (span >= duration) return
    if (playheadSec < view.startSec || playheadSec > view.endSec) {
      const start = Math.max(0, Math.min(playheadSec - span / 2, duration - span))
      setView({ startSec: start, endSec: start + span })
    }
  }, [playheadSec, followPlayhead, view.startSec, view.endSec, duration])

  // --- seeking -------------------------------------------------------------
  const [seekRequest, setSeekRequest] = useState<{ sec: number; nonce: number } | null>(
    null,
  )
  const seekNonce = useRef(0)
  const requestSeek = useCallback((sec: number) => {
    seekNonce.current += 1
    setSeekRequest({ sec, nonce: seekNonce.current })
  }, [])

  const playerSegments = useMemo<PlayerSegment[]>(
    () =>
      personClips.map((c, i) => ({
        key: `${i}`,
        startSec: c.start_sec,
        endSec: c.end_sec,
        excluded:
          person != null &&
          isSegmentExcluded(exclusions, {
            person_id: person.person_id,
            start_sec: c.start_sec,
            end_sec: c.end_sec,
          } as Segment),
        tone: c.manual ? ("manual" as const) : ("default" as const),
      })),
    [personClips, exclusions, person],
  )

  // --- lanes ---------------------------------------------------------------
  const scenes = useMemo(
    () =>
      active?.scenes?.length
        ? active.scenes
        : buildSharedScenesClient(active?.segments ?? []),
    [active?.scenes, active?.segments],
  )

  const isExcluded = useCallback(
    (personId: number, s: { start_sec: number; end_sec: number }) =>
      isSegmentExcluded(exclusions, {
        person_id: personId,
        start_sec: s.start_sec,
        end_sec: s.end_sec,
      } as Segment),
    [exclusions],
  )

  const manualLane = useCallback(
    (people: Person[]): TimelineLane => ({
      id: "manual",
      label: "Manual",
      blocks: manualSegments.map((m) => ({
        key: `manual:${m.id}`,
        startSec: m.start_sec,
        endSec: m.end_sec,
        manual: true,
        personIds: m.person_ids,
        title: `${m.label || "Manual range"} · ${
          m.person_ids
            .map((id) => people.find((p) => p.person_id === id)?.label ?? `#${id}`)
            .join(", ") || "untagged"
        }`,
      })),
    }),
    [manualSegments],
  )

  /** Cut ruler: every shot, so the user can see the structure they're editing. */
  const shotsLane = useMemo<TimelineLane | null>(() => {
    if (!shots.length) return null
    return {
      id: "shots",
      label: `Shots · ${shots.length}`,
      muted: true,
      blocks: shots.map((s, i) => ({
        key: `shot:${i}`,
        startSec: s.start_sec,
        endSec: s.end_sec,
        title: `Shot ${i + 1} · ${formatDuration(s.end_sec - s.start_sec)}`,
      })),
    }
  }, [shots])

  const sceneGroupLane = useMemo<TimelineLane | null>(() => {
    if (!sceneGroups.length) return null
    return {
      id: "scene-groups",
      label: `Scenes · ${sceneGroups.length}`,
      muted: true,
      blocks: sceneGroups.map((g) => ({
        key: `sg:${g.scene_index}`,
        startSec: g.start_sec,
        endSec: g.end_sec,
        title: `Scene ${g.scene_index + 1} · ${g.shot_indices.length} shot${
          g.shot_indices.length === 1 ? "" : "s"
        }`,
      })),
    }
  }, [sceneGroups])

  /**
   * Shots this character was detected in that the pack is not shipping —
   * rendered hollow, one click away from being pulled in.
   */
  const ghostBlocks = useMemo(() => {
    if (!pack?.missing_shot_indices.length) return []
    return pack.missing_shot_indices
      .filter((i) => i >= 0 && i < shots.length)
      .map((i) => ({
        key: `ghost:${i}`,
        startSec: shots[i].start_sec,
        endSec: shots[i].end_sec,
        ghost: true as const,
        title: `Shot ${i + 1} · detected here but not in the pack — click to add`,
      }))
  }, [pack, shots])

  const lanes = useMemo<TimelineLane[]>(() => {
    if (!active) return []
    const people = active.people

    if (viewMode === "person") {
      if (!person) return [manualLane(people)]
      return [
        ...(sceneGroupLane ? [sceneGroupLane] : []),
        ...(shotsLane ? [shotsLane] : []),
        {
          id: `person:${person.person_id}`,
          label: person.label,
          personId: person.person_id,
          thumbnailUrl: person.thumbnail_path
            ? api.mediaUrl(active.id, person.thumbnail_path)
            : null,
          blocks: [
            ...personClips.map((c, i) => ({
              key: `${i}`,
              startSec: c.start_sec,
              endSec: c.end_sec,
              manual: c.manual,
              excluded: isExcluded(person.person_id, {
                start_sec: c.start_sec,
                end_sec: c.end_sec,
              }),
              title: `Clip ${String(i + 1).padStart(2, "0")}`,
            })),
            ...ghostBlocks,
          ],
        },
        manualLane(people),
      ]
    }

    // Full-video mode: a scenes strip plus one lane per cast member.
    const scenesLane: TimelineLane = {
      id: "scenes",
      label: "Scenes",
      muted: true,
      blocks: scenes.map((s) => ({
        key: `scene:${s.scene_id}`,
        startSec: s.start_sec,
        endSec: s.end_sec,
        personIds: s.person_ids,
        title:
          s.person_ids.length > 1
            ? `Shared · ${s.person_ids
                .map((id) => people.find((p) => p.person_id === id)?.label ?? `#${id}`)
                .join(", ")}`
            : "Single",
      })),
    }

    const byPerson = new Map<number, Segment[]>()
    for (const s of active.segments) {
      const list = byPerson.get(s.person_id) ?? []
      list.push(s)
      byPerson.set(s.person_id, list)
    }

    const personLanes: TimelineLane[] = [...people]
      .sort((a, b) => b.screen_time_sec - a.screen_time_sec)
      .filter((p) => (byPerson.get(p.person_id)?.length ?? 0) > 0)
      .map((p) => ({
        id: `person:${p.person_id}`,
        label: p.label,
        personId: p.person_id,
        thumbnailUrl: p.thumbnail_path
          ? api.mediaUrl(active.id, p.thumbnail_path)
          : null,
        blocks: (byPerson.get(p.person_id) ?? []).map((s, i) => ({
          key: `p${p.person_id}:${i}`,
          startSec: s.start_sec,
          endSec: s.end_sec,
          excluded: isExcluded(p.person_id, s),
          title: `${p.label} · clip ${String(i + 1).padStart(2, "0")}`,
        })),
      }))

    return [
      ...(shotsLane ? [shotsLane] : []),
      scenesLane,
      ...personLanes,
      manualLane(people),
    ]
  }, [
    active,
    viewMode,
    person,
    personClips,
    scenes,
    isExcluded,
    manualLane,
    shotsLane,
    sceneGroupLane,
    ghostBlocks,
  ])

  // --- manual range selection ---------------------------------------------
  /**
   * Cast suggestions for a drawn range: anyone whose face window actually
   * overlaps it — the same rule Gallery's "shares the frame" badge uses.
   */
  const suggestTags = useCallback(
    (sel: TimelineSelection): number[] => {
      if (!active) return []
      const ids = new Set<number>()
      for (const s of active.segments) {
        const r = segmentFaceRange(s)
        const overlap =
          Math.min(r.endSec, sel.endSec) - Math.max(r.startSec, sel.startSec)
        if (overlap >= 0.2) ids.add(s.person_id)
      }
      return [...ids].sort((a, b) => a - b)
    },
    [active],
  )

  const onSelectionCommit = useCallback(
    (sel: TimelineSelection) => {
      setSelection(sel)
      setSelectionTags(suggestTags(sel))
    },
    [suggestTags],
  )

  const saveManualRange = async () => {
    if (!active || !selection) return
    setSavingRange(true)
    try {
      const updated = await api.createManualSegment(active.id, {
        start_sec: selection.startSec,
        end_sec: selection.endSec,
        person_ids: selectionTags,
      })
      markAuthoritativeWrite()
      void updated
      setSelection(null)
      setSelectionTags([])
      toast.success(
        selectionTags.length
          ? `Range added to ${selectionTags.length} pack${selectionTags.length === 1 ? "" : "s"}`
          : "Range saved (untagged)",
      )
      await refreshActive()
    } catch (e) {
      toast.error(friendlyError(e))
    } finally {
      setSavingRange(false)
    }
  }

  const deleteManualRange = async (id: string) => {
    if (!active) return
    try {
      await api.deleteManualSegment(active.id, id)
      markAuthoritativeWrite()
      toast.message("Range removed")
      await refreshActive()
    } catch (e) {
      toast.error(friendlyError(e))
    }
  }

  // --- copy timecodes ------------------------------------------------------
  const dropFrameAvailable = fps != null && supportsDropFrame(fps)
  const copyText_ = useMemo(() => {
    if (!person) return ""
    return formatSegmentsForCopy(includedClips, person.label, {
      format: copyPrefs.format,
      fps,
      dropFrame: copyPrefs.dropFrame && dropFrameAvailable,
      prefix: copyPrefs.prefix,
    })
  }, [includedClips, person, copyPrefs, fps, dropFrameAvailable])

  const updatePrefs = (patch: Partial<CopyPrefs>) => {
    setCopyPrefs((prev) => {
      const next = { ...prev, ...patch }
      try {
        localStorage.setItem(COPY_PREFS_KEY, JSON.stringify(next))
      } catch {
        /* storage may be unavailable */
      }
      return next
    })
  }

  // Edit ▸ Copy Timecodes. Must stay above the early return below.
  useMenuAction(
    "edit:copy-timecodes",
    person && includedClips.length
      ? () => {
          void copyText(copyText_, "Timecodes copied")
        }
      : null,
  )

  const onSelectBlock = useCallback(
    (block: TimelineBlock, lane: TimelineLane) => {
      // A hollow block is an offer, not a clip: clicking it adds that shot.
      if (block.ghost) {
        const idx = Number(block.key.split(":")[1])
        if (Number.isInteger(idx)) setShotOverride(idx, "include")
        requestSeek(block.startSec)
        return
      }
      if (lane.personId != null && lane.personId !== person?.person_id) {
        setSelectedPersonId(lane.personId)
      }
      requestSeek(block.startSec)
      if (lane.id === `person:${person?.person_id}` && !block.manual) {
        const idx = personClips.findIndex(
          (c) => c.start_sec === block.startSec && c.end_sec === block.endSec,
        )
        if (idx >= 0) setActiveSegment(idx)
      }
    },
    [person, personClips, requestSeek, setSelectedPersonId, setShotOverride],
  )

  if (!active || active.status !== "ready") {
    if (!projectsReady || (Boolean(activeId) && !active)) {
      return (
        <div className="flex min-h-0 flex-1 flex-col items-center justify-center p-8">
          <div className="w-full max-w-sm">
            <LoadingBlock label="Loading timeline…" rows={4} dense />
          </div>
        </div>
      )
    }
    return (
      <EmptyState
        title="Timeline unavailable"
        description="Finish scanning a project before reviewing segments."
        actionLabel="Import footage"
        onAction={() => nav("/import")}
      />
    )
  }

  const selectionDur = selection ? selection.endSec - selection.startSec : 0

  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <PageHeader
        eyebrow={active.name}
        title="Timeline"
        description="Scrub, zoom, mark ranges, and copy frame-accurate timecodes."
        actions={
          person ? (
            <div className="flex items-center gap-2">
              <CopyButton
                text={copyText_}
                label="Copy timecodes"
                toastLabel="Segment list copied"
              />
              <Button size="sm" className="gap-1.5" onClick={() => nav("/export")}>
                <Clapperboard className="size-3.5" />
                Export
              </Button>
            </div>
          ) : null
        }
      />

      <div className="grid min-h-0 flex-1 lg:grid-cols-[200px_minmax(0,1fr)_320px]">
        <aside className="hidden min-h-0 border-r border-border/60 lg:flex lg:flex-col">
          <p className="px-4 py-3 text-[10px] uppercase tracking-wider text-muted-foreground">
            Cast
          </p>
          <ScrollArea className="min-h-0 flex-1 px-2 pb-4">
            <div className="space-y-1">
              {active.people.map((p) => (
                <button
                  key={p.person_id}
                  type="button"
                  onClick={() => {
                    setSelectedPersonId(p.person_id)
                    setActiveSegment(0)
                  }}
                  className={cn(
                    "flex w-full items-center gap-2.5 rounded-lg px-2.5 py-2 text-left text-sm transition",
                    person?.person_id === p.person_id
                      ? "bg-sidebar-accent text-foreground"
                      : "text-muted-foreground hover:bg-muted/40",
                  )}
                >
                  <Avatar className="size-8 rounded-md">
                    {p.thumbnail_path ? (
                      <AvatarImage
                        src={api.mediaUrl(active.id, p.thumbnail_path)}
                        alt={p.label}
                      />
                    ) : null}
                    <AvatarFallback className="rounded-md text-[10px]">
                      {p.label.slice(0, 2).toUpperCase()}
                    </AvatarFallback>
                  </Avatar>
                  <span className="truncate">{p.label}</span>
                </button>
              ))}
            </div>
          </ScrollArea>
        </aside>

        <div className="flex min-h-0 flex-1 flex-col overflow-hidden border-r border-border/40">
          <div className="grid min-h-0 flex-1 gap-4 p-4 lg:grid-rows-[minmax(0,1fr)_auto] lg:p-6">
            <VideoPlayer
              src={api.videoUrl(active.id)}
              fps={fps}
              durationSec={active.duration_sec}
              segments={playerSegments}
              mode="source"
              activeKey={activeSegment === null ? null : `${activeSegment}`}
              onActiveChange={(key) =>
                setActiveSegment(key === null ? null : Number(key))
              }
              loop={loopSegment}
              onLoopChange={setLoopSegment}
              seekRequest={seekRequest}
              onTimeUpdate={setPlayheadSec}
              globalKeys
              className="min-h-0"
            />

            <div className="flex min-h-0 flex-col gap-3">
              <div className="flex flex-wrap items-center gap-2">
                <div className="inline-flex overflow-hidden rounded-md border border-border/50">
                  <button
                    type="button"
                    onClick={() => setViewMode("person")}
                    className={cn(
                      "inline-flex items-center gap-1.5 px-2.5 py-1 text-[11px] transition",
                      viewMode === "person"
                        ? "bg-primary/15 text-primary"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    <User className="size-3" />
                    Person
                  </button>
                  <button
                    type="button"
                    onClick={() => setViewMode("full")}
                    className={cn(
                      "inline-flex items-center gap-1.5 px-2.5 py-1 text-[11px] transition",
                      viewMode === "full"
                        ? "bg-primary/15 text-primary"
                        : "text-muted-foreground hover:text-foreground",
                    )}
                  >
                    <Layers className="size-3" />
                    Full video
                  </button>
                </div>

                <label className="inline-flex items-center gap-1.5 text-[11px] text-muted-foreground">
                  <input
                    type="checkbox"
                    checked={followPlayhead}
                    onChange={(e) => setFollowPlayhead(e.target.checked)}
                    className="size-3 accent-current"
                  />
                  Follow playhead
                </label>

                {excludedCount > 0 ? (
                  <span className="rounded border border-border/40 px-1.5 py-0.5 text-[10px] text-muted-foreground">
                    {excludedCount} excluded
                  </span>
                ) : null}

                <div className="ml-auto flex items-center gap-1.5">
                  <select
                    aria-label="Timecode format"
                    value={copyPrefs.format}
                    onChange={(e) =>
                      updatePrefs({ format: e.target.value as CopyFormat })
                    }
                    className="rounded-md border border-border/50 bg-background/60 px-1.5 py-1 text-[11px] text-muted-foreground outline-none"
                  >
                    <option value="timecode">HH:MM:SS:FF</option>
                    <option value="csv">CSV</option>
                    <option value="seconds">Seconds</option>
                  </select>
                  {dropFrameAvailable ? (
                    <label className="inline-flex items-center gap-1 text-[11px] text-muted-foreground">
                      <input
                        type="checkbox"
                        checked={copyPrefs.dropFrame}
                        onChange={(e) => updatePrefs({ dropFrame: e.target.checked })}
                        className="size-3 accent-current"
                      />
                      DF
                    </label>
                  ) : null}
                  <Input
                    value={copyPrefs.prefix}
                    onChange={(e) => updatePrefs({ prefix: e.target.value })}
                    placeholder="ep 1 cut"
                    aria-label="Clip name prefix"
                    className="h-7 w-28 text-[11px]"
                  />
                </div>
              </div>

              <PackContextBar
                pack={pack}
                hasSceneGroups={sceneGroups.length > 0}
                busy={packBusy}
                onModeChange={setContextMode}
                onPadChange={setPad}
                onRebuildScenes={() => void rebuildScenes()}
              />

              <TimelineCanvas
                lanes={lanes}
                durationSec={active.duration_sec}
                fps={fps}
                snapPoints={snapPoints}
                view={view}
                onViewChange={(v) => {
                  setFollowPlayhead(false)
                  setView(v)
                }}
                playheadSec={playheadSec}
                onSeek={requestSeek}
                activeKey={activeSegment === null ? null : `${activeSegment}`}
                onSelectBlock={onSelectBlock}
                onSelectLane={(lane) => {
                  if (lane.personId != null) setSelectedPersonId(lane.personId)
                }}
                selection={selection}
                onSelectionChange={setSelection}
                onSelectionCommit={onSelectionCommit}
              />

              {selection && selectionDur > 0 ? (
                <div className="rounded-lg border border-primary/40 bg-primary/5 p-3">
                  <div className="flex flex-wrap items-center gap-2">
                    <span className="timecode text-xs tabular-nums">
                      {formatTimecode(selection.startSec, fps ?? 0)} →{" "}
                      {formatTimecode(selection.endSec, fps ?? 0)}
                    </span>
                    <span className="text-[11px] text-muted-foreground">
                      {formatDuration(selectionDur)}
                    </span>
                    <div className="ml-auto flex gap-1.5">
                      <Button
                        size="sm"
                        variant="ghost"
                        className="h-7 text-[11px]"
                        onClick={() => {
                          setSelection(null)
                          setSelectionTags([])
                        }}
                      >
                        Cancel
                      </Button>
                      <Button
                        size="sm"
                        className="h-7 text-[11px]"
                        disabled={savingRange}
                        onClick={() => void saveManualRange()}
                      >
                        {savingRange ? "Saving…" : "Add range"}
                      </Button>
                    </div>
                  </div>
                  <div className="mt-2 flex flex-wrap items-center gap-1.5">
                    <span className="text-[10px] uppercase tracking-wider text-muted-foreground">
                      Add to
                    </span>
                    {active.people.map((p) => {
                      const on = selectionTags.includes(p.person_id)
                      return (
                        <button
                          key={p.person_id}
                          type="button"
                          onClick={() =>
                            setSelectionTags((prev) =>
                              prev.includes(p.person_id)
                                ? prev.filter((id) => id !== p.person_id)
                                : [...prev, p.person_id],
                            )
                          }
                          className={cn(
                            "rounded-md border px-1.5 py-0.5 text-[11px] transition",
                            on
                              ? "border-primary/50 bg-primary/15 text-primary"
                              : "border-border/40 text-muted-foreground hover:text-foreground",
                          )}
                        >
                          {p.label}
                        </button>
                      )
                    })}
                  </div>
                </div>
              ) : null}

              <ScrollArea className="max-h-48 min-h-0">
                <div className="space-y-1.5 pr-2">
                  {personClips.map((c, i) => {
                    const excluded =
                      person != null &&
                      isSegmentExcluded(exclusions, {
                        person_id: person.person_id,
                        start_sec: c.start_sec,
                        end_sec: c.end_sec,
                      } as Segment)
                    return (
                      <div
                        key={`${c.start_sec}-${i}`}
                        className={cn(
                          "flex w-full items-center gap-2 rounded-lg border px-3 py-2.5 text-left transition",
                          activeSegment === i
                            ? "border-primary/40 bg-primary/5"
                            : "border-border/40 bg-card/30 hover:bg-card/50",
                          excluded && "opacity-45",
                        )}
                      >
                        <button
                          type="button"
                          onClick={() => {
                            setActiveSegment(i)
                            requestSeek(c.start_sec)
                          }}
                          className="flex min-w-0 flex-1 items-center justify-between gap-2 text-left"
                        >
                          <span
                            className={cn(
                              "truncate text-sm font-medium",
                              excluded && "line-through",
                            )}
                          >
                            {clipName(copyPrefs.prefix, i + 1)}
                            {c.manual ? (
                              <span className="ml-1.5 rounded bg-[#7FB2E5]/20 px-1 py-0.5 text-[9px] uppercase tracking-wider text-[#7FB2E5]">
                                manual
                              </span>
                            ) : null}
                          </span>
                          <span className="timecode shrink-0 text-xs text-muted-foreground">
                            {formatTimecode(c.start_sec, fps ?? 0)} →{" "}
                            {formatTimecode(c.end_sec, fps ?? 0)} ·{" "}
                            {formatDuration(c.end_sec - c.start_sec)}
                          </span>
                        </button>
                        {c.manual && c.id ? (
                          <button
                            type="button"
                            aria-label="Delete manual range"
                            title="Delete manual range"
                            onClick={() => void deleteManualRange(c.id!)}
                            className="shrink-0 text-muted-foreground transition hover:text-destructive"
                          >
                            <Trash2 className="size-3.5" />
                          </button>
                        ) : shots.length ? (
                          <button
                            type="button"
                            aria-label="Drop this clip's opening shot"
                            title="Drop this clip's opening shot from the pack"
                            disabled={packBusy}
                            onClick={() => {
                              const si = shotIndexAt(c.start_sec + 0.01)
                              if (si >= 0) setShotOverride(si, "exclude")
                            }}
                            className="shrink-0 text-muted-foreground transition hover:text-destructive disabled:opacity-40"
                          >
                            <Scissors className="size-3.5" />
                          </button>
                        ) : null}
                      </div>
                    )
                  })}
                </div>
              </ScrollArea>
            </div>
          </div>
        </div>

        <aside className="hidden min-h-0 border-l border-border/60 bg-card/25 xl:flex xl:flex-col">
          {person ? (
            <ScrollArea className="min-h-0 flex-1">
              <PersonDossier
                projectId={active.id}
                people={active.people}
                person={person}
                onSelect={(id) => {
                  setSelectedPersonId(id)
                  setActiveSegment(0)
                }}
                allowDeselect={false}
                showTimelineCta={false}
                onExport={() => {
                  setSelectedPersonId(person.person_id)
                  nav("/export")
                }}
                onRename={async (label) => {
                  await api.renamePerson(active.id, person.person_id, label)
                  await refreshActive()
                }}
              />
            </ScrollArea>
          ) : null}
        </aside>
      </div>
    </div>
  )
}
