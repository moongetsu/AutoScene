import type * as React from "react"
import { useState } from "react"
import { useNavigate, useRouterState } from "@tanstack/react-router"
import { Command } from "cmdk"
import { Dialog as D } from "radix-ui"
import {
  DownloadSimple,
  FileText,
  FilmStrip,
  GearSix,
  Info,
  Keyboard,
  ListNumbers,
  MagnifyingGlass,
  Package,
  Plus,
  Queue,
  SidebarSimple,
  SquaresFour,
  Television,
  User,
  UserFocus,
} from "@/components/icons"
import { Artwork } from "@/components/ui/artwork"
import { Kbd } from "@/components/ui/kbd"
import { useAppActions } from "@/lib/actions"
import { useCast, useMediaList, useProjects } from "@/lib/queries"
import { useShortcutKeys } from "@/lib/shortcuts"
import { useUi } from "@/state/ui"
import { type ProjectEntry, characterEntries, episodeEntries } from "./palette-items"

type Item = {
  id: string
  label: string
  icon: React.ReactNode
  run: () => void
  keys?: string[]
  hint?: string
  /** A picture instead of the icon (projects, episodes). */
  poster?: string | null
  /** A face instead of the icon (characters). */
  face?: string | null
  /** More words it is found by. */
  keywords?: string[]
}

/** Without a search, a few of the open project's characters and episodes. */
const FIRST = 5
/** With one, the first that match (a big series has hundreds of extras). */
const MATCHES = 30

/** Everything the app can do or go to, behind one search field. */
export function CommandPalette() {
  const keys = useShortcutKeys()
  const open = useUi((s) => s.paletteOpen)
  const setOpen = useUi((s) => s.setPaletteOpen)
  const a = useAppActions()
  const { data: projects } = useProjects()
  const path = useRouterState({ select: (s) => s.location.pathname })
  const pid = path.match(/^\/p\/([^/]+)/)?.[1]
  const current = projects?.find((p) => p.id === pid)
  const [query, setQuery] = useState("")

  const here: Item[] = current
    ? [
        { id: "overview", label: "Overview", icon: <SquaresFour />, run: () => a.goProject(current.id) },
        {
          id: "episodes",
          label: current.kind === "movie" ? "Files" : "Episodes",
          icon: <ListNumbers />,
          run: () => a.goProject(current.id, "episodes"),
        },
        { id: "cast", label: "Cast", icon: <UserFocus />, run: () => a.goProject(current.id, "cast") },
        { id: "exports", label: "Exports", icon: <Package />, run: () => a.goProject(current.id, "exports") },
      ]
    : []
  const go: Item[] = [
    { id: "library", label: "Library", icon: <SquaresFour />, run: a.goLibrary, keys: keys("global.library") },
    { id: "settings", label: "Settings", icon: <GearSix />, run: () => a.goSettings(), keys: keys("global.settings") },
    { id: "shortcuts", label: "Keyboard shortcuts", icon: <Keyboard />, run: () => a.goSettings("shortcuts") },
    { id: "about", label: "About AutoScene", icon: <Info />, run: () => a.goSettings("about") },
  ]
  const projectItems: Item[] = (projects ?? []).map((p) => ({
    id: `project-${p.id}`,
    label: p.title,
    icon: p.kind === "movie" ? <FilmStrip /> : <Television />,
    poster: p.posterSmall ?? p.poster,
    hint: p.kind === "movie" ? "Movie" : `${p.mediaCount} ${p.mediaCount === 1 ? "episode" : "episodes"}`,
    run: () => a.goProject(p.id),
  }))
  const actions: Item[] = [
    { id: "new-project", label: "New project…", icon: <Plus />, run: a.newProject, keys: keys("global.newProject") },
    { id: "activity", label: "Show activity", icon: <Queue />, run: a.toggleActivity, keys: keys("global.activity") },
    { id: "sidebar", label: "Show or hide the rail", icon: <SidebarSimple />, run: a.toggleSidebar, keys: keys("global.sidebar") },
    { id: "updates", label: "Check for updates", icon: <DownloadSimple />, run: a.checkUpdates },
    { id: "logs", label: "Open logs folder", icon: <FileText />, run: () => void a.openLogs() },
  ]

  const select = (item: Item) => {
    setOpen(false)
    setQuery("")
    // Let the dialog close before navigating so focus returns cleanly.
    requestAnimationFrame(item.run)
  }

  return (
    <D.Root
      open={open}
      onOpenChange={(o) => {
        setOpen(o)
        if (!o) setQuery("")
      }}
    >
      <D.Portal>
        <D.Overlay className="fixed inset-0 z-50 bg-black/40 backdrop-blur-[2px] data-[state=open]:animate-fade-in" />
        <D.Content
          className="glass-strong fixed top-[14vh] left-1/2 z-50 w-[min(640px,calc(100vw-32px))] -translate-x-1/2 overflow-hidden rounded-[22px] focus:outline-none data-[state=open]:animate-pop-in"
          aria-describedby={undefined}
        >
          <D.Title className="sr-only">Command palette</D.Title>
          <Command loop className="flex max-h-[min(500px,66vh)] flex-col">
            <div className="flex h-16 shrink-0 items-center gap-3 border-b border-white/[0.08] px-5">
              <MagnifyingGlass className="size-5 shrink-0 text-fg-2" weight="bold" />
              <Command.Input
                autoFocus
                autoComplete="off"
                spellCheck={false}
                value={query}
                onValueChange={setQuery}
                placeholder={current ? "Search characters, episodes, projects and commands" : "Search projects, places and commands"}
                className="h-full min-w-0 flex-1 bg-transparent text-lg text-fg-1 outline-none placeholder:text-fg-3"
              />
              <Kbd keys={["Esc"]} />
            </div>
            <Command.List className="min-h-0 flex-1 overflow-y-auto p-2">
              <Command.Empty className="px-3 py-10 text-center text-md text-fg-3">Nothing matches.</Command.Empty>
              {current && <Group heading={current.title} items={here} onSelect={select} />}
              {current && <ProjectFinds pid={current.id} query={query} onSelect={select} />}
              {projectItems.length > 0 && <Group heading="Projects" items={projectItems} onSelect={select} />}
              <Group heading="Actions" items={actions} onSelect={select} />
              <Group heading="Go to" items={go} onSelect={select} />
            </Command.List>
          </Command>
        </D.Content>
      </D.Portal>
    </D.Root>
  )
}

/** The open project's characters and episodes, to jump to. */
function ProjectFinds({
  pid,
  query,
  onSelect,
}: {
  pid: string
  query: string
  onSelect: (i: Item) => void
}) {
  const navigate = useNavigate()
  const { data: cast } = useCast(pid)
  const { data: media } = useMediaList(pid)
  // Every word typed is somewhere in the name or a keyword ("walt white",
  // "s1 e3"); the palette's own matching orders what is left.
  const words = query.trim().toLowerCase().split(/\s+/).filter(Boolean)
  const take = (list: ProjectEntry[]) =>
    words.length > 0
      ? list
          .filter((e) => {
            const hay = [e.label, ...e.keywords].map((w) => w.toLowerCase())
            return words.every((w) => hay.some((h) => h.includes(w)))
          })
          .slice(0, MATCHES)
      : list.slice(0, FIRST)
  const characters: Item[] = take(characterEntries(cast)).map((c) => ({
    id: `character-${c.id}`,
    label: c.label,
    icon: <User />,
    face: c.picture,
    hint: c.hint,
    keywords: c.keywords,
    run: () => void navigate({ to: "/p/$pid/cast/$cid", params: { pid, cid: c.id } }),
  }))
  const episodes: Item[] = take(episodeEntries(media)).map((e) => ({
    id: `episode-${e.id}`,
    label: e.label,
    icon: <FilmStrip />,
    poster: e.picture,
    hint: e.hint,
    keywords: e.keywords,
    run: () => void navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid: e.id } }),
  }))
  return (
    <>
      {characters.length > 0 && <Group heading="Characters" items={characters} onSelect={onSelect} />}
      {episodes.length > 0 && <Group heading="Episodes" items={episodes} onSelect={onSelect} />}
    </>
  )
}

function Group({ heading, items, onSelect }: { heading: string; items: Item[]; onSelect: (i: Item) => void }) {
  return (
    <Command.Group
      heading={heading}
      className="[&_[cmdk-group-heading]]:eyebrow mb-1 [&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:pt-2.5 [&_[cmdk-group-heading]]:pb-1.5"
    >
      {items.map((it) => (
        <Command.Item
          key={it.id}
          value={`${heading} ${it.label} ${it.id}`}
          keywords={it.keywords}
          onSelect={() => onSelect(it)}
          className="group flex h-11 cursor-default items-center gap-3 rounded-xl px-3 text-md text-fg-2 data-[selected=true]:bg-white/[0.09] data-[selected=true]:text-fg-1"
        >
          {it.face !== undefined ? (
            <Artwork
              path={it.face}
              className="size-7 shrink-0 rounded-full picture-edge"
              imgClassName="object-[50%_30%]"
              fallback={<span className="flex size-full items-center justify-center text-fg-3 [&_svg]:size-4">{it.icon}</span>}
            />
          ) : it.poster !== undefined ? (
            <Artwork
              path={it.poster}
              className="h-7 w-12 shrink-0 rounded-md picture-edge"
              fallback={<span className="flex size-full items-center justify-center text-fg-3 [&_svg]:size-4">{it.icon}</span>}
            />
          ) : (
            <span className="flex size-7 shrink-0 items-center justify-center rounded-lg bg-white/[0.06] text-fg-2 group-data-[selected=true]:text-fg-1 [&_svg]:size-4">
              {it.icon}
            </span>
          )}
          <span className="flex-1 truncate">{it.label}</span>
          {it.hint && <span className="text-sm text-fg-3">{it.hint}</span>}
          {it.keys && <Kbd keys={it.keys} />}
        </Command.Item>
      ))}
    </Command.Group>
  )
}
