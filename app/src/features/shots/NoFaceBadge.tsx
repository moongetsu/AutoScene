import { UserCircleDashed } from "@/components/icons"
import { cn } from "@/lib/utils"

/** A shot where the character's face doesn't show: their body does (turned away, too small). */
export function NoFaceBadge({ className }: { className?: string }) {
  return (
    <span
      title="Their face doesn't show here: they were found by their body"
      className={cn(
        "inline-flex items-center gap-1 rounded-full bg-black/65 px-2 text-[11px] leading-5 font-medium text-white/90 backdrop-blur-md",
        className,
      )}
    >
      <UserCircleDashed weight="bold" className="size-3" aria-hidden />
      No face
    </span>
  )
}
