import type { PluginInfo } from "@/bindings"

/** An installed plugin as `plugins_list` gives it: reviewed, on and with nothing it adds, unless `o` says otherwise. */
export function pluginInfo(o: Partial<PluginInfo> & Pick<PluginInfo, "id" | "name">): PluginInfo {
  return {
    version: "1.0.0",
    description: "",
    author: "Someone",
    license: null,
    repository: null,
    trust: "reviewed",
    authorKeyId: null,
    source: "package",
    folder: `C:\\AutoScene\\plugins\\${o.id}`,
    enabled: true,
    active: true,
    blockedReason: null,
    broken: null,
    permissions: [],
    networkHosts: [],
    gpu: false,
    uvScript: true,
    exportActions: [],
    commands: [],
    settings: [],
    icon: null,
    logPath: `C:\\AutoScene\\logs\\plugins\\${o.id}.log`,
    warnings: [],
    ...o,
  }
}
