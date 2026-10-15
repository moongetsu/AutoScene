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
    <section className="mb-10">
      <div className="mb-3 px-1">
        <h3 className="text-lg font-semibold text-fg-1">{title}</h3>
        {description && <p className="mt-0.5 text-md text-fg-2">{description}</p>}
      </div>
      <div className="divide-y divide-white/[0.06] overflow-hidden rounded-2xl border border-white/[0.07] bg-white/[0.03]">
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
    <div className={cn("flex min-h-16 items-center justify-between gap-6 px-5 py-4", className)}>
      <div className="min-w-0">
        <div className="text-md font-medium text-fg-1">{label}</div>
        {description && <div className="mt-0.5 text-sm text-fg-2">{description}</div>}
      </div>
      {children && <div className="flex shrink-0 items-center gap-2">{children}</div>}
    </div>
  )
}

/** A filesystem path shown as selectable mono text. */
export function PathText({ path }: { path: string }) {
  return (
    <span data-selectable className="block max-w-[440px] truncate font-mono text-sm text-fg-2" title={path}>
      {path}
    </span>
  )
}
