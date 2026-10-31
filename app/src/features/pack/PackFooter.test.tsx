import type * as React from "react"
import { cleanup, fireEvent, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import { PackFooter } from "./PackFooter"

const totals = { clips: 129, frames: 13834, seconds: 577 }
const wrapper = ({ children }: { children: React.ReactNode }) => <TooltipProvider>{children}</TooltipProvider>
const footer = (bytes: number | null, format: string | null) =>
  render(
    <PackFooter
      totals={totals}
      bytes={bytes}
      format={format}
      picks={0}
      canExport
      onClearPicks={() => {}}
      onExport={() => {}}
      onShowExport={() => {}}
    />,
    { wrapper },
  )

afterEach(cleanup)

describe("PackFooter", () => {
  it("names the format its size is about", () => {
    footer(13.2e9, "ProRes 422 HQ")
    expect(screen.getByText("9:37 · about 12.3 GB (ProRes 422 HQ)")).toBeTruthy()
  })

  it("shows the size alone while the formats load, and no size without one", () => {
    footer(13.2e9, null)
    expect(screen.getByText("9:37 · about 12.3 GB")).toBeTruthy()
    cleanup()
    footer(null, "ProRes 422 HQ")
    expect(screen.getByText("9:37")).toBeTruthy()
  })

  it("shows how far the pack's export is, and opens it rather than exporting again", () => {
    const [onExport, onShowExport] = [vi.fn(), vi.fn()]
    const exporting = (state: "queued" | "running", progress: number | null) =>
      render(
        <PackFooter
          totals={totals}
          bytes={null}
          format={null}
          picks={0}
          canExport={false}
          exporting={{ state, progress }}
          onClearPicks={() => {}}
          onExport={onExport}
          onShowExport={onShowExport}
        />,
        { wrapper },
      )
    exporting("running", 0.12)
    fireEvent.click(screen.getByRole("button", { name: "Exporting… 12%" }))
    expect(onShowExport).toHaveBeenCalledOnce()
    expect(onExport).not.toHaveBeenCalled()
    cleanup()
    exporting("queued", null)
    expect(screen.getByRole("button", { name: "Export queued" })).toBeTruthy()
    expect(screen.queryByRole("button", { name: /^Export$/ })).toBeNull()
  })
})
