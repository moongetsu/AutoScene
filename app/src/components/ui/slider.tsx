import { useState } from "react"
import { Slider as S } from "radix-ui"
import { cn } from "@/lib/utils"

/**
 * A slider in the app's glass, never the browser's range input: a quiet
 * track, a white fill, a round thumb, and its value in a chip over the thumb
 * while it is hovered, focused or dragged. Arrow keys step it (and don't
 * reach the player's frame keys).
 */
export function Slider({
  value,
  onChange,
  min = 0,
  max = 1,
  step = 0.01,
  label,
  format,
  className,
}: {
  value: number
  onChange: (value: number) => void
  min?: number
  max?: number
  step?: number
  /** What it sets, for screen readers. */
  label: string
  /** The chip's text for a value; no chip without it. */
  format?: (value: number) => string
  className?: string
}) {
  const [hover, setHover] = useState(false)
  const [focus, setFocus] = useState(false)
  const [drag, setDrag] = useState(false)
  return (
    <S.Root
      value={[value]}
      min={min}
      max={max}
      step={step}
      onValueChange={([v]) => onChange(v)}
      onPointerEnter={() => setHover(true)}
      onPointerLeave={() => setHover(false)}
      onPointerDown={(e) => {
        // Only the main button slides: a right-click opens the menu (Radix
        // skips its own handler for an event cancelled here).
        if (e.button !== 0) e.preventDefault()
        else setDrag(true)
      }}
      onLostPointerCapture={() => setDrag(false)}
      className={cn("relative flex h-5 touch-none items-center select-none", className)}
    >
      <S.Track className="relative h-1 grow overflow-hidden rounded-full bg-white/[0.14]">
        <S.Range className="absolute h-full rounded-full bg-fg-1" />
      </S.Track>
      <S.Thumb
        aria-label={label}
        onFocus={() => setFocus(true)}
        onBlur={() => setFocus(false)}
        className={cn(
          "relative block size-3.5 rounded-full border border-white/40 bg-white shadow-[0_1px_4px_rgb(0_0_0/0.5)] outline-none",
          "transition-transform duration-[var(--dur-fast)] hover:scale-110 focus-visible:ring-4 focus-visible:ring-accent-subtle",
        )}
      >
        {format && (hover || focus || drag) && (
          <span className="glass-strong pointer-events-none absolute bottom-full left-1/2 mb-2 -translate-x-1/2 rounded-md px-1.5 py-0.5 text-2xs font-medium whitespace-nowrap text-fg-1">
            {format(value)}
          </span>
        )}
      </S.Thumb>
    </S.Root>
  )
}
