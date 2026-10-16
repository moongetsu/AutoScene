import type * as React from "react"
import { Link, useRouterState } from "@tanstack/react-router"
import { FilmStrip, GearSix, MagnifyingGlass, Plus, Queue, SquaresFour, Television } from "@/components/icons"
import type { ProjectSummary } from "@/bindings"
import { AutoSceneMark } from "@/components/brand/AutoSceneMark"
import { Artwork } from "@/components/ui/artwork"
import { Tooltip } from "@/components/ui/tooltip"
import { useAppActions } from "@/lib/actions"
import { useProjects } from "@/lib/queries"
import { shortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useActiveCount, useActiveProgress, useProjectIndexing } from "@/state/activity"
import { useNewProject } from "@/state/new-project"
import { useUi } from "@/state/ui"

/** How many projects the rail shows (most recently touched first, plus the open one). */
const RECENT = 7

/**
 * The window's left edge, on the native material where there is one: the
 * mark, the library, the projects as their pictures, and at the bottom
 * search, background work and settings.
 */
export function Rail() {
  const actions = useAppActions()
  const path = useRouterState({ select: (s) => s.location.pathname })
  const { data: projects } = useProjects()
  const currentPid = path.match(/^\/p\/([^/]+)/)?.[1]
  const recent = (projects ?? []).slice(0, RECENT)
  const current = projects?.find((p) => p.id === currentPid)
  const listed = current && !recent.includes(current) ? [...recent, current] : recent

  return (
    <nav
      aria-label="Main"
      className="relative z-10 flex w-[var(--rail-w)] shrink-0 flex-col items-center border-r border-white/[0.06] bg-rail pt-[calc(var(--traffic-inset)+12px)] pb-3"
    >
      {/* The window is dragged by the rail's top, around the mark. */}
      <div data-tauri-drag-region className="absolute inset-x-0 top-0 h-[var(--topbar-h)]" />
      <Tooltip label="Library" side="right">
        <Link to="/" aria-label="AutoScene library" className="relative mb-4 rounded-xl">
          <AutoSceneMark className="size-9 drop-shadow-[0_6px_16px_rgb(224_180_92/0.25)]" />
        </Link>
      </Tooltip>

      <RailLink to="/" icon={<SquaresFour />} label="Library" active={path === "/"} shortcut={shortcutKeys("library")} />

      <div className="my-3 h-px w-8 bg-white/10" />

      <div className="flex min-h-0 w-full flex-1 flex-col items-center gap-3 overflow-y-auto pt-1 pb-2 [scrollbar-width:none] [&::-webkit-scrollbar]:hidden">
        {listed.map((p) => (
          <ProjectTile key={p.id} project={p} open={p.id === currentPid} />
        ))}
        <Tooltip label="New project" shortcut={shortcutKeys("newProject")} side="right">
          <button
            onClick={() => useNewProject.getState().show()}
            aria-label="New project"
            className="flex size-11 shrink-0 items-center justify-center rounded-[14px] border border-dashed border-white/20 text-fg-3 transition-colors hover:border-white/40 hover:bg-white/[0.05] hover:text-fg-1 [&_svg]:size-[18px]"
          >
            <Plus />
          </button>
        </Tooltip>
      </div>

      <div className="flex flex-col items-center gap-1 pt-2">
        <RailButton icon={<MagnifyingGlass />} label="Search" onClick={actions.openPalette} shortcut={shortcutKeys("palette")} />
        <ActivityButton />
        <RailLink
          to="/settings/general"
          icon={<GearSix />}
          label="Settings"
          active={path.startsWith("/settings")}
          shortcut={shortcutKeys("settings")}
        />
      </div>
    </nav>
  )
}

const itemClass = "group flex w-[64px] flex-col items-center gap-1 rounded-xl py-1.5"
const pillClass =
  "flex h-8 w-11 items-center justify-center rounded-full transition-[background-color,color] duration-[var(--dur-fast)] [&>svg]:size-5"

function RailLink({
  to,
  icon,
  label,
  active,
  shortcut,
}: {
  to: string
  icon: React.ReactElement<{ weight?: string }>
  label: string
  active: boolean
  shortcut?: string[]
}) {
  return (
    <Tooltip label={label} shortcut={shortcut} side="right">
      <Link to={to} aria-current={active ? "page" : undefined} className={itemClass}>
        <span className={cn(pillClass, active ? "bg-white/[0.14] text-fg-1" : "text-fg-2 group-hover:bg-white/[0.07] group-hover:text-fg-1")}>
          {active ? <IconWeight icon={icon} /> : icon}
        </span>
        <span className={cn("text-[10.5px] leading-3 font-medium", active ? "text-fg-1" : "text-fg-3 group-hover:text-fg-2")}>
          {label}
        </span>
      </Link>
    </Tooltip>
  )
}

/** The active place shows its icon filled. */
function IconWeight({ icon }: { icon: React.ReactElement<{ weight?: string }> }) {
  const Comp = icon.type as React.ComponentType<{ weight?: string }>
  return <Comp {...icon.props} weight="fill" />
}

function RailButton({
  icon,
  label,
  onClick,
  shortcut,
  pressed,
  children,
}: {
  icon: React.ReactNode
  label: string
  onClick: () => void
  shortcut?: string[]
  pressed?: boolean
  children?: React.ReactNode
}) {
  return (
    <Tooltip label={label} shortcut={shortcut} side="right">
      <button onClick={onClick} aria-pressed={pressed} className={itemClass}>
        <span
          className={cn(
            pillClass,
            "relative",
            pressed ? "bg-white/[0.14] text-fg-1" : "text-fg-2 group-hover:bg-white/[0.07] group-hover:text-fg-1",
          )}
        >
          {icon}
          {children}
        </span>
        <span className={cn("text-[10.5px] leading-3 font-medium", pressed ? "text-fg-1" : "text-fg-3 group-hover:text-fg-2")}>
          {label}
        </span>
      </button>
    </Tooltip>
  )
}

/** Background work: a ring fills as it runs; the count rides on the icon. */
function ActivityButton() {
  const actions = useAppActions()
  const open = useUi((s) => s.activityOpen)
  const active = useActiveCount()
  const progress = useActiveProgress()
  return (
    <RailButton
      icon={<Queue className={cn(active > 0 && "text-accent")} />}
      label="Activity"
      onClick={actions.toggleActivity}
      shortcut={shortcutKeys("activity")}
      pressed={open}
    >
      {active > 0 && (
        <>
          <div className="pointer-events-none absolute inset-0 flex items-center justify-center">
            <svg aria-hidden viewBox="0 0 32 32" className="!size-8 overflow-visible">
              <circle
                cx="16"
                cy="16"
                r="14"
                fill="none"
                stroke="currentColor"
                strokeWidth="2"
                className="text-white/[0.08]"
              />
              <circle
                cx="16"
                cy="16"
                r="14"
                pathLength={100}
                fill="none"
                stroke="var(--accent)"
                strokeWidth="2"
                strokeLinecap="round"
                strokeDasharray={progress == null ? "24 76" : `${Math.max(2, progress * 100)} 100`}
                className={cn(
                  "drop-shadow-[0_0_6px_var(--accent)] transition-[stroke-dasharray] duration-[var(--dur-slow)]",
                  progress == null
                    ? "origin-center animate-spin [animation-duration:1.6s]"
                    : "-rotate-90 origin-center",
                )}
              />
            </svg>
          </div>
          <span className="timecode absolute -top-1 -right-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-accent px-1 text-[10px] leading-none font-semibold text-on-accent shadow-[0_0_6px_var(--accent)]">
            {active}
          </span>
        </>
      )}
      <span className="sr-only">{active > 0 ? `${active} running` : "Nothing running"}</span>
    </RailButton>
  )
}

function ProjectTile({ project: p, open }: { project: ProjectSummary; open: boolean }) {
  const indexing = useProjectIndexing(p.id)
  const initials = p.title
    .split(/\s+/)
    .slice(0, 2)
    .map((w) => w[0])
    .join("")
    .toUpperCase()
  return (
    <Tooltip
      side="right"
      label={
        <span className="flex flex-col">
          <span className="font-semibold">{p.title}</span>
          <span className="text-fg-3">
            {p.kind === "movie" ? "Movie" : `${p.mediaCount} ${p.mediaCount === 1 ? "episode" : "episodes"}`}
            {indexing && " · indexing"}
          </span>
        </span>
      }
    >
      <Link
        to="/p/$pid"
        params={{ pid: p.id }}
        aria-label={p.title}
        aria-current={open ? "true" : undefined}
        className="group relative flex shrink-0 items-center"
      >
        <span
          aria-hidden
          className={cn(
            "absolute -left-[16px] w-1 rounded-r-full bg-accent transition-[height,opacity] duration-[var(--dur-med)] ease-out",
            open ? "h-6 opacity-100" : "h-2 opacity-0 group-hover:opacity-60",
          )}
        />
        <Artwork
          path={p.posterSmall ?? p.poster}
          className={cn(
            "size-11 rounded-[14px] transition-[transform,box-shadow] duration-[var(--dur-med)] ease-out group-hover:scale-105",
            open
              ? "shadow-[0_0_0_2px_var(--bg-canvas),0_0_0_4px_var(--accent)]"
              : "shadow-[inset_0_0_0_1px_rgb(255_255_255/0.1)]",
          )}
          fallback={
            <span className="flex size-full items-center justify-center artwork-fallback text-xs font-semibold text-fg-2">
              {initials || (p.kind === "movie" ? <FilmStrip /> : <Television />)}
            </span>
          }
        />
        {indexing && (
          <span className="absolute -right-0.5 -bottom-0.5 size-3 rounded-full border-2 border-canvas bg-accent" aria-hidden />
        )}
      </Link>
    </Tooltip>
  )
}
