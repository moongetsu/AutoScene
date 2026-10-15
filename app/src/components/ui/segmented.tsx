import type * as React from "react"
import { Link } from "@tanstack/react-router"
import { ToggleGroup } from "radix-ui"
import { cn } from "@/lib/utils"

const track = "inline-flex h-9 items-center gap-0.5 rounded-full border border-white/10 bg-white/[0.06] p-1 backdrop-blur-xl"
const item = cn(
  "inline-flex h-full items-center gap-2 rounded-full px-3.5 text-sm font-medium text-fg-2 transition-[color,background-color] duration-[var(--dur-fast)] [&_svg]:size-4",
  "hover:text-fg-1 data-[state=on]:bg-white/[0.14] data-[state=on]:text-fg-1 data-[state=on]:shadow-1",
)

type Option<T extends string> = { value: T; label: React.ReactNode }

/** Single-choice pill switch (modes, filters). */
export function Segmented<T extends string>({
  value,
  onChange,
  options,
  className,
  "aria-label": ariaLabel,
}: {
  value: T
  onChange: (v: T) => void
  options: Option<T>[]
  className?: string
  "aria-label": string
}) {
  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      aria-label={ariaLabel}
      onValueChange={(v) => v && onChange(v as T)}
      className={cn(track, className)}
    >
      {options.map((o) => (
        <ToggleGroup.Item key={o.value} value={o.value} className={item}>
          {o.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  )
}

/** The same pill for moving between a view's sections (links). */
export function SegmentedNav({
  items,
  className,
  "aria-label": ariaLabel,
}: {
  items: { to: string; label: React.ReactNode; icon?: React.ReactNode; active: boolean }[]
  className?: string
  "aria-label": string
}) {
  return (
    <nav aria-label={ariaLabel} className={cn(track, className)}>
      {items.map((i) => (
        <Link
          key={i.to}
          to={i.to}
          aria-current={i.active ? "page" : undefined}
          data-state={i.active ? "on" : "off"}
          className={item}
        >
          {i.icon}
          {i.label}
        </Link>
      ))}
    </nav>
  )
}
