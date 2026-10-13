import type * as React from "react"
import { Select as S } from "radix-ui"
import { Check, ChevronDown } from "lucide-react"
import { cn } from "@/lib/utils"

export type SelectOption = { value: string; label: React.ReactNode; description?: React.ReactNode }

/** Single-choice select with optional per-option descriptions. */
export function Select({
  value,
  onChange,
  options,
  placeholder,
  className,
  size = "md",
  disabled,
  "aria-label": ariaLabel,
}: {
  value: string | undefined
  onChange: (v: string) => void
  options: SelectOption[]
  placeholder?: string
  className?: string
  size?: "sm" | "md"
  disabled?: boolean
  "aria-label"?: string
}) {
  return (
    <S.Root value={value} onValueChange={onChange} disabled={disabled}>
      <S.Trigger
        aria-label={ariaLabel}
        className={cn(
          "inline-flex min-w-0 items-center justify-between gap-2 rounded-md border border-soft bg-raised text-left text-fg-1 shadow-1",
          "transition-colors duration-[var(--dur-fast)] hover:border-strong focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-subtle",
          "data-[placeholder]:text-fg-3 disabled:opacity-45",
          size === "sm" ? "h-7 px-2 text-xs" : "h-8 px-2.5 text-sm",
          className,
        )}
      >
        <span className="truncate">
          <S.Value placeholder={placeholder} />
        </span>
        <S.Icon>
          <ChevronDown className="size-3.5 shrink-0 text-fg-3" />
        </S.Icon>
      </S.Trigger>
      <S.Portal>
        <S.Content
          position="popper"
          sideOffset={6}
          className="glass z-50 max-h-[min(360px,var(--radix-select-content-available-height))] min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-lg data-[state=open]:animate-pop-in"
        >
          <S.Viewport className="p-1">
            {options.map((o) => (
              <S.Item
                key={o.value}
                value={o.value}
                className={cn(
                  "relative flex cursor-default flex-col rounded-md py-1.5 pr-8 pl-2.5 text-sm text-fg-2 outline-none select-none",
                  "data-[highlighted]:bg-active data-[highlighted]:text-fg-1 data-[state=checked]:text-fg-1",
                )}
              >
                <S.ItemText>{o.label}</S.ItemText>
                {o.description && <span className="mt-0.5 text-2xs text-fg-3">{o.description}</span>}
                <S.ItemIndicator className="absolute top-2 right-2.5">
                  <Check className="size-3.5 text-accent" />
                </S.ItemIndicator>
              </S.Item>
            ))}
          </S.Viewport>
        </S.Content>
      </S.Portal>
    </S.Root>
  )
}
