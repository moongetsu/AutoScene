import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { openExternalUrl } from "@/lib/os"
import { ContributorsSection } from "./ContributorsSection"

vi.mock("@/lib/os", () => ({
  openExternalUrl: vi.fn(),
}))

afterEach(cleanup)

describe("ContributorsSection", () => {
  it("renders contributors and links to their profiles", () => {
    render(<ContributorsSection />)

    expect(screen.getByText("Community Contributors")).toBeTruthy()
    expect(screen.getByText("Contribute on GitHub")).toBeTruthy()

    expect(screen.getByText("tcmmi")).toBeTruthy()
    expect(screen.getByText("0nsku")).toBeTruthy()
    expect(screen.getByText("ihyledd")).toBeTruthy()
    expect(screen.getByText("Moongetsu")).toBeTruthy()

    const contributeButton = screen.getByRole("button", { name: /Contribute on GitHub/i })
    fireEvent.click(contributeButton)
    expect(openExternalUrl).toHaveBeenCalledWith("https://github.com/tcmmi/AutoScene")

    const profileButton = screen.getByRole("button", { name: /Open Moongetsu's GitHub profile/i })
    fireEvent.click(profileButton)
    expect(openExternalUrl).toHaveBeenCalledWith("https://github.com/moongetsu")
  })
})

