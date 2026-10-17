import type * as React from "react"
import { Fragment, isValidElement, useEffect, useRef, useState, useSyncExternalStore } from "react"
import { useCanGoBack } from "@tanstack/react-router"
import { ContextMenu as M } from "radix-ui"
import {
  ArrowCounterClockwise,
  ArrowLeft,
  Copy,
  GearSix,
  Keyboard,
  MagnifyingGlass,
  Plus,
  Queue,
  SidebarSimple,
  SquaresFour,
} from "@/components/icons"
import {
  ContextMenuContent,
  ContextMenuItem,
  ContextMenuLabel,
  ContextMenuSection,
  ContextMenuSeparator,
} from "@/components/ui/context-menu"
import { useAppActions } from "@/lib/actions"
import { type Area, type MenuContent, areasOf } from "@/lib/context-areas"
import { layerOpen } from "@/lib/layers"
import { copyText } from "@/lib/os"
import { modKey } from "@/lib/platform"
import { shortcutKeys } from "@/lib/shortcuts"
import { textFieldAt } from "@/lib/text-edit"
import { useTextMenu } from "@/state/text-menu"
import { useUi } from "@/state/ui"

type Opened = {
  /** Text selected where the click was: "Copy" comes first. */
  copy: string | null
  sections: readonly Area[]
  /** The app's entries under the rest: a short tail under an element's, all of them on a page's empty space. */
  tail: "short" | "full" | null
}

/**
 * The window's one context menu. A right-click gathers what every area it
 * went through offers (`ContextArea`, innermost first): the element's own
 * actions come first, the app's after them — the short tail (Command
 * palette, Settings) under an element, the full list on a page's empty
 * space. In a text field it is the text menu (`TextMenu`) instead; in a
 * dialog only what the dialog's areas offer, without the app's entries.
 * The browser's own menu never shows (while developing, Shift+right-click
 * still opens it, for Inspect).
 */
export function AppMenu({ children }: { children: React.ReactElement }) {
  const [opened, setOpened] = useState<Opened | null>(null)
  const lastInput = useRef<"pointer" | "key">("pointer")
  /** Where the focus goes back to when the menu closes (keyboard and dialogs only). */
  const restore = useRef<HTMLElement | null>(null)

  useEffect(() => {
    const onPointer = () => {
      lastInput.current = "pointer"
    }
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "ContextMenu" || (e.key === "F10" && e.shiftKey)) lastInput.current = "key"
    }
    const onContextMenu = (e: MouseEvent) => {
      if (!(import.meta.env.DEV && e.shiftKey)) e.preventDefault()
    }
    window.addEventListener("pointerdown", onPointer, true)
    window.addEventListener("keydown", onKey, true)
    window.addEventListener("contextmenu", onContextMenu)
    return () => {
      window.removeEventListener("pointerdown", onPointer, true)
      window.removeEventListener("keydown", onKey, true)
      window.removeEventListener("contextmenu", onContextMenu)
    }
  }, [])

  return (
    // Closed, it forgets the right-click: a touch long-press (no right-click)
    // opens it with the app's entries, not the last element's.
    <M.Root onOpenChange={(open) => !open && setOpened(null)}>
      <M.Trigger
        asChild
        onContextMenuCapture={(e) => {
          if (import.meta.env.DEV && e.shiftKey) {
            e.stopPropagation()
            return
          }
          const field = textFieldAt(e.target)
          if (!field) return
          e.preventDefault()
          e.stopPropagation()
          useTextMenu.getState().open(field, { x: e.clientX, y: e.clientY })
        }}
        onContextMenu={(e) => {
          const target = e.target as Node
          // Dialogs, sheets and menus sit in portals: React brings their
          // right-clicks here, but they aren't the window's.
          const inWindow = e.currentTarget.contains(target)
          const areas = areasOf(e.nativeEvent)
          const copy = selectedTextAt(target)
          // There, without the app's entries, only what is offered right now
          // — or no menu at all rather than an empty one.
          if (!inWindow && !copy && !areas.some((a) => offers(a.get()))) {
            e.preventDefault()
            return
          }
          const elements = areas.filter((a) => a.kind === "element")
          const sections = elements.length > 0 ? elements : areas.filter((a) => a.kind === "page").slice(0, 1)
          const tail = !inWindow ? null : elements.length > 0 ? (elements[0].tail ? "short" : null) : "full"
          const active = document.activeElement
          restore.current =
            (lastInput.current === "key" || !inWindow) && active instanceof HTMLElement && active !== document.body ? active : null
          setOpened({ copy, sections, tail })
        }}
      >
        {children}
      </M.Trigger>
      <ContextMenuContent
        onCloseAutoFocus={(e) => {
          // Not Radix's way (it would select a field's text, and pop the
          // tooltip of the tile right-clicked): back to where the keyboard
          // was — unless an entry took the focus itself (a dialog, the
          // go-to field, a name to edit), which runs before this does.
          e.preventDefault()
          const el = restore.current
          restore.current = null
          const active = document.activeElement
          const dropped = active === null || active === document.body
          if (el?.isConnected && dropped && !layerOpen()) el.focus({ preventScroll: true })
        }}
      >
        <MenuBody opened={opened} />
      </ContextMenuContent>
    </M.Root>
  )
}

function MenuBody({ opened }: { opened: Opened | null }) {
  // A touch long-press opens the menu without a right-click: the app's entries.
  if (!opened) {
    return (
      <ContextMenuSection>
        <FullAppItems />
      </ContextMenuSection>
    )
  }
  return (
    <>
      {opened.copy && (
        <ContextMenuSection>
          <ContextMenuItem onSelect={() => void copyText(opened.copy ?? "")} shortcut={[modKey, "C"]}>
            <Copy /> Copy
          </ContextMenuItem>
        </ContextMenuSection>
      )}
      {opened.sections.map((area, i) => (
        <ContextMenuSection key={i}>
          <AreaItems area={area} />
        </ContextMenuSection>
      ))}
      {opened.tail && (
        <ContextMenuSection>{opened.tail === "short" ? <ShortAppItems /> : <FullAppItems />}</ContextMenuSection>
      )}
    </>
  )
}

/**
 * Whether `content` offers anything right now: lists of `cond && <Item>`
 * can come out empty, and a separator or a label alone is no entry.
 */
function offers(content: MenuContent): boolean {
  const entries = (node: React.ReactNode): boolean => {
    if (node == null || typeof node === "boolean" || node === "") return false
    if (Array.isArray(node)) return node.some(entries)
    if (!isValidElement(node)) return true
    if (node.type === Fragment) return entries((node.props as { children?: React.ReactNode }).children)
    return node.type !== ContextMenuSeparator && node.type !== ContextMenuLabel
  }
  return entries(typeof content === "function" ? content() : content)
}

function AreaItems({ area }: { area: Area }) {
  const content = useSyncExternalStore(area.subscribe, area.get)
  return <>{typeof content === "function" ? content() : content}</>
}

/** Under an element's own actions. */
export function ShortAppItems() {
  const actions = useAppActions()
  return (
    <>
      <ContextMenuItem onSelect={actions.openPalette} shortcut={shortcutKeys("palette")}>
        <MagnifyingGlass /> Command palette…
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => actions.goSettings("general")} shortcut={shortcutKeys("settings")}>
        <GearSix /> Settings
      </ContextMenuItem>
    </>
  )
}

/** On a page's empty space, under the page's own actions. */
export function FullAppItems() {
  const actions = useAppActions()
  const canGoBack = useCanGoBack()
  const railHidden = useUi((s) => s.sidebarCollapsed)
  return (
    <>
      <ContextMenuItem disabled={!canGoBack} onSelect={() => window.history.back()}>
        <ArrowLeft /> Back
      </ContextMenuItem>
      <ContextMenuItem onSelect={actions.openPalette} shortcut={shortcutKeys("palette")}>
        <MagnifyingGlass /> Command palette…
      </ContextMenuItem>
      <ContextMenuItem onSelect={actions.newProject} shortcut={shortcutKeys("newProject")}>
        <Plus /> New project…
      </ContextMenuItem>
      <ContextMenuItem onSelect={actions.goLibrary} shortcut={shortcutKeys("library")}>
        <SquaresFour /> Library
      </ContextMenuItem>
      <ContextMenuItem onSelect={actions.toggleActivity} shortcut={shortcutKeys("activity")}>
        <Queue /> Activity
      </ContextMenuItem>
      <ContextMenuItem onSelect={actions.toggleSidebar} shortcut={shortcutKeys("sidebar")}>
        <SidebarSimple /> {railHidden ? "Show the rail" : "Hide the rail"}
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => actions.goSettings("shortcuts")}>
        <Keyboard /> Keyboard shortcuts
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => actions.goSettings("general")} shortcut={shortcutKeys("settings")}>
        <GearSix /> Settings
      </ContextMenuItem>
      <ContextMenuItem onSelect={() => window.location.reload()}>
        <ArrowCounterClockwise /> Reload window
      </ContextMenuItem>
    </>
  )
}

/** Text selected in a selectable block under the pointer (a path, an error), if any. */
function selectedTextAt(target: Node): string | null {
  const selection = window.getSelection()
  const text = selection?.toString() ?? ""
  if (!selection || !text || selection.rangeCount === 0) return null
  const el = target instanceof Element ? target : target.parentElement
  const block = el?.closest("[data-selectable], code, pre")
  if (!block) return null
  return selection.getRangeAt(0).intersectsNode(block) ? text : null
}
