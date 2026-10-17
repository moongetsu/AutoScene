import type * as React from "react"
import { ContextMenu as M } from "radix-ui"
import { Check } from "@/components/icons"
import { isMac } from "@/lib/platform"
import { cn } from "@/lib/utils"
import { afterClose, menuItem, menuSeparator, menuSurface } from "./menu-base"

// There is one context menu, the window's (`components/shell/AppMenu`): an
// element says what its right-click offers with `ContextArea`, and these are
// the pieces those offers are made of.

/** `keys` as the shortcut reads, e.g. ["Ctrl", "K"] → "Ctrl+K" (⌘K on macOS). */
export function MenuShortcut({ keys }: { keys?: string[] }) {
  if (!keys || keys.length === 0) return null
  return <span className="timecode ml-auto pl-4 text-xs tracking-wider text-fg-3">{keys.join(isMac ? "" : "+")}</span>
}

export function ContextMenuContent({ className, ...props }: React.ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content
        collisionPadding={8}
        className={cn(
          menuSurface,
          "max-h-[var(--radix-context-menu-content-available-height)] origin-[var(--radix-context-menu-content-transform-origin)] overflow-y-auto data-[state=open]:animate-pop-in",
          className,
        )}
        {...props}
      />
    </M.Portal>
  )
}

/** `shortcut` as key caps, e.g. ["Ctrl", "K"] — shown as "Ctrl+K" (⌘K on macOS). */
export function ContextMenuItem({
  className,
  destructive,
  shortcut,
  children,
  onSelect,
  ...props
}: React.ComponentProps<typeof M.Item> & { destructive?: boolean; shortcut?: string[] }) {
  return (
    <M.Item
      className={cn(
        menuItem,
        destructive && "text-danger data-[highlighted]:bg-danger-subtle data-[highlighted]:text-danger [&_svg]:text-danger",
        className,
      )}
      onSelect={afterClose(onSelect)}
      {...props}
    >
      {children}
      <MenuShortcut keys={shortcut} />
    </M.Item>
  )
}

/** A choice to tick. The menu stays open: one usually ticks more than one. */
export function ContextMenuCheckboxItem({
  className,
  children,
  onSelect,
  ...props
}: React.ComponentProps<typeof M.CheckboxItem>) {
  return (
    <M.CheckboxItem
      className={cn(menuItem, "relative pl-9 data-[state=checked]:text-fg-1 data-[state=checked]:[&_svg]:text-accent", className)}
      onSelect={(e) => {
        e.preventDefault()
        onSelect?.(e)
      }}
      {...props}
    >
      <M.ItemIndicator className="absolute left-3 flex items-center">
        <Check weight="bold" />
      </M.ItemIndicator>
      {children}
    </M.CheckboxItem>
  )
}

export const ContextMenuRadioGroup = M.RadioGroup

/** One choice of a `ContextMenuRadioGroup`. */
export function ContextMenuRadioItem({ className, children, ...props }: React.ComponentProps<typeof M.RadioItem>) {
  return (
    <M.RadioItem
      className={cn(menuItem, "relative pl-9 data-[state=checked]:text-fg-1 data-[state=checked]:[&_svg]:text-accent", className)}
      {...props}
    >
      <M.ItemIndicator className="absolute left-3 flex items-center">
        <Check weight="bold" />
      </M.ItemIndicator>
      {children}
    </M.RadioItem>
  )
}

export function ContextMenuSeparator() {
  return <M.Separator className={menuSeparator} />
}

export function ContextMenuLabel({ className, ...props }: React.ComponentProps<typeof M.Label>) {
  return <M.Label className={cn("eyebrow px-3 pt-2 pb-1", className)} {...props} />
}

/**
 * One section of the menu (an element's, the page's, the app's). Sections
 * are set apart by a hairline, drawn only between sections with something
 * in them; separators at a section's edges, or twice in a row, don't show
 * (`styles/index.css`) — conditional items need no bookkeeping.
 */
export function ContextMenuSection({ children }: { children: React.ReactNode }) {
  return <M.Group data-menu-section="">{children}</M.Group>
}
