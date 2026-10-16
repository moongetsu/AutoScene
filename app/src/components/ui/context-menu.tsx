import type * as React from "react"
import { ContextMenu as M } from "radix-ui"
import { isMac } from "@/lib/platform"
import { cn } from "@/lib/utils"

export const ContextMenu = M.Root

/**
 * Right-clicks inside a dialog, a dropdown or another menu reach the trigger
 * through their portals (React bubbles along the component tree): those
 * aren't for it, so no second menu opens on top.
 */
export function ContextMenuTrigger({ onContextMenu, ...props }: React.ComponentProps<typeof M.Trigger>) {
  return (
    <M.Trigger
      onContextMenu={(e) => {
        onContextMenu?.(e)
        if (!e.currentTarget.contains(e.target as Node)) e.preventDefault()
      }}
      {...props}
    />
  )
}

export function ContextMenuContent({ className, ...props }: React.ComponentProps<typeof M.Content>) {
  return (
    <M.Portal>
      <M.Content
        className={cn(
          "glass-strong z-50 min-w-[200px] rounded-xl border border-white/10 p-1.5 shadow-2xl data-[state=open]:animate-pop-in focus:outline-none",
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

/** `shortcut` as key caps, e.g. ["Ctrl", "K"] — shown as "Ctrl+K" (⌘K on macOS). */
export function ContextMenuItem({
  className,
  destructive,
  shortcut,
  children,
  ...props
}: React.ComponentProps<typeof M.Item> & { destructive?: boolean; shortcut?: string[] }) {
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
      {shortcut && shortcut.length > 0 && (
        <span className="timecode ml-auto pl-4 text-xs tracking-wider text-fg-3">{shortcut.join(isMac ? "" : "+")}</span>
      )}
    </M.Item>
  )
}

export function ContextMenuSeparator() {
  return <M.Separator className="mx-2 my-1.5 h-px bg-hairline" />
}
