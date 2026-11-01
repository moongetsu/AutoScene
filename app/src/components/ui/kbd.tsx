import { cn } from "@/lib/utils"

/** Keyboard key caps. Pass `keys` like ["Ctrl", "K"] for a combo; `accent` marks keys the person chose. */
export function Kbd({ keys, tone, className }: { keys: string[]; tone?: "accent"; className?: string }) {
  return (
    <span className={cn("inline-flex items-center gap-1", className)}>
      {keys.map((k, i) => (
        <kbd
          // A combo may name a key twice ("Shift ← Shift →").
          key={`${i}:${k}`}
          className={cn(
            "inline-flex h-5 min-w-5 items-center justify-center rounded-[5px] border px-1.5 font-sans text-2xs leading-none font-medium shadow-[inset_0_-1px_0_rgb(0_0_0/0.4)]",
            tone === "accent"
              ? "border-accent/40 border-b-accent/25 bg-accent-subtle text-accent"
              : "border-white/10 border-b-white/5 bg-white/[0.07] text-fg-2",
          )}
        >
          {k}
        </kbd>
      ))}
    </span>
  )
}
