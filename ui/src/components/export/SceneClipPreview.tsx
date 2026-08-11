import { useEffect, useRef, useState } from "react"
import { api, appendQuery } from "@/lib/api"
import { cn } from "@/lib/utils"

type SceneClipPreviewProps = {
  projectId: string
  startSec: number
  endSec: number
  /** Timestamp of a real detection of the focus person — still-frame anchor. */
  faceTimeSec?: number | null
  /** Unused for Export stills — full plate only (face crops are gallery/import). */
  personId?: number | null
  className?: string
}

/**
 * Export tile: JPEG still + hover loop of a short per-scene H.264 clip.
 * Does not wait on the full-film browser_proxy (that can take many minutes).
 */
export function SceneClipPreview({
  projectId,
  startSec,
  endSec,
  faceTimeSec,
  personId: _personId,
  className,
}: SceneClipPreviewProps) {
  void _personId
  const rootRef = useRef<HTMLDivElement | null>(null)
  const videoRef = useRef<HTMLVideoElement | null>(null)
  const hoverRef = useRef(false)
  const [inView, setInView] = useState(false)
  const [imgReady, setImgReady] = useState(false)
  const [imgFailed, setImgFailed] = useState(false)
  const [hovering, setHovering] = useState(false)
  const [clipReady, setClipReady] = useState(false)
  const [videoReady, setVideoReady] = useState(false)
  const [clipGen, setClipGen] = useState(0)

  const stillAt =
    faceTimeSec != null
      ? Math.min(Math.max(faceTimeSec, startSec), Math.max(startSec, endSec - 0.04))
      : Math.max(
          0,
          startSec + Math.min(0.85, Math.max(0.2, (endSec - startSec) * 0.4)),
        )

  const hoverSrc = appendQuery(
    api.sceneHoverUrl(projectId, startSec, endSec),
    { v: String(clipGen) },
  )
  const frameSrc = appendQuery(api.sceneFrameUrl(projectId, stillAt), {
    v: "6",
  })

  useEffect(() => {
    const el = rootRef.current
    if (!el) return
    const io = new IntersectionObserver(
      (entries) => {
        if (entries.some((e) => e.isIntersecting)) {
          setInView(true)
          io.disconnect()
        }
      },
      { rootMargin: "800px 0px", threshold: 0.01 },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [])

  useEffect(() => {
    setImgReady(false)
    setImgFailed(false)
  }, [frameSrc])

  // Warm / poll short hover clip (independent of full-film proxy).
  useEffect(() => {
    if (!inView && !hovering) return
    let cancelled = false
    let timer: ReturnType<typeof setTimeout> | undefined

    const tick = async () => {
      try {
        const res = await fetch(
          appendQuery(api.sceneHoverUrl(projectId, startSec, endSec), {
            warm: "1",
          }),
        )
        if (cancelled) return
        if (res.status === 200) {
          setClipReady(true)
          setClipGen((g) => g + 1)
          return
        }
        // 202 while building — poll again.
        timer = setTimeout(() => {
          void tick()
        }, 900)
      } catch {
        if (!cancelled) {
          timer = setTimeout(() => {
            void tick()
          }, 1500)
        }
      }
    }
    void tick()
    return () => {
      cancelled = true
      if (timer) clearTimeout(timer)
    }
  }, [projectId, startSec, endSec, inView, hovering])

  useEffect(() => {
    if (!hovering || !clipReady) {
      hoverRef.current = false
      videoRef.current?.pause()
      return
    }
    const v = videoRef.current
    if (!v) return
    hoverRef.current = true
    const kick = () => {
      if (!hoverRef.current) return
      try {
        v.currentTime = 0
      } catch {
        /* ignore */
      }
      void v
        .play()
        .then(() => {
          if (hoverRef.current) setVideoReady(true)
        })
        .catch(() => {
          hoverRef.current = false
          setVideoReady(false)
        })
    }
    if (v.readyState >= 2) kick()
    else {
      const onReady = () => kick()
      v.addEventListener("loadeddata", onReady, { once: true })
      v.addEventListener("canplay", onReady, { once: true })
      return () => {
        v.removeEventListener("loadeddata", onReady)
        v.removeEventListener("canplay", onReady)
      }
    }
  }, [hovering, clipReady, hoverSrc])

  const showVideo = hovering && clipReady && videoReady

  return (
    <div
      ref={rootRef}
      className={cn(
        "absolute inset-0 overflow-hidden bg-[#3a3545]",
        "ring-1 ring-inset ring-white/15",
        className,
      )}
      onPointerEnter={() => setHovering(true)}
      onPointerLeave={() => setHovering(false)}
    >
      {!imgReady && !imgFailed ? (
        <div className="absolute inset-0 z-0 animate-pulse bg-gradient-to-br from-white/10 via-transparent to-black/20" />
      ) : null}

      {inView ? (
        <img
          src={frameSrc}
          alt=""
          draggable={false}
          decoding="async"
          onLoad={() => setImgReady(true)}
          onError={() => setImgFailed(true)}
          className={cn(
            "absolute inset-0 z-[1] size-full object-cover transition-opacity duration-150",
            imgReady && !showVideo ? "opacity-100" : "opacity-0",
          )}
        />
      ) : null}

      {clipReady ? (
        <video
          ref={videoRef}
          src={hoverSrc}
          muted
          playsInline
          loop
          preload="auto"
          disablePictureInPicture
          className={cn(
            "absolute inset-0 z-[2] size-full object-cover",
            showVideo ? "opacity-100" : "opacity-0",
          )}
          onPlaying={() => setVideoReady(true)}
          onLoadedData={() => {
            if (hoverRef.current) setVideoReady(true)
          }}
        />
      ) : null}

      {imgFailed ? (
        <div className="absolute inset-0 z-[3] flex items-center justify-center bg-black/35 px-2 text-center">
          <p className="text-[10px] text-white/70">Preview unavailable</p>
        </div>
      ) : null}
    </div>
  )
}
