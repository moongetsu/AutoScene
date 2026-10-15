import type * as React from "react"
import { useState } from "react"
import { assetUrl } from "@/lib/media"
import { cn } from "@/lib/utils"

/**
 * A picture from the library (still, poster, face): it fades in once loaded,
 * sits on a quiet placeholder until then, and shows `fallback` without one.
 * A new picture of the same thing (a sharp portrait for a thumbnail, a
 * remade poster) replaces the one shown before once loaded, without a
 * flash; key an `Artwork` by what it shows where that can change.
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
  // The last picture that loaded stays underneath until the new one has.
  const [shown, setShown] = useState<string | null>(null)
  const under = url && shown !== url ? shown : null
  return (
    <div className={cn("relative isolate overflow-hidden bg-raised", className)}>
      {under && (
        <img
          aria-hidden
          src={under}
          alt=""
          draggable={false}
          className={cn("absolute inset-0 -z-10 size-full object-cover", imgClassName)}
        />
      )}
      {url ? (
        <FadeImage
          key={url}
          url={url}
          alt={alt}
          className={imgClassName}
          fallback={fallback}
          placeholder={!under}
          onLoaded={() => setShown(url)}
          onFailed={() => setShown(null)}
        />
      ) : (
        fallback
      )}
      {children}
    </div>
  )
}

function FadeImage({
  url,
  alt,
  className,
  fallback,
  placeholder,
  onLoaded,
  onFailed,
}: {
  url: string
  alt: string
  className?: string
  fallback?: React.ReactNode
  /** Shimmer while loading (not over a picture still shown). */
  placeholder: boolean
  onLoaded: () => void
  onFailed: () => void
}) {
  const [state, setState] = useState<"loading" | "loaded" | "failed">("loading")
  if (state === "failed") return <>{fallback}</>
  const loaded = state === "loaded"
  return (
    <>
      {!loaded && placeholder && <div aria-hidden className="skeleton absolute inset-0 -z-10" />}
      <img
        src={url}
        alt={alt}
        loading="lazy"
        decoding="async"
        draggable={false}
        onLoad={() => {
          setState("loaded")
          onLoaded()
        }}
        onError={() => {
          setState("failed")
          onFailed()
        }}
        className={cn(
          "absolute inset-0 -z-10 size-full object-cover duration-[var(--dur-slow)] ease-out",
          // Over a picture still shown it appears at once: that one goes as it does.
          placeholder ? "transition-[opacity,transform]" : "transition-transform",
          loaded ? "opacity-100" : "opacity-0",
          className,
        )}
      />
    </>
  )
}
