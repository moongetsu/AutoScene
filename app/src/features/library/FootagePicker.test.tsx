import { useState } from "react"
import { cleanup, render, screen } from "@testing-library/react"
import userEvent from "@testing-library/user-event"
import { afterEach, describe, expect, it } from "vitest"
import type { ContentKind } from "@/bindings"
import { FootagePicker } from "./FootagePicker"

afterEach(cleanup)

function Picker({ locked }: { locked?: Partial<Record<ContentKind, string>> }) {
  const [v, setV] = useState<ContentKind>("live")
  return (
    <>
      <FootagePicker value={v} onChange={setV} locked={locked} />
      <output>{v}</output>
    </>
  )
}

describe("FootagePicker", () => {
  it("offers animation beside live action, anime and mixed", async () => {
    const user = userEvent.setup()
    render(<Picker />)
    const names = screen.getAllByRole("radio").map((r) => r.textContent)
    expect(names).toEqual(["Live action", "Animation", "Anime", "Mixed"])
    await user.click(screen.getByRole("radio", { name: "Animation" }))
    expect(screen.getByRole("status").textContent).toBe("animation")
  })

  it("shows example stills with their credit while the pointer rests on an option", async () => {
    const user = userEvent.setup()
    render(<Picker />)
    await user.hover(screen.getByRole("radio", { name: "Animation" }))
    expect(await screen.findByText(/Sprite Fright, Spring/)).toBeTruthy()
    const stills = document.querySelectorAll('img[src^="/footage/animation-"]')
    expect(stills.length).toBe(3)
  })

  it("a locked option can't be chosen and its preview says why", async () => {
    const user = userEvent.setup()
    render(<Picker locked={{ anime: "Add anime as a new project." }} />)
    const anime = screen.getByRole("radio", { name: "Anime" }) as HTMLButtonElement
    expect(anime.disabled).toBe(true)
    await user.hover(anime.parentElement!)
    expect(await screen.findByText("Add anime as a new project.")).toBeTruthy()
    expect(screen.getByRole("status").textContent).toBe("live")
  })
})
