import { useEffect, useRef } from "react"
import { useMatches } from "@tanstack/react-router"
import type { DiscordActivityPayload, DiscordButton, DiscordRpcSettings } from "@/bindings"
import { commands } from "@/lib/ipc"
import { mediaTitle } from "@/lib/media"
import { isTauri } from "@/lib/platform"
import { useCast, useMediaList, useProject, useSettings } from "@/lib/queries"
import { useJobs } from "@/state/activity"

const DEFAULT_CLIENT_ID = "1567208524940058695"
const APP_URL = "https://github.com/tcmmi/AutoScene"
const SESSION_START = Math.floor(Date.now() / 1000)

function renderTemplate(
  template: string,
  vars: Record<string, string | number | undefined | null>,
): string {
  let res = template
  for (const [k, v] of Object.entries(vars)) {
    res = res.replaceAll(`{${k}}`, v != null ? String(v) : "")
  }
  return res.trim().replace(/^•\s*|\s*•$/g, "").trim()
}

/**
 * Native Discord Rich Presence updater hook.
 * Monitors router navigation, current project/character/media data, active jobs,
 * and user preferences to push live presence to the Discord desktop client.
 */
export function useDiscordRPC() {
  const { data: settings } = useSettings()
  const jobs = useJobs()
  const matches = useMatches()

  // Track session start timestamp
  const startedRef = useRef(false)

  const rpc: Required<DiscordRpcSettings> = {
    enabled: settings?.discordRpc?.enabled ?? true,
    clientId: settings?.discordRpc?.clientId || DEFAULT_CLIENT_ID,
    showTimer: settings?.discordRpc?.showTimer ?? true,
    showButtons: settings?.discordRpc?.showButtons ?? true,
    showProject: settings?.discordRpc?.showProject ?? true,
    showEpisode: settings?.discordRpc?.showEpisode ?? true,
    showCharacter: settings?.discordRpc?.showCharacter ?? true,
    showProgress: settings?.discordRpc?.showProgress ?? true,
    showCounts: settings?.discordRpc?.showCounts ?? true,
    templateProject: settings?.discordRpc?.templateProject || "{project} • {episode}",
    templateScenepack: settings?.discordRpc?.templateScenepack || "Scenepack: {character} ({count} clips)",
    templateExport: settings?.discordRpc?.templateExport || "Exporting {item} • {progress}%",
  }

  // Find route params
  const currentMatch = matches[matches.length - 1]
  const params = (currentMatch?.params ?? {}) as Record<string, string | undefined>
  const pid = params.pid
  const mid = params.mid
  const cid = params.cid

  // Fetch contextual project data
  const { data: project } = useProject(pid ?? "")
  const { data: mediaList } = useMediaList(pid ?? "")
  const { data: cast } = useCast(pid ?? "")

  // Start / stop Discord RPC worker thread
  useEffect(() => {
    if (!isTauri) return

    if (rpc.enabled) {
      commands.discordRpcStart(rpc.clientId).catch(() => {})
      startedRef.current = true
    } else if (startedRef.current) {
      commands.discordRpcStop().catch(() => {})
      startedRef.current = false
    }

    return () => {
      // Don't stop immediately on component remounts in dev, only on app shutdown (handled by Rust)
    }
  }, [rpc.enabled, rpc.clientId])

  // Update Discord activity state
  useEffect(() => {
    if (!isTauri || !rpc.enabled) return

    // Check if an export job is currently running
    const activeExport = jobs.find((j) => j.kind === "export" && j.state === "running")

    let details: string | null = null
    let state: string | null = null

    const buttons: DiscordButton[] = rpc.showButtons
      ? [{ label: "Get AutoScene", url: APP_URL }]
      : []

    if (activeExport && rpc.showProgress) {
      const progress = activeExport.progress != null ? Math.round(activeExport.progress * 100) : 0
      details = renderTemplate(rpc.templateExport, {
        item: activeExport.title || "Project",
        progress,
      })
      state = project?.title && rpc.showProject ? project.title : "Rendering"
    } else if (pid && project) {
      const activeMedia = mediaList?.find((m) => m.id === mid)
      const activeChar = cast?.characters.find((c) => c.id === cid)

      const projectName = rpc.showProject ? project.title : "AutoScene"
      const episodeName = rpc.showEpisode && activeMedia ? mediaTitle(activeMedia) : undefined

      if (cid && activeChar && rpc.showCharacter) {
        details = projectName
        const clipCount = rpc.showCounts ? (activeChar.shots ?? 0) : undefined
        state = renderTemplate(rpc.templateScenepack, {
          character: activeChar.label || "Character",
          count: clipCount,
        })
      } else if (mid && activeMedia) {
        details = renderTemplate(rpc.templateProject, {
          project: projectName,
          episode: episodeName,
        })
        state = "Timeline editing"
      } else {
        details = renderTemplate(rpc.templateProject, {
          project: projectName,
          episode: episodeName,
        })
        state = "Managing project"
      }
    } else {
      details = "Organizing Library"
      state = "Browsing projects"
    }

    const payload: DiscordActivityPayload = {
      details: details || "Editing in AutoScene",
      state: state || null,
      startTimestamp: rpc.showTimer ? SESSION_START : null,
      largeImage: "as-logo",
      largeText: "AutoScene",
      smallImage: null,
      smallText: null,
      buttons,
    }

    commands.discordRpcUpdate(payload).catch(() => {})
  }, [
    rpc.enabled,
    rpc.showTimer,
    rpc.showButtons,
    rpc.showProject,
    rpc.showEpisode,
    rpc.showCharacter,
    rpc.showCounts,
    rpc.showProgress,
    rpc.templateProject,
    rpc.templateScenepack,
    rpc.templateExport,
    project,
    mediaList,
    cast,
    pid,
    mid,
    cid,
    jobs,
    matches,
  ])
}
