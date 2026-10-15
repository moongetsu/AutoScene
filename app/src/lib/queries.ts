import { QueryClient, useQuery } from "@tanstack/react-query"
import { commands, type CharacterId, type FaceTrackItem, type MediaId, type ProjectId, type TrackFilter } from "./ipc"

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      // Data changes are pushed as events that invalidate queries; never poll.
      staleTime: Infinity,
      refetchOnWindowFocus: false,
      retry: 1,
    },
  },
})

/** Query keys. `lib/events.ts` maps backend events onto these. */
export const qk = {
  appInfo: ["appInfo"] as const,
  settings: ["settings"] as const,
  mediaTools: ["mediaTools"] as const,
  presets: ["exportPresets"] as const,
  projects: ["projects"] as const,
  project: (id: ProjectId) => ["project", id] as const,
  media: (pid: ProjectId) => ["media", pid] as const,
  frameMap: (pid: ProjectId, mid: MediaId) => ["frameMap", pid, mid] as const,
  segments: (pid: ProjectId, mid: MediaId) => ["segments", pid, mid] as const,
  exports: (pid: ProjectId) => ["exports", pid] as const,
  cast: (pid: ProjectId) => ["cast", pid] as const,
  tracks: (pid: ProjectId, filter: TrackFilter) => ["cast", pid, "tracks", filter] as const,
  // Outside the cast: a face's portrait doesn't change when the cast does.
  // The face's place keeps a track number handed out again apart.
  portraits: (pid: ProjectId) => ["portrait", pid] as const,
  portrait: (pid: ProjectId, t: Pick<FaceTrackItem, "id" | "mediaId" | "start">) =>
    ["portrait", pid, t.id, t.mediaId, t.start] as const,
  presence: (pid: ProjectId, mid: MediaId) => ["cast", pid, "presence", mid] as const,
  characterShots: (pid: ProjectId, cid: CharacterId) => ["cast", pid, "shots", cid] as const,
  // Under the cast: a pack is made from it, so every cast change refreshes it.
  pack: (pid: ProjectId, cid: CharacterId) => ["cast", pid, "pack", cid] as const,
  exportDir: (pid: ProjectId) => ["exportDir", pid] as const,
}

export function useAppInfo() {
  return useQuery({ queryKey: qk.appInfo, queryFn: () => commands.appInfo() })
}

export function useSettings() {
  return useQuery({ queryKey: qk.settings, queryFn: () => commands.settingsGet() })
}

export function useMediaTools() {
  return useQuery({ queryKey: qk.mediaTools, queryFn: () => commands.mediaTools() })
}

export function usePresets() {
  return useQuery({ queryKey: qk.presets, queryFn: () => commands.exportPresets() })
}

export function useProjects() {
  return useQuery({ queryKey: qk.projects, queryFn: () => commands.projectsList() })
}

export function useProject(id: ProjectId) {
  return useQuery({ queryKey: qk.project(id), queryFn: () => commands.projectGet(id) })
}

export function useMediaList(pid: ProjectId) {
  return useQuery({ queryKey: qk.media(pid), queryFn: () => commands.mediaList(pid) })
}

export function useFrameMap(pid: ProjectId, mid: MediaId, enabled: boolean) {
  return useQuery({
    queryKey: qk.frameMap(pid, mid),
    queryFn: () => commands.mediaFrameMap(pid, mid),
    enabled,
    retry: false,
  })
}

/** Shots and transitions of a media (empty until detected). */
export function useSegments(pid: ProjectId, mid: MediaId, enabled: boolean) {
  return useQuery({ queryKey: qk.segments(pid, mid), queryFn: () => commands.mediaSegments(pid, mid), enabled })
}

export function useExports(pid: ProjectId) {
  return useQuery({ queryKey: qk.exports(pid), queryFn: () => commands.exportsList(pid) })
}

export function useExportDir(pid: ProjectId) {
  return useQuery({ queryKey: qk.exportDir(pid), queryFn: () => commands.exportDefaultDir(pid) })
}

/** The project's characters and where face analysis stands. */
export function useCast(pid: ProjectId) {
  return useQuery({ queryKey: qk.cast(pid), queryFn: () => commands.castGet(pid) })
}

export function useCastTracks(pid: ProjectId, filter: TrackFilter, enabled = true) {
  return useQuery({ queryKey: qk.tracks(pid, filter), queryFn: () => commands.castTracks(pid, filter), enabled })
}

/**
 * A face's sharp portrait for showing it big (made on first ask, from the
 * exact frame); `null` while it can't be made. Show the thumbnail meanwhile.
 */
export const portraitQuery = (pid: ProjectId, t: FaceTrackItem) => ({
  queryKey: qk.portrait(pid, t),
  queryFn: () => commands.castPortrait(pid, t.id),
  // A failure leaves the thumbnail; it isn't worth a retry.
  retry: false,
})

export function usePortrait(pid: ProjectId, t: FaceTrackItem | undefined) {
  return useQuery({
    queryKey: t ? qk.portrait(pid, t) : qk.portraits(pid),
    queryFn: () => commands.castPortrait(pid, t!.id),
    retry: false,
    enabled: t !== undefined,
  })
}

/** Every shot a character is seen in, in episode order. */
export function useCharacterShots(pid: ProjectId, cid: CharacterId, enabled = true) {
  return useQuery({
    queryKey: qk.characterShots(pid, cid),
    queryFn: () => commands.characterShots(pid, cid),
    enabled,
  })
}

/** Where characters are on screen in one media. */
export function usePresence(pid: ProjectId, mid: MediaId, enabled: boolean) {
  return useQuery({ queryKey: qk.presence(pid, mid), queryFn: () => commands.mediaPresence(pid, mid), enabled })
}

type Refetchable = { isError: boolean; error: unknown; data?: unknown; refetch: () => unknown }

/**
 * The first of a view's queries that failed with nothing to show, and a retry
 * that fetches those again. A refetch that fails keeps its last data on screen.
 */
export function firstFailure(...queries: Refetchable[]): { error: unknown; retry: () => void } | null {
  const failed = queries.filter((q) => q.isError && q.data === undefined)
  if (failed.length === 0) return null
  return { error: failed[0].error, retry: () => failed.forEach((q) => void q.refetch()) }
}
