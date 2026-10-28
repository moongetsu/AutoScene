import { QueryClient, QueryClientProvider } from "@tanstack/react-query"
import { cleanup, render, screen } from "@testing-library/react"
import { afterEach, describe, expect, it, vi } from "vitest"
import type { ModelInfo } from "@/bindings"
import { AboutSection } from "./AboutSection"

const model = (id: string, purpose: string, license: string, nonCommercial: boolean, installed: boolean): ModelInfo => ({
  id,
  purpose,
  source: `${id} source`,
  license,
  nonCommercial,
  sizeBytes: 31_250_929,
  installed,
})

vi.mock("@/lib/ipc", () => ({
  commands: {
    appInfo: vi.fn(() => Promise.resolve({ version: "1.0.0", platform: "windows", arch: "x86_64" })),
    mediaTools: vi.fn(() => Promise.resolve({ ready: true, version: "8.0", path: "ffmpeg", error: null })),
    models: vi.fn(() =>
      Promise.resolve([
        model("transnetv2", "Shot boundaries", "MIT", false, true),
        model("arcface-glintr100", "Face identity embeddings", "InsightFace non-commercial research", true, false),
      ]),
    ),
  },
  errorMessage: (e: unknown) => String(e),
}))

vi.mock("@/lib/actions", () => ({ useAppActions: () => ({ openLogs: vi.fn() }) }))

afterEach(cleanup)

describe("AboutSection", () => {
  it("lists the AI models with their licences and marks the non-commercial ones", async () => {
    render(
      <QueryClientProvider client={new QueryClient()}>
        <AboutSection />
      </QueryClientProvider>,
    )
    expect(await screen.findByText("Face identity embeddings")).toBeTruthy()
    expect(screen.getByText("InsightFace non-commercial research")).toBeTruthy()
    expect(screen.getByText("Shot boundaries")).toBeTruthy()
    // Only the restricted model carries the mark.
    expect(screen.getAllByText("Non-commercial")).toHaveLength(1)
    expect(screen.getByText(/Fast Scan uses none of them/)).toBeTruthy()
    expect(screen.getByText(/^Downloaded · 29\.8 MB/)).toBeTruthy()
    expect(screen.getByText(/^Not downloaded yet/)).toBeTruthy()
  })
})
