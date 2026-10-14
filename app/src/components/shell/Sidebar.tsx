import type * as React from "react"
import { Link, useRouterState } from "@tanstack/react-router"
import { Film, Library, ListVideo, PackageOpen, PanelLeftClose, PanelLeftOpen, Plus, Settings, Tv, Users } from "lucide-react"
import type { ProjectSummary } from "@/bindings"
import { Tooltip } from "@/components/ui/tooltip"
import { useProjects } from "@/lib/queries"
import { shortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useProjectIndexing } from "@/state/activity"
import { useNewProject } from "@/state/new-project"
import { useUi } from "@/state/ui"

type NavItemProps = {
  to: string
  icon: React.ReactNode
  label: string
  active: boolean
  collapsed: boolean
  shortcut?: string[]
  inset?: boolean
}

function NavItem({ to, icon, label, active, collapsed, shortcut, inset }: NavItemProps) {
  const link = (
    <Link
      to={to}
      aria-current={active ? "page" : undefined}
      className={cn(
        "group relative flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm font-medium transition-colors duration-[var(--dur-fast)]",
        "[&_svg]:size-4 [&_svg]:shrink-0",
        active ? "bg-active text-fg-1" : "text-fg-2 hover:bg-hover hover:text-fg-1",
        collapsed && "justify-center px-0",
        inset && !collapsed && "pl-8",
      )}
    >
      {active && <span className="absolute top-1.5 bottom-1.5 left-0 w-[2px] rounded-full bg-accent" aria-hidden />}
      <span className={cn(active ? "text-accent" : "text-fg-3 group-hover:text-fg-2")}>{icon}</span>
      {!collapsed && <span className="truncate">{label}</span>}
    </Link>
  )
  return collapsed ? (
    <Tooltip label={label} shortcut={shortcut} side="right">
      {link}
    </Tooltip>
  ) : (
    link
  )
}

/** How many projects the sidebar lists (most recently touched first). */
const RECENT = 8

export function Sidebar() {
  const collapsed = useUi((s) => s.sidebarCollapsed)
  const toggle = useUi((s) => s.toggleSidebar)
  const path = useRouterState({ select: (s) => s.location.pathname })
  const { data: projects } = useProjects()
  const currentPid = path.match(/^\/p\/([^/]+)/)?.[1]
  const recent = (projects ?? []).slice(0, RECENT)
  // Keep the open project visible even when it isn't among the most recent.
  const current = projects?.find((p) => p.id === currentPid)
  const listed = current && !recent.includes(current) ? [...recent, current] : recent

  return (
    <aside
      className={cn(
        "flex shrink-0 flex-col border-r border-hairline bg-panel",
        collapsed ? "w-[var(--sidebar-w-collapsed)]" : "w-[var(--sidebar-w)]",
      )}
    >
      <nav className="flex flex-1 flex-col gap-0.5 overflow-y-auto p-2" aria-label="Main">
        {!collapsed && <div className="eyebrow px-2.5 pt-2 pb-1.5">Workspace</div>}
        <NavItem
          to="/"
          icon={<Library />}
          label="Library"
          active={path === "/"}
          collapsed={collapsed}
          shortcut={shortcutKeys("library")}
        />

        {listed.length > 0 && (
          <>
            <div className={cn("flex items-center justify-between pt-4 pb-1.5", collapsed ? "px-0" : "pr-1 pl-2.5")}>
              {!collapsed && <span className="eyebrow">Projects</span>}
              <Tooltip label="New project" shortcut={shortcutKeys("newProject")} side="right">
                <button
                  onClick={() => useNewProject.getState().show()}
                  aria-label="New project"
                  className={cn(
                    "flex size-6 items-center justify-center rounded-sm text-fg-3 transition-colors hover:bg-hover hover:text-fg-1",
                    collapsed && "mx-auto",
                  )}
                >
                  <Plus className="size-3.5" />
                </button>
              </Tooltip>
            </div>
            {listed.map((p) => (
              <ProjectNav key={p.id} project={p} path={path} collapsed={collapsed} open={p.id === currentPid} />
            ))}
          </>
        )}
      </nav>

      <div className="flex flex-col gap-0.5 border-t border-hairline p-2">
        <NavItem
          to="/settings/general"
          icon={<Settings />}
          label="Settings"
          active={path.startsWith("/settings")}
          collapsed={collapsed}
          shortcut={shortcutKeys("settings")}
        />
        <Tooltip label={collapsed ? "Expand sidebar" : "Collapse sidebar"} shortcut={shortcutKeys("sidebar")} side="right">
          <button
            onClick={toggle}
            className={cn(
              "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm text-fg-3 transition-colors hover:bg-hover hover:text-fg-1 [&_svg]:size-4",
              collapsed && "justify-center px-0",
            )}
          >
            {collapsed ? <PanelLeftOpen /> : <PanelLeftClose />}
            {!collapsed && <span>Collapse</span>}
          </button>
        </Tooltip>
      </div>
    </aside>
  )
}

function ProjectNav({
  project: p,
  path,
  collapsed,
  open,
}: {
  project: ProjectSummary
  path: string
  collapsed: boolean
  open: boolean
}) {
  const base = `/p/${p.id}`
  const indexing = useProjectIndexing(p.id)
  const link = (
    <Link
      to="/p/$pid/episodes"
      params={{ pid: p.id }}
      className={cn(
        "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm font-medium transition-colors duration-[var(--dur-fast)]",
        open ? "text-fg-1" : "text-fg-2 hover:bg-hover hover:text-fg-1",
        collapsed && "justify-center px-0",
      )}
    >
      <span className={cn("relative shrink-0 [&_svg]:size-4", open ? "text-fg-1" : "text-fg-3 group-hover:text-fg-2")}>
        {p.kind === "movie" ? <Film /> : <Tv />}
        {indexing && <span className="absolute -top-0.5 -right-0.5 size-1.5 rounded-full bg-accent" aria-label="Indexing" />}
      </span>
      {!collapsed && <span className="truncate">{p.title}</span>}
    </Link>
  )
  return (
    <>
      {collapsed ? (
        <Tooltip label={p.title} side="right">
          {link}
        </Tooltip>
      ) : (
        link
      )}
      {open && (
        <>
          <NavItem
            to={`${base}/episodes`}
            icon={<ListVideo />}
            label={p.kind === "movie" ? "Files" : "Episodes"}
            active={path === `${base}/episodes` || path.startsWith(`${base}/timeline`)}
            collapsed={collapsed}
            inset
          />
          <NavItem
            to={`${base}/cast`}
            icon={<Users />}
            label="Cast"
            active={path.startsWith(`${base}/cast`)}
            collapsed={collapsed}
            inset
          />
          <NavItem
            to={`${base}/exports`}
            icon={<PackageOpen />}
            label="Exports"
            active={path === `${base}/exports`}
            collapsed={collapsed}
            inset
          />
        </>
      )}
    </>
  )
}
