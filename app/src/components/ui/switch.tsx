import * as React from "react"
import { Switch as S } from "radix-ui"
import { cn } from "@/lib/utils"

export function Switch({ className, ...props }: React.ComponentProps<typeof S.Root>) {
  return (
    <S.Root
      className={cn(
        "relative inline-flex h-[18px] w-8 shrink-0 cursor-pointer items-center rounded-full border border-soft bg-active transition-colors",
        "disabled:opacity-45 data-[state=checked]:border-accent-line data-[state=checked]:bg-accent",
        className,
      )}
      {...props}
    >
      <S.Thumb className="block size-3 translate-x-[2px] rounded-full bg-fg-1 shadow-1 transition-transform duration-[var(--dur-fast)] data-[state=checked]:translate-x-[16px] data-[state=checked]:bg-on-accent" />
    </S.Root>
  )
}
