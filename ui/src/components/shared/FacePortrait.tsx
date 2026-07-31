import { ScanFace } from "lucide-react"
import { cn } from "@/lib/utils"

type FacePortraitSize = "sm" | "md" | "lg" | "tile"

const SIZE: Record<FacePortraitSize, string> = {
  /** Merge rows, chips */
  sm: "size-11 shrink-0 rounded-lg",
  /** Dossier hero — capped square, never full rail width as height */
  md: "mx-auto h-[min(9.5rem,22vh)] w-[min(9.5rem,22vh)] shrink-0 rounded-2xl sm:h-[min(10.5rem,24vh)] sm:w-[min(10.5rem,24vh)]",
  /** Rare larger preview */
  lg: "mx-auto h-[min(12.5rem,28vh)] w-[min(12.5rem,28vh)] shrink-0 rounded-2xl",
  /** Cast grid tile face plate (width from grid cell) */
  tile: "aspect-square w-full rounded-none",
}

type FacePortraitProps = {
  src?: string | null
  alt?: string
  size?: FacePortraitSize
  className?: string
  imgClassName?: string
  /** cover crops; contain shows full letterboxed thumb */
  fit?: "cover" | "contain"
}

/**
 * Consistent face plate. Never let aspect-square alone fill a wide side rail
 * (that makes a ~420×420 monster in the dossier).
 */
export function FacePortrait({
  src,
  alt = "",
  size = "md",
  className,
  imgClassName,
  fit = "contain",
}: FacePortraitProps) {
  return (
    <div
      className={cn(
        "relative overflow-hidden border border-border/40 bg-[oklch(0.12_0.02_280)]",
        SIZE[size],
        className,
      )}
    >
      {src ? (
        <img
          src={src}
          alt={alt}
          draggable={false}
          className={cn(
            "size-full object-center",
            fit === "contain" ? "object-contain" : "object-cover",
            imgClassName,
          )}
        />
      ) : (
        <div className="flex size-full items-center justify-center text-muted-foreground">
          <ScanFace
            className={cn(
              "opacity-40",
              size === "sm" ? "size-4" : size === "tile" ? "size-10" : "size-8",
            )}
          />
        </div>
      )}
    </div>
  )
}
