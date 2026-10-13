import * as React from "react"
import { Dialog as D } from "radix-ui"
import { X } from "lucide-react"
import { cn } from "@/lib/utils"

export const Sheet = D.Root

type SheetContentProps = Omit<React.ComponentProps<typeof D.Content>, "title"> & {
  title: React.ReactNode
  actions?: React.ReactNode
}

/** Right-hand drawer that floats below the title bar. */
export function SheetContent({ title, actions, className, children, ...props }: SheetContentProps) {
  return (
    <D.Portal>
      <D.Overlay className="fixed inset-0 top-[var(--titlebar-h)] z-40 bg-scrim-soft data-[state=open]:animate-fade-in" />
      <D.Content
        className={cn(
          "glass fixed top-[calc(var(--titlebar-h)+8px)] right-2 bottom-2 z-40 flex w-[min(400px,calc(100vw-16px))] flex-col rounded-xl",
          "focus:outline-none data-[state=open]:animate-sheet-in",
          className,
        )}
        {...props}
      >
        <div className="flex h-12 shrink-0 items-center justify-between gap-2 border-b border-hairline px-4">
          <D.Title className="text-md font-semibold">{title}</D.Title>
          <D.Description className="sr-only">{title}</D.Description>
          <div className="flex items-center gap-1">
            {actions}
            <D.Close className="rounded-sm p-1.5 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1">
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
