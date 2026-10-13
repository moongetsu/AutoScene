import type * as React from "react"
import { cn } from "@/lib/utils"

/** Scrollable page body with consistent gutters. */
export function Page({ className, children }: { className?: string; children: React.ReactNode }) {
  return <div className={cn("h-full overflow-y-auto", className)}>{children}</div>
}

export function PageHeader({
  title,
  description,
  actions,
  className,
}: {
  title: React.ReactNode
  description?: React.ReactNode
  actions?: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex items-end justify-between gap-6 px-8 pt-7 pb-5", className)}>
      <div className="min-w-0">
        <h1 className="truncate text-2xl text-fg-1">{title}</h1>
        {description && <p className="mt-1 text-sm text-fg-2">{description}</p>}
      </div>
      {actions && <div className="flex shrink-0 items-center gap-2">{actions}</div>}
    </div>
  )
}
