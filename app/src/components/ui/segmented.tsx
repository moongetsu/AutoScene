import type * as React from "react"
import { ToggleGroup } from "radix-ui"
import { cn } from "@/lib/utils"

type Option<T extends string> = { value: T; label: React.ReactNode }

/** Single-choice segmented control (mode switches, filters). */
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
      className={cn("inline-flex h-8 items-center gap-0.5 rounded-md border border-hairline bg-inset p-0.5", className)}
    >
      {options.map((o) => (
        <ToggleGroup.Item
          key={o.value}
          value={o.value}
          className={cn(
            "inline-flex h-full items-center gap-1.5 rounded-sm px-2.5 text-xs font-medium text-fg-3 transition-colors [&_svg]:size-3.5",
            "hover:text-fg-1 data-[state=on]:bg-raised data-[state=on]:text-fg-1 data-[state=on]:shadow-1",
          )}
        >
          {o.label}
        </ToggleGroup.Item>
      ))}
    </ToggleGroup.Root>
  )
}
