import type * as React from "react"
import { useState } from "react"
import { assetUrl } from "@/lib/media"
import { cn } from "@/lib/utils"

/**
 * A picture from the library (still, poster, face): it fades in once loaded,
 * sits on a quiet placeholder until then, and shows `fallback` without one.
 * `children` are drawn over it (labels, scrims, badges).
 */
export function Artwork({
  path,
  alt = "",
  className,
  imgClassName,
  fallback,
  children,
}: {
  path: string | null | undefined
  alt?: string
  className?: string
  imgClassName?: string
  fallback?: React.ReactNode
  children?: React.ReactNode
}) {
  const url = path ? assetUrl(path) : null
  return (
    <div className={cn("relative isolate overflow-hidden bg-raised", className)}>
      {url ? <FadeImage key={url} url={url} alt={alt} className={imgClassName} fallback={fallback} /> : fallback}
      {children}
    </div>
  )
}

function FadeImage({
  url,
  alt,
  className,
  fallback,
}: {
  url: string
  alt: string
  className?: string
  fallback?: React.ReactNode
}) {
  const [state, setState] = useState<"loading" | "loaded" | "failed">("loading")
  if (state === "failed") return <>{fallback}</>
  const loaded = state === "loaded"
  return (
    <>
      {!loaded && <div aria-hidden className="skeleton absolute inset-0 -z-10" />}
      <img
        src={url}
        alt={alt}
        loading="lazy"
        decoding="async"
        draggable={false}
        onLoad={() => setState("loaded")}
        onError={() => setState("failed")}
        className={cn(
          "absolute inset-0 -z-10 size-full object-cover transition-[opacity,transform] duration-[var(--dur-slow)] ease-out",
          loaded ? "opacity-100" : "opacity-0",
          className,
        )}
      />
    </>
  )
}
