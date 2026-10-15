import { describe, expect, it, vi } from "vitest"
import { firstFailure } from "./queries"

const ok = () => ({ isError: false, error: null, refetch: vi.fn() })
const failed = (error: unknown) => ({ isError: true, error, refetch: vi.fn() })

describe("firstFailure", () => {
  it("is nothing while every query is fine (or still loading)", () => {
    expect(firstFailure(ok(), ok())).toBeNull()
  })

  it("names the first failure and fetches only the failed queries again", () => {
    const [a, b, c] = [ok(), failed("disk"), failed("db")]
    const f = firstFailure(a, b, c)!
    expect(f.error).toBe("disk")
    f.retry()
    expect(a.refetch).not.toHaveBeenCalled()
    expect(b.refetch).toHaveBeenCalledOnce()
    expect(c.refetch).toHaveBeenCalledOnce()
  })

  it("keeps what a query showed when only its refetch failed", () => {
    expect(firstFailure(ok(), { ...failed("offline"), data: [] })).toBeNull()
  })
})
