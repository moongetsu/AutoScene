import type * as React from "react"
import { Command } from "cmdk"
import { Dialog as D } from "radix-ui"
import { Activity, Download, FileText, Info, Keyboard, Library, PanelLeft, Settings } from "lucide-react"
import { Kbd } from "@/components/ui/kbd"
import { useAppActions } from "@/lib/actions"
import { shortcutKeys } from "@/lib/shortcuts"
import { useUi } from "@/state/ui"

type Item = { id: string; label: string; icon: React.ReactNode; run: () => void; keys?: string[]; hint?: string }

export function CommandPalette() {
  const open = useUi((s) => s.paletteOpen)
  const setOpen = useUi((s) => s.setPaletteOpen)
  const a = useAppActions()

  const go: Item[] = [
    { id: "library", label: "Library", icon: <Library />, run: a.goLibrary, keys: shortcutKeys("library") },
    { id: "settings", label: "Settings", icon: <Settings />, run: () => a.goSettings(), keys: shortcutKeys("settings") },
    { id: "shortcuts", label: "Keyboard shortcuts", icon: <Keyboard />, run: () => a.goSettings("shortcuts") },
    { id: "about", label: "About AutoScene", icon: <Info />, run: () => a.goSettings("about") },
  ]
  const actions: Item[] = [
    { id: "activity", label: "Show activity", icon: <Activity />, run: a.toggleActivity, keys: shortcutKeys("activity") },
    { id: "sidebar", label: "Toggle sidebar", icon: <PanelLeft />, run: a.toggleSidebar, keys: shortcutKeys("sidebar") },
    { id: "updates", label: "Check for updates", icon: <Download />, run: a.checkUpdates },
    { id: "logs", label: "Open logs folder", icon: <FileText />, run: () => void a.openLogs() },
  ]

  const select = (item: Item) => {
    setOpen(false)
    // Let the dialog close before navigating so focus returns cleanly.
    requestAnimationFrame(item.run)
  }

  return (
    <D.Root open={open} onOpenChange={setOpen}>
      <D.Portal>
        <D.Overlay className="fixed inset-0 z-50 bg-black/45 data-[state=open]:animate-fade-in" />
        <D.Content
          className="glass fixed top-[14vh] left-1/2 z-50 w-[min(600px,calc(100vw-32px))] -translate-x-1/2 overflow-hidden rounded-xl focus:outline-none data-[state=open]:animate-pop-in"
          aria-describedby={undefined}
        >
          <D.Title className="sr-only">Command palette</D.Title>
          <Command loop className="flex max-h-[min(460px,64vh)] flex-col">
            <Command.Input
              autoFocus
              placeholder="Type a command or search…"
              className="h-12 w-full shrink-0 border-b border-hairline bg-transparent px-4 text-md text-fg-1 outline-none placeholder:text-fg-3"
            />
            <Command.List className="min-h-0 flex-1 overflow-y-auto p-1.5">
              <Command.Empty className="px-3 py-8 text-center text-sm text-fg-3">No results.</Command.Empty>
              <Group heading="Go to" items={go} onSelect={select} />
              <Group heading="Actions" items={actions} onSelect={select} />
            </Command.List>
          </Command>
        </D.Content>
      </D.Portal>
    </D.Root>
  )
}

function Group({ heading, items, onSelect }: { heading: string; items: Item[]; onSelect: (i: Item) => void }) {
  return (
    <Command.Group
      heading={heading}
      className="[&_[cmdk-group-heading]]:eyebrow [&_[cmdk-group-heading]]:px-2.5 [&_[cmdk-group-heading]]:pt-2 [&_[cmdk-group-heading]]:pb-1"
    >
      {items.map((it) => (
        <Command.Item
          key={it.id}
          value={`${heading} ${it.label}`}
          onSelect={() => onSelect(it)}
          className="flex h-9 cursor-default items-center gap-2.5 rounded-md px-2.5 text-sm text-fg-2 data-[selected=true]:bg-active data-[selected=true]:text-fg-1 [&_svg]:size-4 [&_svg]:text-fg-3"
        >
          {it.icon}
          <span className="flex-1">{it.label}</span>
          {it.keys && <Kbd keys={it.keys} />}
        </Command.Item>
      ))}
    </Command.Group>
  )
}
