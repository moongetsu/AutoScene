import type * as React from "react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import { DownloadSimple, Info, Keyboard, PuzzlePiece, SlidersHorizontal, Users } from "@/components/icons"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuRadioGroup, ContextMenuRadioItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { TopBar } from "@/components/ui/topbar"
import { PluginIcon } from "@/features/plugins/PluginIcon"
import { PluginName, PluginSettingsSection } from "@/features/plugins/PluginSettingsSection"
import { pluginPages } from "@/features/plugins/plugin-items"
import { type PluginSection, SETTINGS_SECTIONS, type SettingsSection, isPluginSection, useAppActions } from "@/lib/actions"
import { usePlugins } from "@/lib/queries"
import { cn } from "@/lib/utils"
import { AboutSection } from "./sections/AboutSection"
import { ContributorsSection } from "./sections/ContributorsSection"
import { GeneralSection } from "./sections/GeneralSection"
import { PluginsSection } from "./sections/PluginsSection"
import { ShortcutsSection } from "./sections/ShortcutsSection"
import { UpdatesSection } from "./sections/UpdatesSection"
import { SETTINGS_LABELS } from "./sections"

const META: Record<SettingsSection, { icon: React.ReactNode; render: () => React.ReactNode }> = {
  general: { icon: <SlidersHorizontal />, render: () => <GeneralSection /> },
  shortcuts: { icon: <Keyboard />, render: () => <ShortcutsSection /> },
  updates: { icon: <DownloadSimple />, render: () => <UpdatesSection /> },
  plugins: { icon: <PuzzlePiece />, render: () => <PluginsSection /> },
  about: { icon: <Info />, render: () => <AboutSection /> },
  contributors: { icon: <Users />, render: () => <ContributorsSection /> },
}

export function SettingsPage() {
  const { section } = useParams({ from: "/settings/$section" })
  const navigate = useNavigate()
  const { checkUpdates } = useAppActions()
  // A plugin's page, else one of the app's.
  const plugin = isPluginSection(section) ? section.slice("plugin.".length) : null
  const { data: plugins } = usePlugins()
  const pages = pluginPages(plugins)
  const menu = (
    <>
      <ContextMenuRadioGroup
        value={section}
        onValueChange={(s) => void navigate({ to: "/settings/$section", params: { section: s as SettingsSection } })}
      >
        {SETTINGS_SECTIONS.map((s) => (
          <ContextMenuRadioItem key={s} value={s}>
            {SETTINGS_LABELS[s]}
          </ContextMenuRadioItem>
        ))}
      </ContextMenuRadioGroup>
      <ContextMenuSeparator />
      <ContextMenuItem onSelect={checkUpdates}>
        <DownloadSimple /> Check for updates
      </ContextMenuItem>
    </>
  )
  return (
    <div className="flex h-full flex-col">
      <TopBar title="Settings" />
      <ContextArea kind="page" menu={menu}>
        <div className="flex min-h-0 flex-1">
          <nav aria-label="Settings" className="w-60 shrink-0 border-r border-white/[0.06] px-3 pt-6">
            <h1 className="display mb-5 px-3 text-4xl text-fg-1">Settings</h1>
            <div className="flex flex-col gap-1">
              {SETTINGS_SECTIONS.map((s) => (
                <NavLink key={s} section={s} current={section} icon={META[s].icon}>
                  {SETTINGS_LABELS[s]}
                </NavLink>
              ))}
              {pages.length > 0 && <div role="separator" className="mx-2 my-2 h-px bg-white/[0.06]" />}
              {pages.map((p) => (
                <NavLink key={p.id} section={`plugin.${p.id}`} current={section} icon={<PluginIcon src={p.icon} />}>
                  {/* One that can't run now stays, dimmed: its page says why. */}
                  <span className={cn("truncate", !p.active && "opacity-50")}>{p.name}</span>
                </NavLink>
              ))}
            </div>
          </nav>
          <div className="min-w-0 flex-1 overflow-y-auto">
            <div key={section} className="mx-auto max-w-3xl animate-rise-in px-10 pt-8 pb-14">
              <h2 className="display mb-8 text-5xl text-fg-1">
                {plugin ? <PluginName id={plugin} /> : SETTINGS_LABELS[section as SettingsSection]}
              </h2>
              {plugin ? <PluginSettingsSection id={plugin} /> : META[section as SettingsSection].render()}
            </div>
          </div>
        </div>
      </ContextArea>
    </div>
  )
}

function NavLink({
  section,
  current,
  icon,
  children,
}: {
  section: SettingsSection | PluginSection
  current: string
  icon: React.ReactNode
  children: React.ReactNode
}) {
  const here = section === current
  return (
    <Link
      to="/settings/$section"
      params={{ section }}
      className={cn(
        "group flex h-11 min-w-0 items-center gap-3 rounded-xl px-2 text-md font-medium transition-colors",
        here ? "bg-white/[0.09] text-fg-1" : "text-fg-2 hover:bg-white/[0.05] hover:text-fg-1",
      )}
    >
      <span
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-lg transition-colors [&_svg]:size-4",
          here ? "bg-accent text-on-accent" : "bg-white/[0.07] text-fg-2 group-hover:text-fg-1",
        )}
      >
        {icon}
      </span>
      {children}
    </Link>
  )
}
