import type * as React from "react"
import { useEffect, useId, useLayoutEffect, useRef, useState } from "react"
import { createPortal } from "react-dom"
import { DismissableLayer } from "radix-ui/internal"
import {
  ArrowUUpLeft,
  ArrowUUpRight,
  Backspace,
  ClipboardText,
  Copy,
  Scissors,
  SelectionAll,
} from "@/components/icons"
import { MenuShortcut } from "@/components/ui/context-menu"
import { menuItem, menuSeparator, menuSurface } from "@/components/ui/menu-base"
import { copyText } from "@/lib/os"
import { type TextAction, canDo, runTextAction, textShortcut } from "@/lib/text-edit"
import { cn } from "@/lib/utils"
import { type OpenedTextMenu, useTextMenu } from "@/state/text-menu"

type Entry =
  | { kind: "item"; action: TextAction; label: string; icon: React.ReactNode }
  | { kind: "separator" }

const ENTRIES: Entry[] = [
  { kind: "item", action: "undo", label: "Undo", icon: <ArrowUUpLeft /> },
  { kind: "item", action: "redo", label: "Redo", icon: <ArrowUUpRight /> },
  { kind: "separator" },
  { kind: "item", action: "cut", label: "Cut", icon: <Scissors /> },
  { kind: "item", action: "copy", label: "Copy", icon: <Copy /> },
  { kind: "item", action: "paste", label: "Paste", icon: <ClipboardText /> },
  { kind: "item", action: "delete", label: "Delete", icon: <Backspace /> },
  { kind: "separator" },
  { kind: "item", action: "selectAll", label: "Select all", icon: <SelectionAll /> },
]

/**
 * The text fields' menu — Undo, Redo, Cut, Copy, Paste, Delete, Select all —
 * in the app's glass instead of the browser's. It never takes the focus: the
 * field keeps its caret and its selection (and a field that closes or saves
 * when it loses the focus — the go-to field, a rename — stays as it is). It
 * is worked with the mouse, or with keys caught before the field sees them.
 */
export function TextMenu() {
  const opened = useTextMenu((s) => s.opened)
  if (!opened) return null
  return createPortal(<OpenTextMenu opened={opened} />, document.body)
}

function OpenTextMenu({ opened }: { opened: OpenedTextMenu }) {
  const close = useTextMenu((s) => s.close)
  const { field, at, text, clipboard } = opened
  const menu = useRef<HTMLDivElement>(null)
  const [pos, setPos] = useState<{ left: number; top: number } | null>(null)
  const items = ENTRIES.map((e) => (e.kind === "item" ? { ...e, enabled: canDo(text, e.action, clipboard) } : e))
  const enabled = items.flatMap((e, i) => (e.kind === "item" && e.enabled ? [i] : []))
  const [active, setActive] = useState<number | null>(null)
  const id = useId()
  const itemId = (action: TextAction) => `${id}-${action}`
  const activeEntry = active == null ? null : items[active]
  const activeId = activeEntry?.kind === "item" ? itemId(activeEntry.action) : null

  // The focus stays in the field, so the field says which entry is active
  // (as a combobox does): screen readers follow the menu from there. What
  // the field said before (a palette's own list) comes back when it closes.
  useEffect(() => {
    const before = ["aria-controls", "aria-activedescendant"].map((name) => [name, field.getAttribute(name)] as const)
    field.setAttribute("aria-controls", `${id}-menu`)
    return () => {
      for (const [name, value] of before) {
        if (value === null) field.removeAttribute(name)
        else field.setAttribute(name, value)
      }
    }
  }, [field, id])
  useEffect(() => {
    if (activeId) field.setAttribute("aria-activedescendant", activeId)
    else field.removeAttribute("aria-activedescendant")
  }, [field, activeId])

  const run = (action: TextAction) => {
    close()
    void runTextAction(field, text, action, { copy: (t) => copyText(t), clipboard })
  }

  // At the pointer, turned back from the window's edges.
  useLayoutEffect(() => {
    const el = menu.current
    if (!el) return
    const { width, height } = el.getBoundingClientRect()
    const left = at.x + width + 8 > window.innerWidth ? Math.max(8, at.x - width - 2) : at.x + 2
    const top = at.y + height + 8 > window.innerHeight ? Math.max(8, window.innerHeight - height - 8) : at.y + 2
    setPos({ left, top })
  }, [at.x, at.y])

  // Keys go to the menu first (the window's capture runs before any dialog's
  // Escape); a click elsewhere, a scroll, the field or the window losing the
  // focus close it.
  const latest = useRef({ active, enabled, items, run })
  useLayoutEffect(() => {
    latest.current = { active, enabled, items, run }
  })
  useEffect(() => {
    const step = (dir: 1 | -1) => {
      const { active: a, enabled: on } = latest.current
      if (on.length === 0) return
      const at = a == null ? -1 : on.indexOf(a)
      const next = at < 0 ? (dir === 1 ? 0 : on.length - 1) : (at + dir + on.length) % on.length
      setActive(on[next])
    }
    const onKey = (e: KeyboardEvent) => {
      const { active: a, enabled: on, items: list, run: go } = latest.current
      const handled = () => {
        e.preventDefault()
        e.stopPropagation()
      }
      switch (e.key) {
        case "ArrowDown":
          handled()
          return step(1)
        case "ArrowUp":
          handled()
          return step(-1)
        case "Home":
          handled()
          return setActive(on[0] ?? null)
        case "End":
          handled()
          return setActive(on[on.length - 1] ?? null)
        case "Enter":
        case " ": {
          handled()
          const entry = a == null ? null : list[a]
          if (entry?.kind === "item" && entry.enabled) go(entry.action)
          return
        }
        case "Escape":
          handled()
          return close()
        default: {
          // A letter goes to the first entry it starts.
          if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
            const hit = on.find((i) => {
              const entry = list[i]
              return entry.kind === "item" && entry.label.toLowerCase().startsWith(e.key.toLowerCase())
            })
            handled()
            if (hit != null) setActive(hit)
            return
          }
          // Anything else (Tab, a shortcut) is the field's again.
          close()
        }
      }
    }
    const onScroll = (e: Event) => {
      if (!menu.current?.contains(e.target as Node)) close()
    }
    window.addEventListener("keydown", onKey, true)
    window.addEventListener("scroll", onScroll, true)
    window.addEventListener("resize", close)
    window.addEventListener("blur", close)
    window.addEventListener("hashchange", close)
    field.addEventListener("blur", close)
    return () => {
      window.removeEventListener("keydown", onKey, true)
      window.removeEventListener("scroll", onScroll, true)
      window.removeEventListener("resize", close)
      window.removeEventListener("blur", close)
      window.removeEventListener("hashchange", close)
      field.removeEventListener("blur", close)
    }
  }, [field, close])

  // A layer like the app's other menus: on top of any dialog (which then
  // ignores clicks on it and its Escape), the rest of the window waiting
  // until it closes — a click outside closes it, as a native menu's does.
  return (
    <DismissableLayer.Root
      asChild
      disableOutsidePointerEvents
      onPointerDownOutside={close}
      onFocusOutside={(e) => e.preventDefault()}
      onEscapeKeyDown={(e) => {
        e.preventDefault()
        close()
      }}
    >
    <div
      ref={menu}
      id={`${id}-menu`}
      role="menu"
      aria-label="Edit"
      data-state="open"
      // Clicks in the menu never take the focus from the field.
      onMouseDown={(e) => e.preventDefault()}
      onPointerDown={(e) => e.preventDefault()}
      onContextMenu={(e) => e.preventDefault()}
      style={{ left: pos?.left ?? at.x, top: pos?.top ?? at.y, visibility: pos ? "visible" : "hidden" }}
      className={cn(menuSurface, "fixed z-[70] animate-pop-in")}
    >
      {items.map((entry, i) =>
        entry.kind === "separator" ? (
          <div key={i} role="separator" className={menuSeparator} />
        ) : (
          <div
            key={entry.action}
            id={itemId(entry.action)}
            role="menuitem"
            aria-disabled={!entry.enabled || undefined}
            data-disabled={entry.enabled ? undefined : ""}
            data-highlighted={active === i ? "" : undefined}
            onPointerMove={() => entry.enabled && setActive(i)}
            onPointerLeave={() => setActive(null)}
            onClick={() => entry.enabled && run(entry.action)}
            className={menuItem}
          >
            {entry.icon}
            {entry.label}
            <MenuShortcut keys={textShortcut(entry.action)} />
          </div>
        ),
      )}
    </div>
    </DismissableLayer.Root>
  )
}
