import type * as React from "react"
import { cn } from "@/lib/utils"

/** Progress bar with a soft glow. `value` in 0..1; omit (or null) for indeterminate. */
export function Progress({ value, className }: { value?: number | null; className?: string }) {
  const determinate = value != null && Number.isFinite(value)
  return (
    <div
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={determinate ? Math.round(value * 100) : undefined}
      className={cn("relative h-1.5 w-full overflow-hidden rounded-full bg-white/10", className)}
    >
      {determinate ? (
        <div
          className="h-full w-full origin-left rounded-full bg-accent shadow-[0_0_12px_var(--accent)] transition-transform duration-[var(--dur-med)] ease-out"
          style={{ transform: `scaleX(${Math.max(0, Math.min(1, value))})` }}
        />
      ) : (
        <div className="absolute inset-y-0 w-1/3 animate-[indeterminate_1.4s_var(--ease-in-out)_infinite] rounded-full bg-accent shadow-[0_0_12px_var(--accent)]" />
      )}
    </div>
  )
}

/**
 * The same as a ring, filling clockwise from the top; a `size-*` class sizes
 * it. `value` in 0..1; omit (or null) and an arc spins. `grid` is the width it
 * is drawn on, the line 2 of it: a bigger grid draws a finer line.
 */
export function ProgressRing({
  value,
  grid = 16,
  className,
  trackClassName = "text-white/15",
  ...props
}: { value?: number | null; grid?: number; trackClassName?: string } & React.ComponentProps<"svg">) {
  const determinate = value != null && Number.isFinite(value)
  const v = determinate ? Math.max(0, Math.min(1, value)) : 0
  const c = grid / 2
  return (
    <svg
      role="progressbar"
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={determinate ? Math.round(v * 100) : undefined}
      viewBox={`0 0 ${grid} ${grid}`}
      className={cn("size-4 shrink-0 overflow-visible", className)}
      {...props}
    >
      <circle cx={c} cy={c} r={c - 2} fill="none" stroke="currentColor" strokeWidth={2} className={trackClassName} />
      <circle
        cx={c}
        cy={c}
        r={c - 2}
        pathLength={100}
        fill="none"
        stroke="var(--accent)"
        strokeWidth={2}
        strokeLinecap="round"
        strokeDasharray={determinate ? `${Math.max(3, v * 100)} 100` : "25 75"}
        className={cn(
          "origin-center drop-shadow-[0_0_6px_var(--accent)] transition-[stroke-dasharray] duration-[var(--dur-slow)]",
          determinate ? "-rotate-90" : "animate-spin [animation-duration:1.4s]",
        )}
      />
    </svg>
  )
}
