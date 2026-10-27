import { useRef, useState } from "react"
import { act, cleanup, render } from "@testing-library/react"
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest"
import { fittedFontSize, useFittedTitle } from "./fit-text"

describe("fittedFontSize", () => {
  it("leaves a title that fits as it is", () => {
    expect(fittedFontSize(84, 400, 1000, 48)).toBeNull()
    expect(fittedFontSize(84, 1000, 1000, 48)).toBeNull()
  })

  it("makes a wider one smaller in proportion, never below the floor", () => {
    expect(fittedFontSize(84, 900, 760, 48)).toBe(70)
    expect(fittedFontSize(84, 1520, 760, 48)).toBe(48)
  })

  it("has nothing to fit into a row with no room", () => {
    expect(fittedFontSize(84, 900, 0, 48)).toBeNull()
  })
})

/** The title, below the row it fits into (as on the character page). */
function Title({ row, text }: { row: HTMLDivElement | null; text: string }) {
  const title = useRef<HTMLHeadingElement>(null)
  const size = useFittedTitle(title, row, text, 48)
  return (
    <h1 ref={title} style={size ? { fontSize: size } : undefined}>
      {text}
    </h1>
  )
}

/** A hero row: the title starts at 200, its icon takes 40 px after it. */
function Hero({ text }: { text: string }) {
  const [row, setRow] = useState<HTMLDivElement | null>(null)
  return (
    <div ref={setRow}>
      <button>
        <Title row={row} text={text} />
      </button>
    </div>
  )
}

const rect = (left: number, right: number) => ({ left, right, top: 0, bottom: 84, width: right - left, height: 84 }) as DOMRect
/** Where the row ends (the window's width). */
let rowRight = 1000
/** How wide the display face draws, against the stand-in before it loads. */
let face = 1
/** What the row's observers are told when it changes size. */
let resized: (() => void)[] = []
/** The title's size: its own, else its classes' 84 px. */
const sizeOf = (el: HTMLElement) => parseFloat(el.style.fontSize) || 84
/** Its text: 10 px a character at 84 px. */
const widthOf = (el: HTMLElement) => ((el.textContent ?? "").length * 10 * face * sizeOf(el)) / 84
const sizeIn = (container: HTMLElement) => container.querySelector("h1")!.style.fontSize

beforeEach(() => {
  rowRight = 1000
  face = 1
  resized = []
  vi.stubGlobal(
    "ResizeObserver",
    class {
      constructor(changed: () => void) {
        resized.push(changed)
      }
      observe() {}
      disconnect() {}
    },
  )
  vi.spyOn(HTMLElement.prototype, "clientWidth", "get").mockImplementation(() => rowRight)
  vi.spyOn(window, "getComputedStyle").mockImplementation(
    (el) => ({ fontSize: `${sizeOf(el as HTMLElement)}px` }) as CSSStyleDeclaration,
  )
  vi.spyOn(HTMLElement.prototype, "scrollWidth", "get").mockImplementation(function (this: HTMLElement) {
    return widthOf(this)
  })
  // The title's box ends where the room does (the rest is cut).
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    const title = this.tagName === "H1" ? this : this.querySelector("h1")
    const right = 200 + Math.min(rowRight - 240, title ? widthOf(title) : 0)
    if (this.tagName === "H1") return rect(200, right)
    return this.tagName === "BUTTON" ? rect(192, right + 40) : rect(0, rowRight)
  })
})

afterEach(() => {
  cleanup()
  vi.restoreAllMocks()
  vi.unstubAllGlobals()
  Reflect.deleteProperty(document, "fonts")
})

describe("useFittedTitle", () => {
  it("keeps the classes' size for a name that fits", () => {
    const { container } = render(<Hero text="Jesse Pinkman" />)
    expect(sizeIn(container)).toBe("")
  })

  it("fits a long name the moment the page opens with it", () => {
    // 900 px at 84 px into 760: 70 px.
    const { container, rerender } = render(<Hero text={"R".repeat(90)} />)
    expect(sizeIn(container)).toBe("70px")
    // Measured again when the name changes, from the size it has then.
    rerender(<Hero text={"R".repeat(80)} />)
    expect(sizeIn(container)).toBe("79px")
    rerender(<Hero text="Jesse Pinkman" />)
    expect(sizeIn(container)).toBe("")
  })

  it("fits again when the row gets narrower or wider", () => {
    const { container } = render(<Hero text={"R".repeat(90)} />)
    const resize = (right: number) => {
      rowRight = right
      act(() => resized.forEach((changed) => changed()))
    }
    expect(sizeIn(container)).toBe("70px")
    // 900 px at 84 px into 560: 52 px.
    resize(800)
    expect(sizeIn(container)).toBe("52px")
    resize(1500)
    expect(sizeIn(container)).toBe("")
  })

  it("fits again when the display face arrives", async () => {
    let arrived = () => {}
    const ready = new Promise<void>((r) => (arrived = r))
    Object.defineProperty(document, "fonts", { configurable: true, value: { ready } })
    // Drawn in a narrower stand-in, the name fits; in its own face it doesn't.
    const { container } = render(<Hero text={"R".repeat(60)} />)
    expect(sizeIn(container)).toBe("")
    face = 1.5
    await act(async () => arrived())
    expect(sizeIn(container)).toBe("70px")
  })

  it("stops at the floor and cuts the rest short", () => {
    const { container } = render(<Hero text={"R".repeat(200)} />)
    expect(sizeIn(container)).toBe("48px")
  })
})
