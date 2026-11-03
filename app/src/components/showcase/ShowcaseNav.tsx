import * as React from "react"
import {
  Square,
  CursorClick,
  Tag,
  ToggleLeft,
  CircleNotch,
  ChatCircle,
  Image,
  Cards,
  Sidebar,
  Palette,
  VideoCamera,
} from "@phosphor-icons/react"
import { cn } from "@/lib/utils"

export type ShowcaseSectionId =
  | "buttons"
  | "badges"
  | "inputs"
  | "toggles"
  | "indicators"
  | "overlays"
  | "media"
  | "cards"
  | "shell"
  | "tokens"
  | "playground"
  | "record"

interface ShowcaseNavProps {
  activeSection: ShowcaseSectionId
  onSelect: (id: ShowcaseSectionId) => void
}

const NAV_ITEMS: { id: ShowcaseSectionId; label: string; icon: React.ReactNode; group: string }[] = [
  // Primitives
  { id: "buttons", label: "Buttons & Pills", icon: <CursorClick className="size-4" />, group: "Primitives" },
  { id: "badges", label: "Badges & Tags", icon: <Tag className="size-4" />, group: "Primitives" },
  { id: "inputs", label: "Inputs & Search", icon: <Square className="size-4" />, group: "Primitives" },
  { id: "toggles", label: "Toggles & Sliders", icon: <ToggleLeft className="size-4" />, group: "Primitives" },
  { id: "indicators", label: "Indicators & States", icon: <CircleNotch className="size-4" />, group: "Feedback" },
  { id: "overlays", label: "Overlays & Menus", icon: <ChatCircle className="size-4" />, group: "Feedback" },
  { id: "media", label: "Media & Artwork", icon: <Image className="size-4" />, group: "Surfaces" },
  { id: "cards", label: "Composite Cards", icon: <Cards className="size-4" />, group: "Surfaces" },
  { id: "shell", label: "Shell & System", icon: <Sidebar className="size-4" />, group: "Surfaces" },
  { id: "tokens", label: "Design Tokens", icon: <Palette className="size-4" />, group: "Foundation" },
  // Studio & Recorder
  { id: "playground", label: "Live Playground", icon: <CursorClick className="size-4" />, group: "Studio" },
  { id: "record", label: "Animation Recorder", icon: <VideoCamera className="size-4" />, group: "Studio" },
]

export function ShowcaseNav({ activeSection, onSelect }: ShowcaseNavProps) {
  const groups = Array.from(new Set(NAV_ITEMS.map((item) => item.group)))

  return (
    <nav className="w-64 shrink-0 border-r border-white/[0.08] bg-panel p-4 flex flex-col gap-6 select-none overflow-y-auto">
      <div>
        <div className="text-xs font-mono tracking-widest text-accent uppercase px-3 py-1">AutoScene UI</div>
        <h2 className="text-xl font-bold text-fg-1 px-3">Design System</h2>
      </div>

      <div className="flex flex-col gap-5">
        {groups.map((group) => (
          <div key={group} className="flex flex-col gap-1">
            <span className="text-[11px] font-semibold text-fg-3 uppercase tracking-wider px-3 mb-1">
              {group}
            </span>
            {NAV_ITEMS.filter((item) => item.group === group).map((item) => {
              const active = activeSection === item.id
              return (
                <button
                  key={item.id}
                  type="button"
                  onClick={() => onSelect(item.id)}
                  className={cn(
                    "flex items-center gap-3 px-3 py-2 rounded-xl text-sm font-medium transition-all text-left",
                    active
                      ? "bg-accent/15 text-accent shadow-sm"
                      : "text-fg-2 hover:bg-white/[0.04] hover:text-fg-1"
                  )}
                >
                  {item.icon}
                  <span>{item.label}</span>
                </button>
              )
            })}
          </div>
        ))}
      </div>
    </nav>
  )
}
