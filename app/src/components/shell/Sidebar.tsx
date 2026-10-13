import type * as React from "react"
import { Link, useRouterState } from "@tanstack/react-router"
import { Library, PanelLeftClose, PanelLeftOpen, Settings } from "lucide-react"
import { Tooltip } from "@/components/ui/tooltip"
import { shortcutKeys } from "@/lib/shortcuts"
import { cn } from "@/lib/utils"
import { useUi } from "@/state/ui"

type NavItemProps = {
  to: string
  icon: React.ReactNode
  label: string
  active: boolean
  collapsed: boolean
  shortcut?: string[]
}

function NavItem({ to, icon, label, active, collapsed, shortcut }: NavItemProps) {
  const link = (
    <Link
      to={to}
      aria-current={active ? "page" : undefined}
      className={cn(
        "group relative flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm font-medium transition-colors duration-[var(--dur-fast)]",
        "[&_svg]:size-4 [&_svg]:shrink-0",
        active ? "bg-active text-fg-1" : "text-fg-2 hover:bg-hover hover:text-fg-1",
        collapsed && "justify-center px-0",
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

export function Sidebar() {
  const collapsed = useUi((s) => s.sidebarCollapsed)
  const toggle = useUi((s) => s.toggleSidebar)
  const path = useRouterState({ select: (s) => s.location.pathname })

  return (
    <aside
      className={cn(
        "flex shrink-0 flex-col border-r border-hairline bg-panel transition-[width] duration-[var(--dur-slow)] ease-out",
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
