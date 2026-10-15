import type * as React from "react"
import { cn } from "@/lib/utils"

/** A view with nothing in it yet: a lit icon, a title in the display face, what to do. */
export function EmptyState({
  icon,
  title,
  description,
  actions,
  className,
}: {
  icon?: React.ReactNode
  title: React.ReactNode
  description?: React.ReactNode
  actions?: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex flex-col items-center justify-center px-6 py-14 text-center", className)}>
      {icon && (
        <div className="relative mb-6 flex size-16 items-center justify-center">
          <div
            aria-hidden
            className="absolute -inset-8 rounded-full bg-[radial-gradient(circle,rgb(224_180_92/0.16),transparent_65%)]"
          />
          <div className="relative flex size-16 items-center justify-center rounded-2xl border border-white/10 bg-white/[0.06] text-fg-1 shadow-2 [&_svg]:size-7">
            {icon}
          </div>
        </div>
      )}
      <h3 className="display text-3xl text-fg-1">{title}</h3>
      {description && <p className="mt-2 max-w-md text-md text-fg-2">{description}</p>}
      {actions && <div className="mt-7 flex items-center gap-3">{actions}</div>}
    </div>
  )
}
