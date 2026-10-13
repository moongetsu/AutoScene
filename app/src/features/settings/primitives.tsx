import type * as React from "react"
import { cn } from "@/lib/utils"

export function SettingsGroup({
  title,
  description,
  children,
}: {
  title: string
  description?: string
  children: React.ReactNode
}) {
  return (
    <section className="mb-8">
      <div className="mb-3">
        <h2 className="text-md font-semibold text-fg-1">{title}</h2>
        {description && <p className="mt-0.5 text-sm text-fg-2">{description}</p>}
      </div>
      <div className="divide-y divide-hairline overflow-hidden rounded-lg border border-hairline bg-panel shadow-1">
        {children}
      </div>
    </section>
  )
}

export function SettingRow({
  label,
  description,
  children,
  className,
}: {
  label: React.ReactNode
  description?: React.ReactNode
  children?: React.ReactNode
  className?: string
}) {
  return (
    <div className={cn("flex min-h-14 items-center justify-between gap-6 px-4 py-3", className)}>
      <div className="min-w-0">
        <div className="text-sm font-medium text-fg-1">{label}</div>
        {description && <div className="mt-0.5 text-xs text-fg-2">{description}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** A filesystem path shown as selectable mono text. */
export function PathText({ path }: { path: string }) {
  return (
    <span data-selectable className="block max-w-[440px] truncate font-mono text-xs text-fg-2" title={path}>
      {path}
    </span>
  )
}
