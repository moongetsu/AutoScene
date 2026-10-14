import { QueryClient, useQuery } from "@tanstack/react-query"
import { commands, type MediaId, type ProjectId } from "./ipc"

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
