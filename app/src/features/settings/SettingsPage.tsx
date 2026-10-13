import type * as React from "react"
import { Link, useParams } from "@tanstack/react-router"
import { Download, Info, Keyboard, SlidersHorizontal } from "lucide-react"
import { Page } from "@/components/ui/page"
import { SETTINGS_SECTIONS, type SettingsSection } from "@/lib/actions"
import { cn } from "@/lib/utils"
import { AboutSection } from "./sections/AboutSection"
import { GeneralSection } from "./sections/GeneralSection"
import { ShortcutsSection } from "./sections/ShortcutsSection"
import { UpdatesSection } from "./sections/UpdatesSection"
import { SETTINGS_LABELS } from "./sections"

const META: Record<SettingsSection, { icon: React.ReactNode; render: () => React.ReactNode }> = {
  general: { icon: <SlidersHorizontal />, render: () => <GeneralSection /> },
  shortcuts: { icon: <Keyboard />, render: () => <ShortcutsSection /> },
  updates: { icon: <Download />, render: () => <UpdatesSection /> },
  about: { icon: <Info />, render: () => <AboutSection /> },
}

export function SettingsPage() {
  const { section } = useParams({ from: "/settings/$section" })
  return (
    <div className="flex h-full">
      <nav aria-label="Settings" className="w-56 shrink-0 border-r border-hairline px-3 pt-7">
        <h1 className="mb-4 px-2.5 text-xl text-fg-1">Settings</h1>
        <div className="flex flex-col gap-0.5">
          {SETTINGS_SECTIONS.map((s) => (
            <Link
              key={s}
              to="/settings/$section"
              params={{ section: s }}
              className={cn(
                "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-sm font-medium transition-colors [&_svg]:size-4",
                s === section ? "bg-active text-fg-1 [&_svg]:text-accent" : "text-fg-2 hover:bg-hover hover:text-fg-1 [&_svg]:text-fg-3",
              )}
            >
              {META[s].icon}
              {SETTINGS_LABELS[s]}
            </Link>
          ))}
        </div>
      </nav>
      <Page className="flex-1">
        <div className="mx-auto max-w-3xl px-10 pt-8 pb-12">{META[section].render()}</div>
      </Page>
    </div>
  )
}
