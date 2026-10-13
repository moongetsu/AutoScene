import { beforeEach, describe, expect, it } from "vitest"
import type { JobInfo } from "@/bindings"
import { useActivity } from "./activity"

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
