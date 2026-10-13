import type * as React from "react"
import { cn } from "@/lib/utils"

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
    <div className={cn("flex flex-col items-center justify-center px-6 py-12 text-center", className)}>
      {icon && (
        <div className="mb-4 flex size-12 items-center justify-center rounded-lg border border-hairline bg-panel text-fg-3 shadow-1 [&_svg]:size-5">
          {icon}
        </div>
      )}
      <h3 className="text-md font-semibold text-fg-1">{title}</h3>
      {description && <p className="mt-1 max-w-sm text-sm text-fg-2">{description}</p>}
      {actions && <div className="mt-5 flex items-center gap-2">{actions}</div>}
    </div>
  )
}
