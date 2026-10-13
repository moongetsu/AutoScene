import { cn } from "@/lib/utils"

/** Thin progress bar. `value` in 0..1; omit (or null) for indeterminate. */
export function Progress({ value, className }: { value?: number | null; className?: string }) {
  const determinate = value != null && Number.isFinite(value)
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={determinate ? Math.round(value * 100) : undefined}
      className={cn("relative h-1 w-full overflow-hidden rounded-full bg-active", className)}
    >
      {determinate ? (
        <div
          className="h-full w-full origin-left rounded-full bg-accent transition-transform duration-[var(--dur-med)] ease-out"
          style={{ transform: `scaleX(${Math.max(0, Math.min(1, value))})` }}
        />
      ) : (
        <div className="absolute inset-y-0 w-1/3 animate-[indeterminate_1.4s_var(--ease-in-out)_infinite] rounded-full bg-accent" />
      )}
    </div>
  )
}
