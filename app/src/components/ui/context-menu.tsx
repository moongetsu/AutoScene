import type * as React from "react"
import { ContextMenu as M } from "radix-ui"
import { Check } from "@/components/icons"
import { cn } from "@/lib/utils"

export const ContextMenu = M.Root
export const ContextMenuTrigger = M.Trigger
export const ContextMenuGroup = M.Group
export const ContextMenuPortal = M.Portal
export const ContextMenuSub = M.Sub
export const ContextMenuRadioGroup = M.RadioGroup

export function ContextMenuContent({
  className,
  ...props
}: React.ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content
        className={cn(
          "glass-strong z-50 min-w-[200px] rounded-xl p-1.5 shadow-2xl border border-white/10 data-[state=open]:animate-pop-in focus:outline-none",
          className,
        )}
        {...props}
      />
    </M.Portal>
  )
}

const item = cn(
  "flex h-9 cursor-default items-center gap-3 rounded-lg px-3 text-md text-fg-2 outline-none select-none",
  "data-[highlighted]:bg-active data-[highlighted]:text-fg-1 data-[disabled]:opacity-40",
  "[&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-3 data-[highlighted]:[&_svg]:text-fg-2",
)

export function ContextMenuItem({
  className,
  destructive,
  shortcut,
  children,
  ...props
}: React.ComponentProps<typeof M.Item> & { destructive?: boolean; shortcut?: string }) {
  return (
    <M.Item
      className={cn(
        item,
        destructive && "text-danger data-[highlighted]:bg-danger-subtle data-[highlighted]:text-danger [&_svg]:text-danger",
        className,
      )}
      {...props}
    >
      {children}
      {shortcut && <span className="timecode ml-auto pl-4 text-xs text-fg-3 tracking-wider">{shortcut}</span>}
    </M.Item>
  )
}

export function ContextMenuCheckboxItem({
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

export function ContextMenuRadioItem({ className, children, ...props }: React.ComponentProps<typeof M.RadioItem>) {
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

export function ContextMenuSeparator() {
  return <M.Separator className="mx-2 my-1.5 h-px bg-hairline" />
}

export function ContextMenuLabel({ className, ...props }: React.ComponentProps<typeof M.Label>) {
  return <M.Label className={cn("eyebrow px-3 pt-2 pb-1", className)} {...props} />
}
