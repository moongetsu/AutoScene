import type * as React from "react"

/** What a right-click on an area offers: menu items, or a function that gives them (no hooks in it). */
export type MenuContent = React.ReactNode | (() => React.ReactNode)

/**
 * "element": the thing right-clicked — a card, a row, a face: its actions
 * come first and the app's short tail after them. "page": a view's empty
 * space — its actions, then the app's full list.
 */
export type AreaKind = "element" | "page"

/** An area a right-click went through: what it offers, kept live while the menu is open. */
export class Area {
  private content: MenuContent = null
  private readonly listeners = new Set<() => void>()

  constructor(
    public kind: AreaKind,
    public tail: boolean,
  ) {}

  set(content: MenuContent, kind: AreaKind, tail: boolean) {
    this.kind = kind
    this.tail = tail
    if (content === this.content) return
    this.content = content
    for (const l of this.listeners) l()
  }

  readonly get = () => this.content

  readonly subscribe = (listener: () => void) => {
    this.listeners.add(listener)
    return () => void this.listeners.delete(listener)
  }
}

// Keyed by the native event: React makes a new synthetic event per phase
// and per portal, the native one is the right-click itself.
const collected = new WeakMap<Event, Area[]>()

/** Note that a right-click went through `area` (called innermost first). */
export function collectArea(e: Event, area: Area) {
  const list = collected.get(e)
  if (list) list.push(area)
  else collected.set(e, [area])
}

/** The areas a right-click went through, innermost first. */
export function areasOf(e: Event): readonly Area[] {
  return collected.get(e) ?? []
}
