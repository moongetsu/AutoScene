import type * as React from "react"
import { DropdownMenu as M } from "radix-ui"
import { Check } from "@/components/icons"
import { cn } from "@/lib/utils"

export const DropdownMenu = M.Root
export const DropdownMenuTrigger = M.Trigger
export const DropdownMenuRadioGroup = M.RadioGroup

export function DropdownMenuContent({
  className,
  align = "end",
  sideOffset = 6,
  ...props
}: React.ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content
        align={align}
        sideOffset={sideOffset}
        className={cn(
          "glass-strong z-50 min-w-[200px] rounded-xl p-1.5 data-[state=open]:animate-pop-in focus:outline-none",
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

export function DropdownMenuItem({
  className,
  destructive,
  ...props
}: React.ComponentProps<typeof M.Item> & { destructive?: boolean }) {
  return (
    <M.Item
      className={cn(
        item,
        destructive && "text-danger data-[highlighted]:bg-danger-subtle data-[highlighted]:text-danger [&_svg]:text-danger",
        className,
      )}
      {...props}
    />
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
