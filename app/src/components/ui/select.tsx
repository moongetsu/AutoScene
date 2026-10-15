import type * as React from "react"
import { Select as S } from "radix-ui"
import { CaretDown, Check } from "@/components/icons"
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
          "inline-flex min-w-0 items-center justify-between gap-2 rounded-lg border border-soft bg-white/[0.06] text-left text-fg-1",
          "transition-colors duration-[var(--dur-fast)] hover:border-strong hover:bg-white/[0.09] focus-visible:outline-none focus-visible:ring-4 focus-visible:ring-accent-subtle",
          "data-[placeholder]:text-fg-3 disabled:opacity-40",
          size === "sm" ? "h-8 px-2.5 text-sm" : "h-10 px-3 text-md",
          className,
        )}
      >
        <span className="truncate">
          <S.Value placeholder={placeholder} />
        </span>
        <S.Icon>
          <CaretDown className="size-3.5 shrink-0 text-fg-3" />
        </S.Icon>
      </S.Trigger>
      <S.Portal>
        <S.Content
          position="popper"
          sideOffset={6}
          className="glass-strong z-50 max-h-[min(380px,var(--radix-select-content-available-height))] min-w-[var(--radix-select-trigger-width)] overflow-hidden rounded-xl data-[state=open]:animate-pop-in"
        >
          <S.Viewport className="p-1.5">
            {options.map((o) => (
              <S.Item
                key={o.value}
                value={o.value}
                className={cn(
                  "relative flex cursor-default flex-col rounded-lg py-2 pr-9 pl-3 text-md text-fg-2 outline-none select-none",
                  "data-[highlighted]:bg-active data-[highlighted]:text-fg-1 data-[state=checked]:text-fg-1",
                )}
              >
                <S.ItemText>{o.label}</S.ItemText>
                {o.description && <span className="mt-0.5 text-xs text-fg-3">{o.description}</span>}
                <S.ItemIndicator className="absolute top-2.5 right-3">
                  <Check className="size-4 text-accent" weight="bold" />
                </S.ItemIndicator>
              </S.Item>
            ))}
          </S.Viewport>
        </S.Content>
      </S.Portal>
    </S.Root>
  )
}
