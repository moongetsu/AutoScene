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
          sideOffset={6}
          className="glass z-50 flex items-center gap-2 rounded-sm px-2 py-1 text-xs text-fg-1 data-[state=delayed-open]:animate-fade-in"
        >
          {label}
          {shortcut && <Kbd keys={shortcut} />}
        </T.Content>
      </T.Portal>
    </T.Root>
  )
}
