import * as React from "react"
import { Slot } from "radix-ui"
import { cva, type VariantProps } from "class-variance-authority"
import { cn } from "@/lib/utils"

/**
 * Pills. `primary` is the one main action of a screen (warm white, like a
 * player's Play button); `secondary` and `glass` sit on the dark and on
 * pictures; `accent` (gold) is for what is selected or running.
 */
const buttonVariants = cva(
  [
    "inline-flex shrink-0 select-none items-center justify-center gap-2 whitespace-nowrap rounded-full font-medium",
    "transition-[background-color,border-color,color,box-shadow,transform] duration-[var(--dur-fast)] ease-out",
    "disabled:pointer-events-none disabled:opacity-40 active:scale-[0.97]",
    "[&_svg]:pointer-events-none [&_svg]:size-4 [&_svg]:shrink-0",
  ],
  {
    variants: {
      variant: {
        primary:
          "bg-cta font-semibold text-on-cta shadow-[0_10px_30px_-10px_rgb(255_255_255/0.35)] hover:bg-cta-hover",
        accent: "bg-accent font-semibold text-on-accent hover:bg-accent-hover active:bg-accent-press",
        secondary: "border border-soft bg-white/[0.07] text-fg-1 hover:border-strong hover:bg-white/[0.11]",
        glass:
          "border border-white/15 bg-white/[0.12] text-white backdrop-blur-xl backdrop-saturate-150 hover:bg-white/[0.2]",
        ghost: "text-fg-2 hover:bg-hover hover:text-fg-1 active:bg-active",
        danger: "border border-danger/30 bg-danger-subtle text-danger hover:bg-danger/20",
      },
      size: {
        sm: "h-8 px-3.5 text-sm",
        md: "h-9 px-4 text-md",
        lg: "h-11 px-6 text-md [&_svg]:size-[18px]",
        icon: "size-9",
        "icon-sm": "size-8 [&_svg]:size-[15px]",
        "icon-lg": "size-11 [&_svg]:size-5",
      },
    },
    defaultVariants: { variant: "secondary", size: "md" },
  },
)

export type ButtonProps = React.ComponentProps<"button"> &
  VariantProps<typeof buttonVariants> & { asChild?: boolean }

export function Button({ className, variant, size, asChild, type = "button", ...props }: ButtonProps) {
  const Comp = asChild ? Slot.Root : "button"
  return (
    <Comp
      type={asChild ? undefined : type}
      className={cn(buttonVariants({ variant, size }), className)}
      {...props}
    />
  )
}
