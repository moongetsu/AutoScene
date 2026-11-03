import { describe, expect, it } from "vitest"
import { pluginInfo } from "@/test/plugins"
import { pluginPages } from "./plugin-items"

describe("plugin pages", () => {
  it("are listed for the plugins that have something to set, also those that can't run now", () => {
    const action = { id: "post", title: "Post", description: null, afterExport: false }
    const setting = { key: "loud", title: "Loud", description: null, kind: { type: "boolean", default: false } } as const
    const plugins = [
      pluginInfo({ id: "dev.a.settings", name: "Settings", settings: [setting] }),
      pluginInfo({ id: "dev.a.action", name: "Action", exportActions: [action] }),
      // Nothing to set; turned off, its page says so.
      pluginInfo({ id: "dev.a.bare", name: "Bare" }),
      pluginInfo({ id: "dev.a.off", name: "Off", active: false, settings: [setting] }),
    ]
    expect(pluginPages(plugins).map((p) => p.id)).toEqual(["dev.a.settings", "dev.a.action", "dev.a.off"])
    expect(pluginPages(undefined)).toEqual([])
  })
})
