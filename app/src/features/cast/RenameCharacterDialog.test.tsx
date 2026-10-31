import { fireEvent, render, screen, waitFor } from "@testing-library/react"
import { describe, expect, it, vi } from "vitest"
import type { Character } from "@/bindings"
import * as castModule from "./cast"
import { RenameCharacterDialog } from "./RenameCharacterDialog"

describe("RenameCharacterDialog", () => {
  const dummyChar: Character = {
    id: "char-1",
    name: "Old Name",
    label: "Old Name",
    hidden: false,
    tracks: 5,
    shots: 12,
    media: 1,
    screenSec: 120,
    unsure: 0,
    waiting: 0,
    cover: null,
    coverSmall: null,
  }

  it("renders with current character name and saves updated name", async () => {
    const castEditSpy = vi.spyOn(castModule, "castEdit").mockResolvedValue("char-1")
    const onOpenChange = vi.fn()

    render(
      <RenameCharacterDialog
        pid="proj-1"
        character={dummyChar}
        open={true}
        onOpenChange={onOpenChange}
      />,
    )

    const input = screen.getByLabelText("Character name") as HTMLInputElement
    expect(input.value).toBe("Old Name")

    fireEvent.change(input, { target: { value: "New Name" } })
    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      expect(castEditSpy).toHaveBeenCalledWith("proj-1", {
        op: "rename",
        characterId: "char-1",
        name: "New Name",
      })
      expect(onOpenChange).toHaveBeenCalledWith(false)
    })
  })

  it("closes without saving if name has not changed", async () => {
    const castEditSpy = vi.spyOn(castModule, "castEdit").mockResolvedValue("char-1")
    const onOpenChange = vi.fn()

    render(
      <RenameCharacterDialog
        pid="proj-1"
        character={dummyChar}
        open={true}
        onOpenChange={onOpenChange}
      />,
    )

    fireEvent.click(screen.getByRole("button", { name: "Save" }))

    await waitFor(() => {
      expect(castEditSpy).not.toHaveBeenCalled()
      expect(onOpenChange).toHaveBeenCalledWith(false)
    })
  })
})
