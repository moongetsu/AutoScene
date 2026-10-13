import type * as React from "react"
import { cn } from "@/lib/utils"

export function Input({ className, ...props }: React.ComponentProps<"input">) {
  return (
    <input
      spellCheck={false}
      className={cn(
        "h-8 w-full min-w-0 rounded-md border border-soft bg-inset px-2.5 text-sm text-fg-1 shadow-[inset_0_1px_2px_rgb(0_0_0/0.35)]",
        "transition-[border-color,box-shadow] duration-[var(--dur-fast)] outline-none placeholder:text-fg-3",
        "hover:border-strong focus-visible:border-accent-line focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-accent-subtle",
        "disabled:opacity-45",
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
    <div className={cn("flex flex-col gap-1.5", className)}>
      <label htmlFor={htmlFor} className="text-xs font-medium text-fg-2">
        {label}
      </label>
      {children}
      {hint && <p className="text-2xs text-fg-3">{hint}</p>}
    </div>
  )
}
