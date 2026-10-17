import type * as React from "react"
import { DropdownMenu as M } from "radix-ui"
import { Check } from "@/components/icons"
import { cn } from "@/lib/utils"
import { MenuShortcut } from "./context-menu"
import { afterClose, menuItem, menuSurface } from "./menu-base"

export const DropdownMenu = M.Root
export const DropdownMenuTrigger = M.Trigger
export const DropdownMenuRadioGroup = M.RadioGroup

export function DropdownMenuContent({
  className,
  align = "end",
  sideOffset = 6,
  onCloseAutoFocus,
  ...props
}: React.ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content
        align={align}
        sideOffset={sideOffset}
        // Back to the ⋯ button only when the focus would be lost: an entry
        // that opened a field or a dialog keeps it there.
        onCloseAutoFocus={(e) => {
          onCloseAutoFocus?.(e)
          const active = document.activeElement
          if (active !== null && active !== document.body) e.preventDefault()
        }}
        className={cn(menuSurface, "data-[state=open]:animate-pop-in", className)}
        {...props}
      />
    </M.Portal>
  )
}

const item = menuItem

/** `shortcut` as the keys read, e.g. ["Ctrl", "E"] — like the context menu's items. */
export function DropdownMenuItem({
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
        item,
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

/** A choice to tick among several. The menu stays open: one usually ticks more than one. */
export function DropdownMenuCheckboxItem({
  className,
  children,
  onSelect,
  ...props
}: React.ComponentProps<typeof M.CheckboxItem>) {
  return (
    <M.CheckboxItem
      className={cn(item, "relative pl-9 data-[state=checked]:text-fg-1 data-[state=checked]:[&_svg]:text-accent", className)}
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

/** One choice of a `DropdownMenuRadioGroup`. */
export function DropdownMenuRadioItem({ className, children, ...props }: React.ComponentProps<typeof M.RadioItem>) {
  return (
    <M.RadioItem
      className={cn(item, "relative pl-9 data-[state=checked]:text-fg-1 data-[state=checked]:[&_svg]:text-accent", className)}
      {...props}
    >
      <M.ItemIndicator className="absolute left-3 flex items-center">
        <Check weight="bold" />
      </M.ItemIndicator>
      {children}
    </M.RadioItem>
  )
}

export function DropdownMenuSeparator() {
  return <M.Separator className="mx-2 my-1.5 h-px bg-hairline" />
}

export function DropdownMenuLabel({ className, ...props }: React.ComponentProps<typeof M.Label>) {
  return <M.Label className={cn("eyebrow px-3 pt-2 pb-1", className)} {...props} />
}
