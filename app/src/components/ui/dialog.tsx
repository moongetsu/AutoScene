import * as React from "react"
import { Dialog as D } from "radix-ui"
import { X } from "lucide-react"
import { cn } from "@/lib/utils"

export const Dialog = D.Root
export const DialogTrigger = D.Trigger
export const DialogClose = D.Close

type ContentProps = Omit<React.ComponentProps<typeof D.Content>, "title"> & {
  title: React.ReactNode
  description?: React.ReactNode
  hideClose?: boolean
}

export function DialogContent({ title, description, hideClose, className, children, ...props }: ContentProps) {
  return (
    <D.Portal>
      <D.Overlay className="fixed inset-0 z-50 bg-scrim data-[state=open]:animate-fade-in" />
      <D.Content
        className={cn(
          "glass fixed top-1/2 left-1/2 z-50 w-[min(520px,calc(100vw-32px))] -translate-x-1/2 -translate-y-1/2 rounded-xl p-5",
          "focus:outline-none data-[state=open]:animate-pop-in",
          className,
        )}
        {...props}
      >
        <div className="mb-4 flex items-start justify-between gap-4">
          <div className="min-w-0">
            <D.Title className="text-lg font-semibold tracking-tight">{title}</D.Title>
            {description ? (
              <D.Description className="mt-1 text-sm text-fg-2">{description}</D.Description>
            ) : (
              <D.Description className="sr-only">{title}</D.Description>
            )}
          </div>
          {!hideClose && (
            <D.Close className="-mt-1 -mr-1 rounded-sm p-1.5 text-fg-3 transition-colors hover:bg-hover hover:text-fg-1">
              <X className="size-4" />
              <span className="sr-only">Close</span>
            </D.Close>
          )}
        </div>
        {children}
      </D.Content>
    </D.Portal>
  )
}

export function DialogFooter({ className, ...props }: React.ComponentProps<"div">) {
  return <div className={cn("mt-5 flex items-center justify-end gap-2", className)} {...props} />
}
