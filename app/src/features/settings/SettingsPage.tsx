import type * as React from "react"
import { Link, useNavigate, useParams } from "@tanstack/react-router"
import { DownloadSimple, Info, Keyboard, SlidersHorizontal, Users } from "@/components/icons"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuRadioGroup, ContextMenuRadioItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { TopBar } from "@/components/ui/topbar"
import { SETTINGS_SECTIONS, type SettingsSection, useAppActions } from "@/lib/actions"
import { cn } from "@/lib/utils"
import { AboutSection } from "./sections/AboutSection"
import { ContributorsSection } from "./sections/ContributorsSection"
import { GeneralSection } from "./sections/GeneralSection"
import { ShortcutsSection } from "./sections/ShortcutsSection"
import { UpdatesSection } from "./sections/UpdatesSection"
import { SETTINGS_LABELS } from "./sections"

const META: Record<SettingsSection, { icon: React.ReactNode; render: () => React.ReactNode }> = {
  general: { icon: <SlidersHorizontal />, render: () => <GeneralSection /> },
  shortcuts: { icon: <Keyboard />, render: () => <ShortcutsSection /> },
  updates: { icon: <DownloadSimple />, render: () => <UpdatesSection /> },
  about: { icon: <Info />, render: () => <AboutSection /> },
  contributors: { icon: <Users />, render: () => <ContributorsSection /> },
}

export function SettingsPage() {
  const { section } = useParams({ from: "/settings/$section" })
  const navigate = useNavigate()
  const { checkUpdates } = useAppActions()
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
                <Link
                  key={s}
                  to="/settings/$section"
                  params={{ section: s }}
                  className={cn(
                    "group flex h-11 items-center gap-3 rounded-xl px-2 text-md font-medium transition-colors",
                    s === section ? "bg-white/[0.09] text-fg-1" : "text-fg-2 hover:bg-white/[0.05] hover:text-fg-1",
                  )}
                >
                  <span
                    className={cn(
                      "flex size-7 items-center justify-center rounded-lg transition-colors [&_svg]:size-4",
                      s === section ? "bg-accent text-on-accent" : "bg-white/[0.07] text-fg-2 group-hover:text-fg-1",
                    )}
                  >
                    {META[s].icon}
                  </span>
                  {SETTINGS_LABELS[s]}
                </Link>
              ))}
            </div>
          </nav>
          <div className="min-w-0 flex-1 overflow-y-auto">
            <div key={section} className="mx-auto max-w-3xl animate-rise-in px-10 pt-8 pb-14">
              <h2 className="display mb-8 text-5xl text-fg-1">{SETTINGS_LABELS[section]}</h2>
              {META[section].render()}
            </div>
          </div>
        </div>
      </ContextArea>
    </div>
  )
}
