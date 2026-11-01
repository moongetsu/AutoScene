import { act, renderHook } from "@testing-library/react"
import { beforeEach, describe, expect, it } from "vitest"
import type { JobInfo } from "@/bindings"
import { useActivity, useProjectIndexProgress } from "./activity"

const job = (o: Partial<JobInfo>): JobInfo => ({
  id: "j1",
  kind: "export",
  title: "Exporting 1 clip",
  subtitle: null,
  projectId: "p",
  mediaId: null,
  exportId: null,
  state: "queued",
  progress: null,
  stage: null,
  etaMs: null,
  error: null,
  createdAt: 1,
  finishedAt: null,
  rev: 1,
  ...o,
})

describe("activity store", () => {
  beforeEach(() => useActivity.setState({ jobs: {}, order: [] }))

  it("ignores an event that arrives after a newer one", () => {
    const s = useActivity.getState()
    s.upsert(job({ state: "done", rev: 3 }))
    s.upsert(job({ state: "running", rev: 2 }))
    expect(useActivity.getState().jobs.j1.state).toBe("done")
  })

  it("ignores stale progress", () => {
    const s = useActivity.getState()
    s.upsert(job({ state: "running", progress: 0.5, rev: 5 }))
    s.progress({ id: "j1", rev: 4, progress: 0.1, stage: null, etaMs: null })
    expect(useActivity.getState().jobs.j1.progress).toBe(0.5)
    s.progress({ id: "j1", rev: 6, progress: 0.7, stage: "Rendering", etaMs: 10 })
    expect(useActivity.getState().jobs.j1.progress).toBe(0.7)
  })

  it("keeps the frames a job reads only while it tells them", () => {
    const s = useActivity.getState()
    s.upsert(job({ kind: "ingest", state: "running", rev: 1 }))
    const frames = { done: 1200, total: 34_000, rate: 23.976, perSec: null }
    s.progress({ id: "j1", rev: 2, progress: 0.03, stage: "Indexing frames", etaMs: null, frames })
    expect(useActivity.getState().jobs.j1.frames).toEqual(frames)
    s.progress({ id: "j1", rev: 3, progress: 0.96, stage: "Creating poster", etaMs: null, frames: null })
    expect(useActivity.getState().jobs.j1.frames).toBeNull()
  })

  it("merges a late snapshot without losing newer events", () => {
    const s = useActivity.getState()
    s.upsert(job({ id: "new", state: "running", createdAt: 5, rev: 9 }))
    s.upsert(job({ id: "j1", state: "done", rev: 8 }))
    s.replaceAll([job({ id: "j1", state: "running", rev: 7 })])
    const st = useActivity.getState()
    expect(st.jobs.j1.state).toBe("done")
    expect(st.order).toEqual(["new", "j1"])
  })
})

describe("a project's indexing", () => {
  beforeEach(() => useActivity.setState({ jobs: {}, order: [] }))

  it("is as far as its running indexing jobs are, on average", () => {
    const { result } = renderHook(() => useProjectIndexProgress("p"))
    expect(result.current).toBeNull()
    act(() => {
      const s = useActivity.getState()
      s.upsert(job({ id: "a", kind: "ingest", state: "running", progress: 0.2 }))
      s.upsert(job({ id: "b", kind: "ingest", state: "running", progress: 0.6 }))
      // Not counted: waiting, no progress yet, not indexing, another project.
      s.upsert(job({ id: "c", kind: "ingest", state: "queued", progress: null }))
      s.upsert(job({ id: "d", kind: "ingest", state: "running", progress: null }))
      s.upsert(job({ id: "e", kind: "analyze", state: "running", progress: 0.9 }))
      s.upsert(job({ id: "f", kind: "ingest", state: "running", progress: 0.9, projectId: "q" }))
    })
    expect(result.current).toBeCloseTo(0.4)
  })
})
