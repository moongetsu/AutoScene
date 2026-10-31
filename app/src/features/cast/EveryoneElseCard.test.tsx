import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { AppMenu } from "@/components/shell/AppMenu"
import { EveryoneElseCard } from "./EveryoneElseCard"

const navigate = vi.fn()
vi.mock("@tanstack/react-router", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@tanstack/react-router")>()),
  Link: ({ to: _to, params: _params, ...props }: React.ComponentProps<"a"> & Record<string, unknown>) => <a href="#everyone" {...props} />,
  useNavigate: () => navigate,
  useCanGoBack: () => false,
}))

afterEach(cleanup)

describe("Everyone else", () => {
  for (const view of ["grid", "list"] as const) {
    it(`opens from its menu (${view})`, async () => {
      render(
        <AppMenu>
          <div>
            <EveryoneElseCard pid="p" unassigned={14} view={view} />
          </div>
        </AppMenu>,
      )
      fireEvent.contextMenu(screen.getByRole("link"), { clientX: 40, clientY: 40 })
      fireEvent.click(await screen.findByRole("menuitem", { name: /Open everyone else/ }))
      await vi.waitFor(() => expect(navigate).toHaveBeenCalledWith({ to: "/p/$pid/cast/$cid", params: { pid: "p", cid: "unassigned" } }))
    })
  }

  it("isn't there when nobody is left over", () => {
    render(<EveryoneElseCard pid="p" unassigned={0} />)
    expect(screen.queryByRole("link")).toBeNull()
  })
})
