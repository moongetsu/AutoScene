import type * as React from "react"
import { DropdownMenu as M } from "radix-ui"
import { cn } from "@/lib/utils"

export const DropdownMenu = M.Root
export const DropdownMenuTrigger = M.Trigger

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
          "glass z-50 min-w-[180px] rounded-lg p-1 data-[state=open]:animate-pop-in focus:outline-none",
          className,
        )}
        {...props}
      />
    </M.Portal>
  )
}

export function DropdownMenuItem({
  className,
  destructive,
  ...props
}: React.ComponentProps<typeof M.Item> & { destructive?: boolean }) {
  return (
    <M.Item
      className={cn(
        "flex h-8 cursor-default items-center gap-2.5 rounded-md px-2.5 text-sm text-fg-2 outline-none select-none",
        "data-[highlighted]:bg-active data-[highlighted]:text-fg-1 data-[disabled]:opacity-40",
        "[&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-fg-3 data-[highlighted]:[&_svg]:text-fg-2",
        destructive && "text-danger data-[highlighted]:bg-danger-subtle data-[highlighted]:text-danger [&_svg]:text-danger",
        className,
      )}
      {...props}
    />
  )
}

export function DropdownMenuSeparator() {
  return <M.Separator className="my-1 h-px bg-hairline" />
}

export function DropdownMenuLabel({ className, ...props }: React.ComponentProps<typeof M.Label>) {
  return <M.Label className={cn("eyebrow px-2.5 pt-1.5 pb-1", className)} {...props} />
}
