import { cn } from "@/lib/utils"

function Skeleton({ className, ...props }: React.ComponentProps<"div">) {
  return (
    <div
      data-slot="skeleton"
      className={cn(
        // Visible on dark panels: muted alone is nearly invisible on Panel Ink.
        "animate-pulse rounded-md bg-foreground/[0.08] ring-1 ring-foreground/[0.06]",
        className,
      )}
      {...props}
    />
  )
}

export { Skeleton }
