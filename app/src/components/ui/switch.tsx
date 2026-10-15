import * as React from "react"
import { Switch as S } from "radix-ui"
import { cn } from "@/lib/utils"

export function Switch({ className, ...props }: React.ComponentProps<typeof S.Root>) {
  return (
    <S.Root
      className={cn(
        "relative inline-flex h-6 w-10 shrink-0 items-center rounded-full bg-white/15 transition-colors duration-[var(--dur-med)]",
        "disabled:opacity-40 data-[state=checked]:bg-accent",
        className,
      )}
      {...props}
    >
      <S.Thumb className="block size-5 translate-x-0.5 rounded-full bg-white shadow-[0_2px_6px_rgb(0_0_0/0.45)] transition-transform duration-[var(--dur-med)] ease-[var(--ease-spring)] data-[state=checked]:translate-x-[18px]" />
    </S.Root>
  )
}
