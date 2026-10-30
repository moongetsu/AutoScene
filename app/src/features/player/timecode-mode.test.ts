import { act } from "@testing-library/react"
import { beforeEach, describe, expect, it } from "vitest"
import { useTimecodeMode } from "./timecode-mode"

describe("useTimecodeMode", () => {
  beforeEach(() => {
    act(() => {
      useTimecodeMode.getState().setMode("elapsed")
    })
  })

  it("cycles through all modes in order", () => {
    expect(useTimecodeMode.getState().mode).toBe("elapsed")

    act(() => useTimecodeMode.getState().cycleMode())
    expect(useTimecodeMode.getState().mode).toBe("remaining")

    act(() => useTimecodeMode.getState().cycleMode())
    expect(useTimecodeMode.getState().mode).toBe("frames")

    act(() => useTimecodeMode.getState().cycleMode())
    expect(useTimecodeMode.getState().mode).toBe("percent")

    act(() => useTimecodeMode.getState().cycleMode())
    expect(useTimecodeMode.getState().mode).toBe("elapsed")
  })

  it("allows setting a specific mode directly", () => {
    act(() => useTimecodeMode.getState().setMode("remaining"))
    expect(useTimecodeMode.getState().mode).toBe("remaining")

    act(() => useTimecodeMode.getState().setMode("percent"))
    expect(useTimecodeMode.getState().mode).toBe("percent")
  })
})

