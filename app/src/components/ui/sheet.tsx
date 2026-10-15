import * as React from "react"
import { Dialog as D } from "radix-ui"
import { X } from "@/components/icons"
import { cn } from "@/lib/utils"

export const Sheet = D.Root

type SheetContentProps = Omit<React.ComponentProps<typeof D.Content>, "title"> & {
  title: React.ReactNode
  actions?: React.ReactNode
}

/** Right-hand drawer, floating below the top bar (the caption buttons stay usable). */
export function SheetContent({ title, actions, className, children, ...props }: SheetContentProps) {
  return (
    <D.Portal>
      <D.Overlay className="fixed inset-0 top-[var(--topbar-h)] z-40 bg-scrim-soft data-[state=open]:animate-fade-in" />
      <D.Content
        className={cn(
          "glass-strong fixed top-[calc(var(--topbar-h)+8px)] right-3 bottom-3 z-40 flex w-[min(420px,calc(100vw-24px))] flex-col rounded-2xl",
          "focus:outline-none data-[state=open]:animate-sheet-in",
          className,
        )}
        {...props}
      >
        <div className="flex h-14 shrink-0 items-center justify-between gap-2 border-b border-hairline px-5">
          <D.Title className="text-lg font-semibold">{title}</D.Title>
          <D.Description className="sr-only">{title}</D.Description>
          <div className="flex items-center gap-1">
            {actions}
            <D.Close className="flex size-8 items-center justify-center rounded-full bg-white/[0.06] text-fg-2 transition-colors hover:bg-white/[0.12] hover:text-fg-1">
              <X className="size-4" />
              <span className="sr-only">Close</span>
            </D.Close>
          </div>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto">{children}</div>
      </D.Content>
    </D.Portal>
  )
}
