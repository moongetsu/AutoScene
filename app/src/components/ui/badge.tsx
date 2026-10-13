import * as React from "react"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"

const badgeVariants = cva(
  "inline-flex h-5 items-center gap-1 whitespace-nowrap rounded-xs px-1.5 text-2xs font-medium [&_svg]:size-3",
  {
    variants: {
      tone: {
        neutral: "bg-active text-fg-2",
        accent: "bg-accent-subtle text-accent",
        danger: "bg-danger-subtle text-danger",
        outline: "border border-soft text-fg-2",
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
