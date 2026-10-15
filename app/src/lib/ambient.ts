import { useEffect, useState } from "react"
import { assetUrl } from "./media"

const cache = new Map<string, string | null>()

/**
 * The colour a picture glows with: a vivid, not too dark average of a tiny
 * copy (saturated pixels count most). `null` while loading, without a
 * picture, or where canvas reads aren't possible.
 */
export function useAmbient(path: string | null | undefined): string | null {
  const url = path ? assetUrl(path) : null
  // The last colour read, with the picture it belongs to.
  const [read, setRead] = useState<{ url: string; color: string | null } | null>(null)
  useEffect(() => {
    if (!url || cache.has(url)) return
    let live = true
    const img = new Image()
    img.crossOrigin = "anonymous"
    img.decoding = "async"
    img.onload = () => {
      const color = ambientOf(img)
      cache.set(url, color)
      if (live) setRead({ url, color })
    }
    img.onerror = () => cache.set(url, null)
    img.src = url
    return () => {
      live = false
    }
  }, [url])
  if (!url) return null
  return cache.get(url) ?? (read?.url === url ? read.color : null)
}

function ambientOf(img: HTMLImageElement): string | null {
  const w = 32
  const h = 18
  const canvas = document.createElement("canvas")
  canvas.width = w
  canvas.height = h
  const ctx = canvas.getContext("2d", { willReadFrequently: true })
  if (!ctx) return null
  try {
    ctx.drawImage(img, 0, 0, w, h)
    const d = ctx.getImageData(0, 0, w, h).data
    let [r, g, b, total] = [0, 0, 0, 0]
    for (let i = 0; i < d.length; i += 4) {
      const [pr, pg, pb] = [d[i], d[i + 1], d[i + 2]]
      const max = Math.max(pr, pg, pb)
      const min = Math.min(pr, pg, pb)
      // Saturated, mid-bright pixels carry the mood; greys and blacks barely count.
      const weight = 0.05 + ((max - min) / 255) ** 2 * (max / 255)
      r += pr * weight
      g += pg * weight
      b += pb * weight
      total += weight
    }
    if (total === 0) return null
    r /= total
    g /= total
    b /= total
    // Lift dark results so the glow reads on near-black.
    const lift = Math.max(1, 150 / Math.max(r, g, b, 1))
    const c = (v: number) => Math.round(Math.min(255, v * lift))
    return `rgb(${c(r)} ${c(g)} ${c(b)})`
  } catch {
    return null
  }
}
