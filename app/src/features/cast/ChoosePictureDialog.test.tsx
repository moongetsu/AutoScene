import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { Character, FaceTrackItem } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import * as castModule from "./cast"
import { ChoosePictureDialog } from "./ChoosePictureDialog"

const face = (id: number, quality: number): FaceTrackItem => ({
  id,
  mediaId: "m",
  start: id * 10,
  end: id * 10 + 5,
  quality,
  faceHeight: 0.2,
  thumb: `faces/${id}.jpg`,
  characterId: "walter",
  assign: "auto",
  score: 1,
})

vi.mock("@/lib/queries", () => ({
  useCastTracks: () => ({ isPending: false, isError: false, data: [face(1, 0.4), face(2, 0.9), face(3, 0.6)] }),
  useMediaList: () => ({ data: [] }),
}))

const walter: Character = {
  id: "walter",
  name: "Walter",
  label: "Walter",
  hidden: false,
  tracks: 3,
  shots: 3,
  media: 1,
  screenSec: 30,
  cover: "faces/1.jpg",
  coverSmall: "faces/1.jpg",
  unsure: 0,
  waiting: 0,
}

afterEach(cleanup)

const open = (onOpenChange = vi.fn()) =>
  render(
    <TooltipProvider>
      <ChoosePictureDialog pid="p" character={walter} open onOpenChange={onOpenChange} />
    </TooltipProvider>,
  )

describe("ChoosePictureDialog", () => {
  it("shows their picture now first, then the clearest, and makes the one clicked their picture", async () => {
    const edit = vi.spyOn(castModule, "castEdit").mockResolvedValue(null)
    const onOpenChange = vi.fn()
    open(onOpenChange)
    const faces = screen.getAllByRole("button", { name: /^Face/ })
    expect(faces.map((b) => b.getAttribute("aria-pressed"))).toEqual(["true", "false", "false"])
    fireEvent.click(faces[1])
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
    expect(edit).toHaveBeenCalledWith("p", { op: "set_cover", characterId: "walter", track: 2 })
  })

  it("just closes when their picture now is clicked", async () => {
    const edit = vi.spyOn(castModule, "castEdit").mockResolvedValue(null)
    const onOpenChange = vi.fn()
    open(onOpenChange)
    fireEvent.click(screen.getAllByRole("button", { name: /^Face/ })[0])
    await waitFor(() => expect(onOpenChange).toHaveBeenCalledWith(false))
    expect(edit).not.toHaveBeenCalled()
  })
})
