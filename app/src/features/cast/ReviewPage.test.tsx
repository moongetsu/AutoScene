import type * as React from "react"
import { act, cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { Character, FaceTrackItem } from "@/bindings"
import { TooltipProvider } from "@/components/ui/tooltip"
import { castEdit } from "./cast"
import { ReviewPage } from "./ReviewPage"

const walter = { id: "walter", label: "Walter", name: "Walter", hidden: false } as Character
const face = { id: 7, mediaId: "m", start: 24, end: 48, thumb: "t.jpg", characterId: "walter", assign: "unsure", score: 0.5 } as FaceTrackItem

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ pid: "p" }),
  useSearch: () => ({}),
  useNavigate: () => vi.fn(),
  Link: ({ children }: { children: React.ReactNode }) => <a>{children}</a>,
}))
// The page around it scrolls and collects menus; here only the face matters.
vi.mock("@/components/ui/page", () => ({ Page: ({ children }: { children: React.ReactNode }) => <main>{children}</main> }))
vi.mock("@/lib/queries", () => ({
  useCast: () => ({ data: { characters: [walter] } }),
  useMediaList: () => ({ data: [] }),
  useCastTracks: () => ({ data: [face], isSuccess: true }),
  usePortrait: () => ({ data: undefined }),
  portraitQuery: () => ({}),
  queryClient: { prefetchQuery: () => Promise.resolve() },
  firstFailure: () => null,
}))
vi.mock("./cast", async (importOriginal) => ({
  ...(await importOriginal<typeof import("./cast")>()),
  // Still on its way: the page is answering.
  castEdit: vi.fn(() => new Promise(() => {})),
}))

afterEach(cleanup)

describe("reviewing faces", () => {
  it("answers once, however quickly the same answer comes twice", () => {
    render(<ReviewPage />, { wrapper: TooltipProvider })
    const yes = screen.getByRole("button", { name: /Yes, Walter/ })
    // Both clicks before the page draws again (the button still enabled).
    act(() => {
      yes.click()
      yes.click()
    })
    expect(castEdit).toHaveBeenCalledOnce()
    expect(castEdit).toHaveBeenCalledWith("p", { op: "assign", tracks: [7], characterId: "walter" })
  })
})
