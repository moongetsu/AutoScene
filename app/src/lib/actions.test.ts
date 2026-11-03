import { describe, expect, it } from "vitest"
import { settingsSection } from "./actions"

describe("Settings links", () => {
  it("keep a plugin's page, and send what they don't know to General", () => {
    expect(settingsSection("plugins")).toBe("plugins")
    expect(settingsSection("plugin.dev.a.b")).toBe("plugin.dev.a.b")
    expect(settingsSection("plugin.dev.example.discord-notify")).toBe("plugin.dev.example.discord-notify")
    for (const unknown of ["", "nothing", "plugin.", "plugin.ab", "plugin.Dev.A.B", "plugin.dev/a", "pluginsdev.a.b"]) {
      expect(settingsSection(unknown), unknown).toBe("general")
    }
  })
})
