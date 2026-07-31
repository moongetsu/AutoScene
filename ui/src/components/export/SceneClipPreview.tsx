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
  /** False while HEVC→H.264 proxy still building — hover stays on still. */
  hoverVideoEnabled?: boolean
  className?: string
}

/**
 * Export tile: JPEG still (lazy) + optional hover loop on browser-playable video.
 * Must not sit inside a <button> — nested <video> breaks hover/play.
 */
export function SceneClipPreview({
  projectId,
  startSec,
  endSec,
  faceTimeSec,
  personId: _personId,
  hoverVideoEnabled = true,
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
  const [videoReady, setVideoReady] = useState(false)

  // Prefer a frame where the focus person's face was actually detected so a
  // tile never shows the opposing shot plate.
  const stillAt =
    faceTimeSec != null
      ? Math.min(Math.max(faceTimeSec, startSec), Math.max(startSec, endSec - 0.04))
      : Math.max(
          0,
          startSec + Math.min(0.85, Math.max(0.2, (endSec - startSec) * 0.4)),
        )
  const endAt = Math.max(stillAt + 0.1, endSec - 0.04)
  const videoSrc = appendQuery(api.videoUrl(projectId), { v: projectId })
  // Full-frame plate only. Face crops stay on gallery cast + import live preview.
  const frameSrc = appendQuery(api.sceneFrameUrl(projectId, stillAt), {
    v: "5",
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
      { rootMargin: "160px 0px", threshold: 0.01 },
    )
    io.observe(el)
    return () => io.disconnect()
  }, [])

  useEffect(() => {
    setImgReady(false)
    setImgFailed(false)
  }, [frameSrc])

  useEffect(() => {
    if (!hovering || !hoverVideoEnabled) {
      hoverRef.current = false
      setVideoReady(false)
      videoRef.current?.pause()
      return
    }

    const v = videoRef.current
    if (!v) return
    hoverRef.current = true

    const kick = () => {
      if (!hoverRef.current) return
      try {
        v.currentTime = stillAt
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
  }, [hovering, hoverVideoEnabled, stillAt])

  const showVideo = hovering && hoverVideoEnabled && videoReady

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
          loading="lazy"
          decoding="async"
          onLoad={() => setImgReady(true)}
          onError={() => setImgFailed(true)}
          className={cn(
            "absolute inset-0 z-[1] size-full object-cover transition-opacity duration-200",
            imgReady && !showVideo ? "opacity-100" : "opacity-0",
          )}
        />
      ) : null}

      {hovering && hoverVideoEnabled ? (
        <video
          ref={videoRef}
          src={videoSrc}
          muted
          playsInline
          preload="metadata"
          disablePictureInPicture
          className={cn(
            "absolute inset-0 z-[2] size-full object-cover",
            showVideo ? "opacity-100" : "opacity-0",
          )}
          onLoadedData={() => {
            const v = videoRef.current
            if (!v || !hoverRef.current) return
            try {
              v.currentTime = stillAt
            } catch {
              /* ignore */
            }
          }}
          onSeeked={() => {
            if (hoverRef.current) setVideoReady(true)
          }}
          onPlaying={() => setVideoReady(true)}
          onTimeUpdate={() => {
            const v = videoRef.current
            if (!v || !hoverRef.current) return
            if (v.currentTime >= endAt) {
              try {
                v.currentTime = stillAt
              } catch {
                /* ignore */
              }
            }
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
