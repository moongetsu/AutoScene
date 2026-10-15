import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"

/** Chips: short facts (format, state, counts). `media` sits on pictures. */
const badgeVariants = cva(
  "inline-flex h-6 items-center gap-1.5 whitespace-nowrap rounded-full px-2.5 text-xs font-medium [&_svg]:size-3.5",
  {
    variants: {
      tone: {
        neutral: "bg-white/[0.07] text-fg-2",
        accent: "bg-accent-subtle text-accent",
        danger: "bg-danger-subtle text-danger",
        outline: "border border-soft text-fg-2",
        media: "border border-white/15 bg-black/35 text-white/90 backdrop-blur-md",
      },
    },
    defaultVariants: { tone: "neutral" },
  },
)

export function Badge({
  className,
  tone,
  ...props
}: React.ComponentProps<"span"> & VariantProps<typeof badgeVariants>) {
  return <span className={cn(badgeVariants({ tone }), className)} {...props} />
}
