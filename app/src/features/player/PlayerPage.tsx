import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router"
import { CaretLeft, CaretRight, SidebarSimple } from "@/components/icons"
import type { MediaItem } from "@/bindings"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { TopBar } from "@/components/ui/topbar"
import { Tooltip } from "@/components/ui/tooltip"
import { mediaTitle, releaseTitle } from "@/lib/media"
import { useFrameMap, useMediaList, useProject } from "@/lib/queries"
import { useUi } from "@/state/ui"
import { NotReady } from "./NotReady"
import { ReadyPlayer } from "./ReadyPlayer"

export function PlayerPage() {
  const { pid, mid } = useParams({ from: "/p/$pid/timeline/$mid" })
  const { f: openAt } = useSearch({ from: "/p/$pid/timeline/$mid" })
  const mediaQuery = useMediaList(pid)
  const { data: media } = mediaQuery
  const { data: project } = useProject(pid)
  const m = media?.find((x) => x.id === mid)
  const ready = m?.status === "ready"
  const map = useFrameMap(pid, mid, ready)
  const index = media?.findIndex((x) => x.id === mid) ?? -1
  const prev = index > 0 ? media![index - 1] : undefined
  const next = index >= 0 && index < (media?.length ?? 0) - 1 ? media![index + 1] : undefined
  const inspector = useUi((s) => s.inspectorOpen)
  const toggleInspector = useUi((s) => s.toggleInspector)
  const navigate = useNavigate()
  const watch = (target?: MediaItem) => target && void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: target.id } })

  return (
    <ContextArea
      kind="page"
      menu={
        <>
          <ContextMenuItem disabled={!prev} onSelect={() => watch(prev)}>
            <CaretLeft /> Previous episode
          </ContextMenuItem>
          <ContextMenuItem disabled={!next} onSelect={() => watch(next)}>
            <CaretRight /> Next episode
          </ContextMenuItem>
          <ContextMenuSeparator />
          <ContextMenuItem onSelect={toggleInspector}>
            <SidebarSimple mirrored /> {inspector ? "Hide details" : "Show details"}
          </ContextMenuItem>
        </>
      }
    >
      <div className="flex h-full min-h-0 flex-col bg-inset">
        <TopBar
          back={{ label: project?.kind === "movie" ? "Files" : "Episodes", to: `/p/${pid}/episodes` }}
          title={
            <Tooltip label={m?.fileName} side="bottom" delay={600} disabled={!m}>
              <span className="flex min-w-0 items-baseline gap-2.5">
                {m?.label && <span className="timecode shrink-0 text-md font-medium text-fg-2">{m.label}</span>}
                <span className="truncate">{m ? releaseTitle(m.fileName) : ""}</span>
              </span>
            </Tooltip>
          }
          className="border-white/[0.05] bg-black/50"
        >
          {map.data?.isProxy && (
            <Tooltip label="Playing a preview copy; exports always cut from the original file.">
              <span>
                <Badge tone="outline">Preview</Badge>
              </span>
            </Tooltip>
          )}
          <div className="flex items-center gap-1">
            <EpisodeNav pid={pid} target={prev} dir="prev" />
            <EpisodeNav pid={pid} target={next} dir="next" />
          </div>
          <Tooltip label={inspector ? "Hide details" : "Show details"}>
            <Button
              size="icon-sm"
              variant={inspector ? "secondary" : "ghost"}
              aria-pressed={inspector}
              aria-label="Details"
              onClick={toggleInspector}
            >
              <SidebarSimple mirrored />
            </Button>
          </Tooltip>
        </TopBar>
        {m && ready && map.data ? (
          <ReadyPlayer
            key={`${mid}:${map.data.src}`}
            pid={pid}
            media={m}
            map={map.data}
            openAt={openAt}
            inspector={inspector}
          />
        ) : (
          <NotReady
            pid={pid}
            media={m}
            error={map.error}
            gone={mediaQuery.isSuccess && !m}
            listError={mediaQuery.data === undefined ? mediaQuery.error : null}
            onRetryList={() => void mediaQuery.refetch()}
          />
        )}
      </div>
    </ContextArea>
  )
}

function EpisodeNav({ pid, target, dir }: { pid: string; target?: MediaItem; dir: "prev" | "next" }) {
  const label = dir === "prev" ? "Previous episode" : "Next episode"
  const icon = dir === "prev" ? <CaretLeft /> : <CaretRight />
  if (!target) {
    return (
      <Button size="icon-sm" variant="ghost" disabled aria-label={label}>
        {icon}
      </Button>
    )
  }
  return (
    <Tooltip label={`${label}: ${mediaTitle(target)}`}>
      <Button size="icon-sm" variant="secondary" asChild>
        <Link to="/p/$pid/timeline/$mid" params={{ pid, mid: target.id }} aria-label={label}>
          {icon}
        </Link>
      </Button>
    </Tooltip>
  )
}
