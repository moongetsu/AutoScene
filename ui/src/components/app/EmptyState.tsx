import type { ReactNode } from "react"
import { Button } from "@/components/ui/button"

type EmptyStateProps = {
  title: string
  description?: string
  actionLabel?: string
  onAction?: () => void
  icon?: ReactNode
}

export function EmptyState({
  title,
  description,
  actionLabel,
  onAction,
  icon,
}: EmptyStateProps) {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-4 p-10 text-center">
      {icon ? (
        <div className="flex size-14 items-center justify-center rounded-2xl border border-border/50 bg-card/60 text-primary">
          {icon}
        </div>
      ) : null}
      <div className="max-w-md space-y-2">
        <h2 className="font-heading text-xl">{title}</h2>
        {description ? <p className="text-sm text-muted-foreground">{description}</p> : null}
      </div>
      {actionLabel && onAction ? (
        <Button onClick={onAction}>{actionLabel}</Button>
      ) : null}
    </div>
  )
}
