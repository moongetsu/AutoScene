import type * as React from "react"
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react"
import { useReducedMotion } from "motion/react"
import { HoverCard } from "radix-ui"
import type { FaceTrackItem, MediaItem } from "@/bindings"
import { FrameClock } from "@/lib/frame-clock"
import { assetUrl } from "@/lib/media"
import { useFrameMap } from "@/lib/queries"
import { timecode } from "@/lib/timecode"
import { cn } from "@/lib/utils"
import { canPlayMoment, loopsBack, momentLoop } from "./moment"

type VideoFrameMeta = { mediaTime: number }
type RvfcVideo = HTMLVideoElement & {
  requestVideoFrameCallback?: (cb: (now: number, meta: VideoFrameMeta) => void) => number
  cancelVideoFrameCallback?: (handle: number) => void
}

/** Files the webview couldn't play this session: their faces keep their stills without trying again. */
const unplayable = new Set<string>()

/**
 * A face whose moment plays in a card above it while the pointer rests on it
 * (or Space asks for it): muted and round and round, without leaving the
 * page. The grid says which face is open, so only one plays at a time. Faces
 * whose episode can't play yet keep their still, and say nothing about it.
 */
export function FacePreview({
  pid,
  track,
  media,
  open,
  onOpenChange,
  children,
}: {
  pid: string
  track: FaceTrackItem
  media?: MediaItem
  open: boolean
  onOpenChange: (open: boolean) => void
  /** The face: the card opens while the pointer is anywhere on it. */
  children: React.ReactElement
}) {
  // Pictures that start moving by themselves stay still; Space still plays them.
  const reduced = useReducedMotion()
  const playable = canPlayMoment(media)
  return (
    <HoverCard.Root
      openDelay={450}
      closeDelay={80}
      open={open && playable}
      onOpenChange={(o) => {
        if (o && (!playable || reduced)) return
        onOpenChange(o)
      }}
    >
      {/* Focus alone doesn't play it: tabbing through the faces would start every one. */}
      <HoverCard.Trigger asChild onFocus={(e) => e.preventDefault()}>
        {children}
      </HoverCard.Trigger>
      {open && playable && <Moment pid={pid} track={track} label={media?.label} onFail={() => onOpenChange(false)} />}
    </HoverCard.Root>
  )
}

/**
 * The moment itself, from the file the player plays (the episode or its
 * preview copy, as its frame map says). The card stays invisible until the
 * moment's first frame shows; when the file can't play, it never shows.
 */
function Moment({ pid, track, label, onFail }: { pid: string; track: FaceTrackItem; label?: string; onFail: () => void }) {
  const map = useFrameMap(pid, track.mediaId, true)
  const data = map.data
  const clock = useMemo(() => {
    if (!data) return null
    try {
      return new FrameClock(data)
    } catch {
      return null
    }
  }, [data])
  const loop = useMemo(() => (clock ? momentLoop(clock, track.start, track.end) : null), [clock, track.start, track.end])
  const src = data ? assetUrl(data.src) : null
  // The element as state, not a ref: the portal mounts it a render after
  // this one, and the effect below has to run again once it is there.
  const [video, setVideo] = useState<HTMLVideoElement | null>(null)
  const [shown, setShown] = useState(false)
  const fail = useRef(onFail)
  useLayoutEffect(() => {
    fail.current = onFail
  })

  const broken = map.isError || (data != null && (!loop || (src != null && unplayable.has(src))))
  useEffect(() => {
    if (broken) fail.current()
  }, [broken])

  useEffect(() => {
    const v = video as RvfcVideo | null
    if (!v || !src || !clock || !loop || unplayable.has(src)) return
    const moving = loop.last > loop.first
    let handle = 0
    let raf = 0
    const begin = () => {
      v.currentTime = loop.from
    }
    const onSeeked = () => {
      setShown(true)
      if (moving && v.paused) v.play().catch(() => {})
    }
    const onError = () => {
      unplayable.add(src)
      fail.current()
    }
    // Back to the start once the last frame shows, not when the shot after it does.
    const check = (t: number) => {
      if (!v.seeking && loopsBack(clock, loop, t)) begin()
    }
    v.addEventListener("loadedmetadata", begin)
    v.addEventListener("seeked", onSeeked)
    v.addEventListener("ended", begin)
    v.addEventListener("error", onError)
    v.muted = true
    v.src = src
    if (moving && v.requestVideoFrameCallback) {
      const onFrame = (_: number, meta: VideoFrameMeta) => {
        check(meta.mediaTime)
        handle = v.requestVideoFrameCallback!(onFrame)
      }
      handle = v.requestVideoFrameCallback(onFrame)
    } else if (moving) {
      const tick = () => {
        check(v.currentTime)
        raf = requestAnimationFrame(tick)
      }
      raf = requestAnimationFrame(tick)
    }
    return () => {
      v.cancelVideoFrameCallback?.(handle)
      cancelAnimationFrame(raf)
      v.removeEventListener("loadedmetadata", begin)
      v.removeEventListener("seeked", onSeeked)
      v.removeEventListener("ended", begin)
      v.removeEventListener("error", onError)
      // Let go of the file and its decoder now, not whenever the element is collected.
      v.pause()
      v.removeAttribute("src")
      v.load()
    }
  }, [video, src, clock, loop])

  if (!data || broken) return null
  const where = [label, timecode(track.start, data.fps, data.dropFrame)].filter(Boolean).join(" · ")
  return (
    <HoverCard.Portal>
      {/* Only to look at: it never takes a click or the pointer from the faces it covers. */}
      <HoverCard.Content
        side="top"
        sideOffset={10}
        collisionPadding={12}
        style={{ pointerEvents: "none" }}
        className={cn(
          "glass-strong z-50 w-72 max-w-[calc(100vw-24px)] rounded-2xl p-1.5 transition-opacity duration-[var(--dur-fast)]",
          !shown && "opacity-0",
        )}
      >
        {/* Loaded only while the card is open. No crossOrigin: nothing reads its pixels (the player's is for canvas grabs). */}
        <video
          ref={setVideo}
          muted
          playsInline
          disablePictureInPicture
          preload="auto"
          className="aspect-video w-full rounded-xl bg-black object-contain"
        />
        <div className="flex items-baseline justify-between gap-3 px-1.5 pt-1.5 pb-0.5">
          <span className="timecode truncate text-xs font-medium text-fg-1">{where}</span>
          <span className="shrink-0 text-2xs text-fg-3">Double-click to watch</span>
        </div>
      </HoverCard.Content>
    </HoverCard.Portal>
  )
}
