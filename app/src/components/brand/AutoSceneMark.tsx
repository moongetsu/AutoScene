import { cn } from "@/lib/utils"

type AutoSceneMarkProps = {
  className?: string
  /** Accessible name; omit for decorative */
  title?: string
}

/** Shared brand mark — same geometry as boot splash SVG. */
export function AutoSceneMark({ className, title }: AutoSceneMarkProps) {
  return (
    <svg
      viewBox="0 0 48 48"
      preserveAspectRatio="xMidYMid meet"
      className={cn("block h-full w-full shrink-0", className)}
      role={title ? "img" : "presentation"}
      aria-hidden={title ? undefined : true}
      aria-label={title}
    >
      <rect
        x="1"
        y="1"
        width="46"
        height="46"
        rx="10"
        fill="#1C1A17"
        stroke="rgba(242,237,230,0.12)"
      />
      <path
        d="M14 32 L24 12 L34 32"
        fill="none"
        stroke="#C4A35A"
        strokeWidth="2.5"
        strokeLinecap="round"
        strokeLinejoin="round"
      />
      <circle cx="24" cy="28" r="3" fill="#C4A35A" />
    </svg>
  )
}
