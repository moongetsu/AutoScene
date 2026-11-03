/**
 * Mock plugins for the browser backend, shaped like `plugins.rs`: one
 * reviewed plugin installed, packages to install (each .asp picked offers
 * the next: one AutoScene hasn't reviewed and one its author signed, as
 * every real package is for now, then a reviewed one and an update of the
 * installed one), a developer's folder to load, and what each run says when
 * it ends. The jobs are the backend's to run.
 */
import type { ActionKind, Permission, PluginInfo, PluginReview, PluginUpdate, SettingSpec, SettingValue } from "@/bindings"

/** A plugin as installed or offered: what the setting decides is worked out when it is shown. */
type MockPlugin = Omit<PluginInfo, "active" | "blockedReason">

const DATA = String.raw`C:\Users\you\AppData\Local\AutoScene`
const NOT_ALLOWED = "Plugins that AutoScene has not reviewed are turned off in Settings."

/** A plain rounded square in `fill`, as a package's SVG icon arrives (base64). */
const icon = (fill: string) =>
  `data:image/svg+xml;base64,${btoa(`<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><rect x="2" y="2" width="20" height="20" rx="6" fill="${fill}"/><circle cx="12" cy="12" r="4" fill="#0b0b0c"/></svg>`)}`

function plugin(o: Pick<MockPlugin, "id" | "name" | "version" | "description" | "trust"> & Partial<MockPlugin>): MockPlugin {
  return {
    author: "AutoScene",
    license: "MIT",
    repository: null,
    authorKeyId: null,
    source: "package",
    folder: `${DATA}\\plugins\\${o.id}\\${o.version}+1a2b3c4d`,
    enabled: true,
    broken: null,
    permissions: [],
    networkHosts: [],
    gpu: false,
    uvScript: true,
    exportActions: [],
    commands: [],
    settings: [],
    icon: null,
    logPath: `${DATA}\\logs\\plugins\\${o.id}.log`,
    warnings: [],
    ...o,
  }
}

const HELLO_SETTINGS: SettingSpec[] = [
  {
    key: "greeting",
    title: "Greeting",
    description: "What the note beside each export starts with.",
    kind: { type: "string", default: "Hello", placeholder: "Hello", maxLength: 80 },
  },
  {
    key: "clips",
    title: "Clips to name",
    description: "How many of the export's clips the note names.",
    kind: { type: "number", default: 3, min: 1, max: 10, step: 1, integer: true },
  },
  {
    key: "pause",
    title: "Wait before writing",
    description: "In seconds.",
    kind: { type: "number", default: 0.5, min: 0, max: null, step: null, integer: false },
  },
  { key: "signed", title: "Sign the note", description: null, kind: { type: "boolean", default: true } },
  {
    key: "style",
    title: "Style",
    description: null,
    kind: {
      type: "select",
      options: [
        { value: "plain", label: "Plain" },
        { value: "friendly", label: "Friendly" },
        { value: "formal", label: "Formal" },
      ],
      default: "plain",
    },
  },
]

const hello = (version: string, more: Partial<MockPlugin> = {}) =>
  plugin({
    id: "dev.autoscene.hello-export",
    name: "Hello Export",
    version,
    trust: "reviewed",
    description: "Writes a short note beside each export. An example of what plugins can do.",
    repository: "https://github.com/tcmmi/autoscene",
    permissions: ["exports:read", "exports:write"],
    icon: icon("#e0b45c"),
    exportActions: [{ id: "hello", title: "Say hello", description: "Writes hello.txt beside the export.", afterExport: false }],
    commands: [{ id: "greet", title: "Say hello to the project", description: null, needsProject: true }],
    settings: HELLO_SETTINGS,
    ...more,
  })

/** What each .asp picked in the mock is, in the order they are offered. */
const PACKAGES: { path: string; plugin: MockPlugin }[] = [
  {
    path: String.raw`C:\Plugins\discord-notify-unreviewed.asp`,
    plugin: plugin({
      id: "dev.example.discord-notify",
      name: "Discord Notify",
      version: "0.3.1",
      trust: "unreviewed",
      author: "Example Dev",
      license: null,
      description: "Posts each finished export's clip list to a Discord channel.",
      permissions: ["exports:read", "network"],
      networkHosts: ["discord.com"],
      icon: icon("#7c8cf5"),
      exportActions: [{ id: "post", title: "Post to Discord", description: "Posts the clip list to the channel.", afterExport: false }],
      settings: [
        {
          key: "webhook",
          title: "Webhook address",
          description: "From the channel's settings in Discord: Integrations, then Webhooks.",
          kind: { type: "string", default: null, placeholder: "https://discord.com/api/webhooks/…", maxLength: 2000 },
        },
        {
          key: "mention",
          title: "Mention",
          description: null,
          kind: {
            type: "select",
            options: [
              { value: "none", label: "Nobody" },
              { value: "here", label: "Everyone online" },
              { value: "everyone", label: "Everyone" },
              { value: "role", label: "The editors' role" },
            ],
            default: "none",
          },
        },
      ],
      warnings: ["Pages need a newer AutoScene."],
    }),
  },
  {
    path: String.raw`C:\Plugins\frame-guard.asp`,
    plugin: plugin({
      id: "dev.someone.frame-guard",
      name: "Frame Guard",
      version: "2.0.0",
      trust: "pinned",
      author: "Someone",
      authorKeyId: "4B2D9C1E0F3A7E51",
      description: "Checks each exported clip for dropped or repeated frames.",
      permissions: ["exports:read"],
      gpu: true,
      uvScript: false,
      exportActions: [{ id: "check", title: "Check for dropped frames", description: null, afterExport: false }],
    }),
  },
  {
    path: String.raw`C:\Plugins\shot-notes.asp`,
    plugin: plugin({
      id: "dev.autoscene.shot-notes",
      name: "Shot Notes",
      version: "1.2.0",
      trust: "reviewed",
      description: "Lists each clip's episode and timecodes in a text file beside the export.",
      permissions: ["exports:read", "exports:write", "project:read"],
      icon: icon("#5fc58f"),
      exportActions: [{ id: "notes", title: "Write shot notes", description: null, afterExport: false }],
      commands: [{ id: "whats-new", title: "What's new in Shot Notes", description: null, needsProject: false }],
    }),
  },
  {
    path: String.raw`C:\Plugins\hello-export-1.1.0.asp`,
    plugin: hello("1.1.0", {
      permissions: ["exports:read", "exports:write", "network"],
      networkHosts: ["hooks.example.com"],
      description: "Writes a short note beside each export, and can send it on. An example of what plugins can do.",
    }),
  },
]

/** The developer's folder "Load a folder…" picks. */
export const MOCK_PLUGIN_FOLDER = String.raw`D:\Plugins\my-plugin`

const folderPlugin = (path: string) =>
  plugin({
    id: "dev.me.my-plugin",
    name: "My Plugin",
    version: "0.1.0",
    trust: "unreviewed",
    author: "You",
    description: "A plugin being written, loaded from its folder.",
    source: "folder",
    folder: path,
    exportActions: [{ id: "try", title: "Try my plugin", description: null, afterExport: false }],
    commands: [{ id: "hello", title: "Run my plugin", description: null, needsProject: false }],
  })

/** What a run of each action says when it ends, by plugin and action. */
const MESSAGES = new Map([
  ["dev.autoscene.hello-export hello", "Wrote hello.txt beside the export."],
  ["dev.autoscene.hello-export greet", "Said hello to the project."],
  ["dev.autoscene.shot-notes notes", "Wrote the shot notes for every clip."],
  ["dev.autoscene.shot-notes whats-new", "Shot Notes 1.2 names each clip's episode."],
  ["dev.example.discord-notify post", "Posted the clip list to Discord."],
  ["dev.someone.frame-guard check", "No frame was dropped or repeated."],
])

const installed = new Map<string, MockPlugin>([[hello("1.0.0").id, hello("1.0.0")]])
const tickets = new Map<string, MockPlugin>()
/** The settings the person changed, by plugin. */
const changed = new Map<string, Map<string, SettingValue>>()
let offered = 0

/** Whether AutoScene reviewed `p`: a loaded folder never is. */
export function reviewed(p: MockPlugin | undefined): boolean {
  return p?.source === "package" && p.trust === "reviewed"
}

function info(p: MockPlugin, allow: boolean): PluginInfo {
  const blocked = !reviewed(p) && !allow
  return { ...p, active: p.enabled && p.broken == null && !blocked, blockedReason: blocked && p.broken == null ? NOT_ALLOWED : null }
}

function get(id: string): MockPlugin {
  const p = installed.get(id)
  if (!p) throw { code: "not_found", message: `${id} isn't installed.` }
  return p
}

export function mockPlugin(id: string): MockPlugin | undefined {
  return installed.get(id)
}

export function pluginsList(allow: boolean): PluginInfo[] {
  return [...installed.values()].sort((a, b) => a.id.localeCompare(b.id)).map((p) => info(p, allow))
}

/** The next package the file dialog offers. */
export function nextPluginPackage(): string {
  return PACKAGES[offered++ % PACKAGES.length].path
}

function review(p: MockPlugin, allow: boolean): PluginReview {
  const previous = installed.get(p.id)
  const ticketId = crypto.randomUUID()
  // As installed, it keeps what the version before had turned on.
  const next = previous
    ? {
        ...p,
        enabled: previous.enabled,
        exportActions: p.exportActions.map((a) => ({
          ...a,
          afterExport: previous.exportActions.some((b) => b.id === a.id && b.afterExport),
        })),
      }
    : p
  tickets.set(ticketId, next)
  const update: PluginUpdate | null = previous
    ? {
        fromVersion: previous.version,
        trustBefore: previous.source === "folder" ? "unreviewed" : previous.trust,
        sourceBefore: previous.source,
        addedPermissions: p.permissions.filter((x: Permission) => !previous.permissions.includes(x)),
        addedHosts: p.networkHosts.filter((h) => !previous.networkHosts.includes(h)),
      }
    : null
  const needsUnreviewed = !reviewed(next)
  return { ticketId, plugin: info(next, allow), update, needsUnreviewed, allowed: !needsUnreviewed || allow, uvMissing: false }
}

export function inspectPackage(path: string, allow: boolean): PluginReview {
  const offer = PACKAGES.find((x) => x.path === path)
  if (!offer) throw { code: "invalid", message: "This isn't an AutoScene plugin package." }
  return review(offer.plugin, allow)
}

export function inspectFolder(path: string, allow: boolean): PluginReview {
  if (!allow) throw { code: "invalid", message: NOT_ALLOWED }
  return review(folderPlugin(path), allow)
}

export function installPlugin(ticketId: string, allow: boolean, accept: boolean): PluginInfo {
  const p = tickets.get(ticketId)
  if (!p) throw { code: "not_found", message: "This review was closed. Open the plugin again." }
  tickets.delete(ticketId)
  if (!reviewed(p) && !allow) throw { code: "invalid", message: NOT_ALLOWED }
  if (!reviewed(p) && !accept) throw { code: "invalid", message: "Confirm that you want to install it." }
  installed.set(p.id, p)
  return info(p, allow)
}

export function discardTicket(ticketId: string) {
  tickets.delete(ticketId)
}

export function uninstallPlugin(id: string) {
  get(id)
  installed.delete(id)
  changed.delete(id)
}

export function setPluginEnabled(id: string, on: boolean, allow: boolean): PluginInfo {
  const p = { ...get(id), enabled: on }
  installed.set(id, p)
  return info(p, allow)
}

export function setAfterExport(id: string, action: string, on: boolean, allow: boolean): PluginInfo {
  const old = get(id)
  if (!old.exportActions.some((a) => a.id === action)) throw { code: "invalid", message: `The plugin has no export action ${action}.` }
  const p = { ...old, exportActions: old.exportActions.map((a) => (a.id === action ? { ...a, afterExport: on } : a)) }
  installed.set(id, p)
  return info(p, allow)
}

function defaultOf(s: SettingSpec): SettingValue | undefined {
  return s.kind.default ?? undefined
}

/** Each setting's value: the person's, else its default (none without one). */
export function pluginValues(id: string): Record<string, SettingValue> {
  const own = changed.get(id)
  const values: Record<string, SettingValue> = {}
  for (const s of get(id).settings) {
    const v = own?.get(s.key) ?? defaultOf(s)
    if (v !== undefined) values[s.key] = v
  }
  return values
}

/** Why `v` can't be the value of `s`, as `check_value` words it. */
function problem(s: SettingSpec, v: SettingValue): string | null {
  const k = s.kind
  switch (k.type) {
    case "string":
      return typeof v !== "string" ? "must be text" : v.length > k.maxLength ? `can be at most ${k.maxLength} characters` : null
    case "boolean":
      return typeof v !== "boolean" ? "must be on or off" : null
    case "select":
      return typeof v === "string" && k.options.some((o) => o.value === v) ? null : "must be one of its choices"
    case "number":
      if (typeof v !== "number" || !Number.isFinite(v)) return "must be a number"
      if (k.integer && !Number.isInteger(v)) return "must be a whole number"
      if (k.min != null && v < k.min) return `must be at least ${k.min}`
      if (k.max != null && v > k.max) return `must be at most ${k.max}`
      return null
  }
}

export function setPluginValues(id: string, values: Record<string, SettingValue>): Record<string, SettingValue> {
  const specs = get(id).settings
  const own = new Map(changed.get(id))
  for (const [key, v] of Object.entries(values)) {
    const s = specs.find((x) => x.key === key)
    if (!s) continue
    const why = problem(s, v)
    if (why) throw { code: "invalid", message: `${s.title} ${why}.` }
    if (v === defaultOf(s)) own.delete(key)
    else own.set(key, v)
  }
  changed.set(id, own)
  return pluginValues(id)
}

/**
 * The run of `action` of `id`, as `queue_run` checks it: its plugin may run
 * now, an export action has an export, a command never runs on one, and one
 * that needs a project has it. Its title, its plugin's name and what it says
 * when it ends.
 */
export function pluginRun(
  id: string,
  kind: ActionKind,
  action: string,
  projectId: string | null,
  exportId: string | null,
  allow: boolean,
): { title: string; name: string; message: string } {
  const p = get(id)
  const shown = info(p, allow)
  if (!shown.active) throw { code: "invalid", message: p.broken ?? shown.blockedReason ?? `${p.name} is turned off.` }
  const refuse = (message: string) => {
    throw { code: "invalid", message }
  }
  const spec = kind === "exportAction" ? p.exportActions.find((a) => a.id === action) : p.commands.find((c) => c.id === action)
  if (!spec) return refuse(`${p.name} has no such action.`)
  if (kind === "exportAction" && (!exportId || !projectId)) refuse("Choose an export to run it on.")
  if (kind === "command" && exportId) refuse("A command doesn't run on an export.")
  if (kind === "command" && "needsProject" in spec && spec.needsProject && !projectId) refuse("Open a project to run it in.")
  return { title: spec.title, name: p.name, message: MESSAGES.get(`${id} ${action}`) ?? `${spec.title} is done.` }
}

/** The export actions the plugins that may run now turned on for after every export. */
export function afterExportRuns(allow: boolean): { pluginId: string; actionId: string }[] {
  return pluginsList(allow)
    .filter((p) => p.active)
    .flatMap((p) => p.exportActions.filter((a) => a.afterExport).map((a) => ({ pluginId: p.id, actionId: a.id })))
}
