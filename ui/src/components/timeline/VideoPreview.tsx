import { useEffect, useRef } from "react"
import { cn } from "@/lib/utils"

type VideoPreviewProps = {
  src: string
  currentTime?: number
  onTimeUpdate?: (time: number) => void
  className?: string
}

export function VideoPreview({ src, currentTime, onTimeUpdate, className }: VideoPreviewProps) {
  const ref = useRef<HTMLVideoElement>(null)

  useEffect(() => {
    const el = ref.current
    if (!el || currentTime === undefined) return
    if (Math.abs(el.currentTime - currentTime) > 0.35) {
      el.currentTime = currentTime
    }
  }, [currentTime])

  return (
    <div className={cn("media-above-grain panel-inset overflow-hidden bg-black", className)}>
      <video
        ref={ref}
        src={src}
        controls
        className="aspect-video w-full bg-black object-contain"
        onTimeUpdate={(e) => onTimeUpdate?.(e.currentTarget.currentTime)}
      />
    </div>
  )
}
