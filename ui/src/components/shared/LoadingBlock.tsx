import { cn } from "@/lib/utils"
import { Skeleton } from "@/components/ui/skeleton"

type LoadingBlockProps = {
  /** Short status line shown above the skeleton. */
  label?: string
  /** How many skeleton rows to paint. */
  rows?: number
  className?: string
  /** Taller first row for “rack” / title feel. */
  dense?: boolean
}

/** Visible loading shell — never leave a blank panel while data fetches. */
export function LoadingBlock({
  label = "Loading…",
  rows = 3,
  className,
  dense = false,
}: LoadingBlockProps) {
  return (
    <div
      role="status"
      aria-busy="true"
      aria-live="polite"
      className={cn("min-w-0", className)}
    >
      <p className="mb-3 text-[11px] uppercase tracking-[0.16em] text-muted-foreground">
        {label}
      </p>
      <div className="space-y-2.5">
        {Array.from({ length: Math.max(1, rows) }).map((_, i) => (
          <Skeleton
            key={i}
            className={cn(
              "w-full rounded-lg",
              dense
                ? i === 0
                  ? "h-8 w-36 max-w-full"
                  : "h-[3.25rem]"
                : i === 0
                  ? "h-5 w-2/3 max-w-[12rem]"
                  : "h-12",
            )}
          />
        ))}
      </div>
      <span className="sr-only">{label}</span>
    </div>
  )
}
