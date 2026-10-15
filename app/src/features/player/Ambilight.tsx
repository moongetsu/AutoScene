import type * as React from "react"
import { useEffect, useRef } from "react"
import type { FrameStore } from "./frame-store"

/** How often the glow follows playback. */
const AMBILIGHT_MS = 100

/**
 * The picture's glow in the room: a tiny copy of the current frame, blurred
 * far beyond the screen's edges. It follows the frame about ten times a
 * second (a trailing draw catches the frame playback stops on), drawing
 * straight from the frame store — nothing else draws again for it.
 */
export function Ambilight({ video, store }: { video: React.RefObject<HTMLVideoElement | null>; store: FrameStore }) {
  const canvas = useRef<HTMLCanvasElement>(null)
  useEffect(() => {
    let last = 0
    let timer = 0
    const draw = () => {
      last = performance.now()
      timer = 0
      drawFrame(video.current, canvas.current)
    }
    const onFrame = () => {
      if (timer) return
      const wait = AMBILIGHT_MS - (performance.now() - last)
      if (wait <= 0) draw()
      else timer = window.setTimeout(draw, wait)
    }
    const unsubscribe = store.subscribe(onFrame)
    const v = video.current
    v?.addEventListener("seeked", draw)
    v?.addEventListener("loadeddata", draw)
    return () => {
      unsubscribe()
      window.clearTimeout(timer)
      v?.removeEventListener("seeked", draw)
      v?.removeEventListener("loadeddata", draw)
    }
  }, [video, store])
  return (
    <canvas
      ref={canvas}
      width={48}
      height={27}
      aria-hidden
      className="pointer-events-none absolute inset-0 size-full scale-110 opacity-60 blur-[100px] brightness-125 saturate-[1.8]"
    />
  )
}

function drawFrame(v: HTMLVideoElement | null, c: HTMLCanvasElement | null) {
  const ctx = c?.getContext("2d")
  if (!v || !ctx || v.readyState < 2) return
  try {
    ctx.drawImage(v, 0, 0, ctx.canvas.width, ctx.canvas.height)
  } catch {
    // A frame that can't be read (no CORS) just leaves the room dark.
  }
}
