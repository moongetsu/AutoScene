import { cleanup, render } from "@testing-library/react"
import { afterEach, describe, expect, it } from "vitest"
import { PluginIcon } from "./PluginIcon"

afterEach(cleanup)

const PNG = "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mNk+M9QDwADhgGAWjR9awAAAABJRU5ErkJggg=="
const SVG = `data:image/svg+xml;base64,${btoa('<svg xmlns="http://www.w3.org/2000/svg"/>')}`

describe("a plugin's icon", () => {
  it("never renders anything but an img of a data URL", () => {
    for (const src of [PNG, SVG]) {
      const { container } = render(<PluginIcon src={src} />)
      const img = container.querySelector("img")
      expect(img?.getAttribute("src")).toBe(src)
      expect(img?.getAttribute("alt")).toBe("")
      expect(img?.getAttribute("draggable")).toBe("false")
      // The picture alone: nothing drawn inline beside it.
      expect(container.querySelector("svg, object, embed, iframe")).toBeNull()
      cleanup()
    }
    const refused = [
      null,
      "",
      "https://example.com/icon.png",
      "javascript:alert(1)",
      "data:text/html;base64,PHNjcmlwdD5hbGVydCgxKTwvc2NyaXB0Pg==",
      // An SVG not in base64 could carry quotes and markup.
      `data:image/svg+xml;utf8,<svg onload="alert(1)"/>`,
      `${SVG}"><script>alert(1)</script>`,
      "DATA:IMAGE/PNG;BASE64,iVBORw0KGgo=",
    ]
    for (const src of refused) {
      const { container } = render(<PluginIcon src={src} />)
      // The puzzle piece, from the app's own icons.
      expect(container.querySelector("img, object, embed, iframe, script")).toBeNull()
      expect(container.querySelectorAll("svg")).toHaveLength(1)
      expect(container.innerHTML).not.toContain("alert")
      cleanup()
    }
  })
})
