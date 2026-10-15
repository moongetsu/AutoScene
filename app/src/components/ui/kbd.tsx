import { cn } from "@/lib/utils"

/** Keyboard key caps. Pass `keys` like ["Ctrl", "K"] for a combo. */
export function Kbd({ keys, className }: { keys: string[]; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1", className)}>
      {keys.map((k) => (
        <kbd
          key={k}
          className="inline-flex h-5 min-w-5 items-center justify-center rounded-[5px] border border-white/10 border-b-white/5 bg-white/[0.07] px-1.5 font-sans text-2xs leading-none font-medium text-fg-2 shadow-[inset_0_-1px_0_rgb(0_0_0/0.4)]"
        >
          {k}
        </kbd>
      ))}
    </span>
  )
}
