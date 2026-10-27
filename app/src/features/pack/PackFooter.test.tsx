import type * as React from "react"
import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"
import { TooltipProvider } from "@/components/ui/tooltip"
import { PackFooter } from "./PackFooter"

const totals = { clips: 129, frames: 13834, seconds: 577 }
const wrapper = ({ children }: { children: React.ReactNode }) => <TooltipProvider>{children}</TooltipProvider>
const footer = (bytes: number | null, format: string | null) =>
  render(
    <PackFooter totals={totals} bytes={bytes} format={format} picks={0} canExport onClearPicks={() => {}} onExport={() => {}} />,
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
})
