import type * as React from "react"
import { useLayoutEffect, useState } from "react"
import { Slot } from "radix-ui"
import { Area, type AreaKind, type MenuContent, collectArea } from "@/lib/context-areas"

export type { AreaKind, MenuContent }

/**
 * Says what a right-click on its child offers (`menu`). It adds no element
 * of its own: props and ref go to the child. The window's one menu
 * (`AppMenu`) gathers every area the click went through — the innermost
 * first — and adds the app's own entries below them.
 *
 * `tail={false}`: no app entries under this element's (the rail's tiles,
 * the window buttons). `disabled`: nothing to offer right now. `only`: offer
 * it only where the right-clicked element passes (the top bar's empty parts).
 */
export function ContextArea({
  menu,
  kind = "element",
  tail = true,
  disabled = false,
  only,
  children,
  ...props
}: {
  menu: MenuContent
  kind?: AreaKind
  tail?: boolean
  disabled?: boolean
  only?: (target: Element) => boolean
  children: React.ReactElement
} & React.HTMLAttributes<HTMLElement> & { ref?: React.Ref<HTMLElement> }) {
  const [area] = useState(() => new Area(kind, tail))
  // Every render: an open menu shows what the area offers now (a face just
  // selected by the right-click, a pack card taken in).
  useLayoutEffect(() => area.set(menu, kind, tail))
  return (
    <Slot.Root
      {...props}
      onContextMenu={(e: React.MouseEvent<HTMLElement>) => {
        props.onContextMenu?.(e)
        if (disabled) return
        // React also brings right-clicks from portals opened inside the
        // element (its dialog, its ⋯ menu): those weren't on it.
        const target = e.target
        if (!(target instanceof Element) || !e.currentTarget.contains(target)) return
        if (only && !only(target)) return
        collectArea(e.nativeEvent, area)
      }}
    >
      {children}
    </Slot.Root>
  )
}
