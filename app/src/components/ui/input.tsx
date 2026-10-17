import type * as React from "react"
import { cn } from "@/lib/utils"

/**
 * A text field. No browser autofill, autocorrect or spell-check underline,
 * and no `title` (the app's tooltips only); right-click opens the app's edit
 * menu (`TextMenu`).
 */
export function Input({ className, ...props }: Omit<React.ComponentProps<"input">, "title">) {
  return (
    <input
      spellCheck={false}
      autoComplete="off"
      autoCorrect="off"
      autoCapitalize="off"
      className={cn(
        "h-10 w-full min-w-0 rounded-lg border border-soft bg-white/[0.05] px-3 text-md text-fg-1",
        "transition-[border-color,box-shadow,background-color] duration-[var(--dur-fast)] outline-none placeholder:text-fg-3",
        "hover:border-strong focus-visible:border-accent-line focus-visible:bg-white/[0.07] focus-visible:ring-4 focus-visible:ring-accent-subtle focus-visible:outline-none",
        "disabled:opacity-40",
        className,
      )}
      {...props}
    />
  )
}

/** Label + control + optional hint, stacked. */
export function Field({
  label,
  hint,
  htmlFor,
  className,
  children,
}: {
  label: React.ReactNode
  hint?: React.ReactNode
  htmlFor?: string
  className?: string
  children: React.ReactNode
}) {
  return (
    <div className={cn("flex flex-col gap-2", className)}>
      <label htmlFor={htmlFor} className="text-sm font-medium text-fg-2">
        {label}
      </label>
      {children}
      {hint && <p className="text-xs text-fg-3">{hint}</p>}
    </div>
  )
}
