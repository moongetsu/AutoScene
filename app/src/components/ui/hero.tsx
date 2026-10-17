import type * as React from "react"
import { assetUrl } from "@/lib/media"
import { cn } from "@/lib/utils"

/**
 * The big picture at the top of a project or character: the image bleeds to
 * the window's edges and into the page below, lit by its own ambient colour,
 * with the text on its dark side.
 */
export function Hero({
  path,
  ambient,
  blur,
  className,
  children,
  ...props
}: {
  path: string | null | undefined
  /** Colour the picture glows with (`useAmbient`). */
  ambient?: string | null
  /** Blur the picture into a backdrop (a face is too small to show sharp). */
  blur?: boolean
  className?: string
  children: React.ReactNode
} & Omit<React.HTMLAttributes<HTMLElement>, "title" | "children"> & { ref?: React.Ref<HTMLElement> }) {
  const url = path ? assetUrl(path) : null
  return (
    <section className={cn("relative isolate flex min-h-[clamp(380px,56vh,600px)] flex-col justify-end", className)} {...props}>
      <div aria-hidden className="absolute inset-0 -z-10 overflow-hidden">
        {url ? (
          <img
            src={url}
            alt=""
            draggable={false}
            className={cn(
              "size-full animate-fade-in object-cover",
              blur ? "scale-125 blur-3xl saturate-150" : "scale-[1.02] object-[50%_30%]",
            )}
          />
        ) : (
          <div className="artwork-fallback size-full" />
        )}
        {ambient && (
          <div
            className="absolute inset-0 opacity-60 mix-blend-soft-light"
            style={{ background: `radial-gradient(70% 70% at 20% 90%, ${ambient}, transparent 70%)` }}
          />
        )}
        {/* Legibility: under the top bar, behind the text, into the page below. */}
        <div className="absolute inset-x-0 top-0 h-40 bg-gradient-to-b from-black/55 to-transparent" />
        <div className="absolute inset-0 bg-[linear-gradient(90deg,rgb(9_9_11/0.92)_0%,rgb(9_9_11/0.6)_38%,rgb(9_9_11/0.1)_70%,transparent_100%)]" />
        <div className="absolute inset-x-0 bottom-0 h-2/3 bg-gradient-to-t from-canvas via-canvas/60 to-transparent" />
      </div>
      <div className="relative px-10 pt-28 pb-10">{children}</div>
    </section>
  )
}
