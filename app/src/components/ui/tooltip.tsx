import * as React from "react"
import { Tooltip as T } from "radix-ui"
import { Kbd } from "./kbd"

export const TooltipProvider = T.Provider

type TooltipProps = {
  label: React.ReactNode
  shortcut?: string[]
  side?: "top" | "right" | "bottom" | "left"
  children: React.ReactElement
}

export function Tooltip({ label, shortcut, side = "bottom", children }: TooltipProps) {
  return (
    <T.Root>
      <T.Trigger asChild>{children}</T.Trigger>
      <T.Portal>
        <T.Content
          side={side}
          sideOffset={8}
          className="glass-strong z-50 flex items-center gap-2 rounded-lg px-2.5 py-1.5 text-xs font-medium text-fg-1 data-[state=delayed-open]:animate-fade-in"
        >
          {label}
          {shortcut && <Kbd keys={shortcut} />}
        </T.Content>
      </T.Portal>
    </T.Root>
  )
}
