import { cleanup, render, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { FaceTrackItem, MediaItem } from "@/bindings"
import { FacePreview } from "./FacePreview"

vi.mock("@/lib/queries", () => ({
  useFrameMap: () => ({
    data: { src: "C:/Show/E01.mkv", frames: 240, fps: { num: 24, den: 1 }, offset: 0, step: 1 / 24, times: null, dropFrame: false },
    isError: false,
  }),
}))
vi.mock("@/lib/media", () => ({ assetUrl: (p: string) => `asset://${p}` }))

afterEach(cleanup)

describe("a face's moment", () => {
  it("loads its file into the card's video once the card is open", async () => {
    // jsdom has no media: the effect's own calls must not throw.
    vi.spyOn(HTMLMediaElement.prototype, "load").mockImplementation(() => {})
    vi.spyOn(HTMLMediaElement.prototype, "pause").mockImplementation(() => {})
    const track = { id: 1, mediaId: "m", start: 48, end: 72, thumb: "t.jpg" } as FaceTrackItem
    const media = { status: "ready", proxy: "not_needed", label: "S01E01" } as MediaItem
    render(
      <FacePreview pid="p" track={track} media={media} open onOpenChange={() => {}}>
        <button>face</button>
      </FacePreview>,
    )
    // The portal mounts the video a render after the card opens: it still gets its file.
    await waitFor(() => expect(document.querySelector("video")?.getAttribute("src")).toBe("asset://C:/Show/E01.mkv"))
  })
})
