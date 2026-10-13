import { cn } from "@/lib/utils"

/** Keyboard key caps. Pass `keys` like ["Ctrl", "K"] for a combo. */
export function Kbd({ keys, className }: { keys: string[]; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-0.5", className)}>
      {keys.map((k) => (
        <kbd
          key={k}
          className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-xs border border-soft bg-inset px-1 font-mono text-2xs leading-none text-fg-3"
        >
          {k}
        </kbd>
      ))}
    </span>
  )
}
