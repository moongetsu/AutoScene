import type * as React from "react"
import { useState } from "react"
import { Link } from "@tanstack/react-router"
import { HoverCard, ToggleGroup } from "radix-ui"
import { cn } from "@/lib/utils"

const track = "inline-flex h-9 items-center gap-0.5 rounded-full border border-white/10 bg-white/[0.06] p-1 backdrop-blur-xl"
const item = cn(
  "inline-flex h-full items-center gap-2 rounded-full px-3.5 text-sm font-medium text-fg-2 transition-[color,background-color] duration-[var(--dur-fast)] [&_svg]:size-4",
  "hover:text-fg-1 data-[state=on]:bg-white/[0.14] data-[state=on]:text-fg-1 data-[state=on]:shadow-1",
  "disabled:pointer-events-none disabled:opacity-40",
)

type Option<T extends string> = {
  value: T
  label: React.ReactNode
  /** Shown in a card while the pointer rests on the option (examples). */
  preview?: React.ReactNode
  disabled?: boolean
}

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
  // One preview at a time: the focused option's (after a click) must not
  // stay open beside the one the pointer moved on to.
  const [peek, setPeek] = useState<T | null>(null)
  return (
    <ToggleGroup.Root
      type="single"
      value={value}
      aria-label={ariaLabel}
      onValueChange={(v) => v && onChange(v as T)}
      className={cn(track, className)}
    >
      {options.map((o) => {
        const button = (
          <ToggleGroup.Item key={o.value} value={o.value} disabled={o.disabled} className={item}>
            {o.label}
          </ToggleGroup.Item>
        )
        if (!o.preview) return button
        return (
          <Preview
            key={o.value}
            content={o.preview}
            open={peek === o.value}
            onOpenChange={(open) => setPeek((p) => (open ? o.value : p === o.value ? null : p))}
          >
            {button}
          </Preview>
        )
      })}
    </ToggleGroup.Root>
  )
}

/**
 * A card under an option while the pointer rests on it. Disabled options
 * show theirs too (it says why), through a wrapper that takes the hover.
 * Only to look at: it never takes a click or the focus from what it covers
 * (in a dialog, Radix would otherwise let it take the pointer).
 */
function Preview({
  content,
  open,
  onOpenChange,
  children,
}: {
  content: React.ReactNode
  open: boolean
  onOpenChange: (open: boolean) => void
  children: React.ReactElement
}) {
  return (
    <HoverCard.Root openDelay={250} closeDelay={80} open={open} onOpenChange={onOpenChange}>
      <HoverCard.Trigger asChild>
        <span className="inline-flex h-full">{children}</span>
      </HoverCard.Trigger>
      <HoverCard.Portal>
        <HoverCard.Content
          side="bottom"
          sideOffset={10}
          collisionPadding={12}
          style={{ pointerEvents: "none" }}
          className="glass-strong z-50 w-[360px] max-w-[calc(100vw-24px)] rounded-2xl p-3 data-[state=open]:animate-fade-in"
        >
          {content}
        </HoverCard.Content>
      </HoverCard.Portal>
    </HoverCard.Root>
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
