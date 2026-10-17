import * as React from "react"
import { useRef, useState } from "react"
import { Tooltip as T } from "radix-ui"
import { cn } from "@/lib/utils"
import { Kbd } from "./kbd"

export const TooltipProvider = T.Provider

type Side = "top" | "right" | "bottom" | "left"

type TooltipProps = {
  label: React.ReactNode
  /** A second, quieter line: what a click does, where it comes from. */
  hint?: React.ReactNode
  shortcut?: string[]
  side?: Side
  /** How long the pointer rests before it shows, in ms (the provider's otherwise). */
  delay?: number
  /** Off: it doesn't show (the children stay as they are). */
  disabled?: boolean
  /** The tooltip's own class. */
  className?: string
  children: React.ReactElement
} & Omit<React.HTMLAttributes<HTMLElement>, "title" | "children" | "className">

/**
 * The app's tooltip — never the browser's (`title` is not used anywhere).
 * Other props (a right-click's handler from an area around it) go to the
 * element it is about.
 */
export function Tooltip({ label, hint, shortcut, side = "bottom", delay, disabled, className, children, ...rest }: TooltipProps) {
  const [open, setOpen] = useState(false)
  return (
    <T.Root delayDuration={delay} open={open && !disabled} onOpenChange={(o) => setOpen(o && !disabled)}>
      <T.Trigger asChild {...rest}>
        {children}
      </T.Trigger>
      <TipContent label={label} hint={hint} shortcut={shortcut} side={side} className={className} />
    </T.Root>
  )
}

function TipContent({
  label,
  hint,
  shortcut,
  side,
  className,
}: {
  label: React.ReactNode
  hint?: React.ReactNode
  shortcut?: string[]
  side: Side
  className?: string
}) {
  return (
    <T.Portal>
      <T.Content
        side={side}
        sideOffset={8}
        collisionPadding={8}
        className={cn(
          "glass-strong z-50 max-w-[min(520px,calc(100vw-24px))] rounded-lg px-2.5 py-1.5 text-xs font-medium text-fg-1 [overflow-wrap:anywhere] data-[state=delayed-open]:animate-fade-in",
          className,
        )}
      >
        <span className="flex items-center gap-2">
          <span>{label}</span>
          {shortcut && <Kbd keys={shortcut} />}
        </span>
        {hint && <span className="mt-0.5 block text-2xs font-normal text-fg-3">{hint}</span>}
      </T.Content>
    </T.Portal>
  )
}

/**
 * Text cut off with an ellipsis where it doesn't fit; all of it shows in a
 * tooltip — only when it is cut. `label` replaces the text in the tooltip
 * (a full path behind a short name).
 */
export function TruncatedText({
  children,
  label,
  side = "top",
  className,
  ...rest
}: {
  children: React.ReactNode
  label?: React.ReactNode
  side?: Side
  className?: string
} & Omit<React.HTMLAttributes<HTMLSpanElement>, "title" | "children">) {
  const ref = useRef<HTMLSpanElement>(null)
  const [open, setOpen] = useState(false)
  return (
    <T.Root
      open={open}
      onOpenChange={(o) => {
        const el = ref.current
        setOpen(o && el != null && el.scrollWidth > el.clientWidth)
      }}
    >
      <T.Trigger asChild>
        <span ref={ref} className={cn("block truncate", className)} {...rest}>
          {children}
        </span>
      </T.Trigger>
      <TipContent label={label ?? children} side={side} />
    </T.Root>
  )
}
