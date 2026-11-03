import type { Permission, PluginInfo, PluginUpdate, Trust } from "@/bindings"

/**
 * How far a plugin is trusted, as its badge says it. A loaded folder says
 * that first: it is never reviewed. A signature proves no one's name (the
 * key may come in the package), so a signed plugin isn't marked as vouched for.
 */
export function trustLabel(p: Pick<PluginInfo, "trust" | "source">): {
  label: string
  tone: "neutral" | "danger" | "outline"
} {
  if (p.source === "folder") return { label: "Loaded from a folder", tone: "outline" }
  return TRUST[p.trust]
}

const TRUST: Record<Trust, { label: string; tone: "neutral" | "danger" | "outline" }> = {
  reviewed: { label: "Reviewed by AutoScene", tone: "neutral" },
  pinned: { label: "Signed, not reviewed", tone: "outline" },
  unreviewed: { label: "Not reviewed", tone: "danger" },
}

/**
 * What a plugin's trust means, in one or two sentences for its review. A
 * signed update of a signed plugin used its pinned key (another is
 * refused); a first signature only brings a key along.
 */
export function trustLine(p: Pick<PluginInfo, "trust" | "source">, update?: Pick<PluginUpdate, "trustBefore" | "sourceBefore"> | null): string {
  if (p.source === "folder") return "It runs from its folder, as the files there are now. AutoScene has not reviewed it."
  switch (p.trust) {
    case "reviewed":
      return "AutoScene reviewed this version."
    case "pinned":
      return update?.trustBefore === "pinned" && update.sourceBefore === "package"
        ? "Signed with the same key as the installed version. AutoScene has not reviewed it."
        : "Signed with a key that came in the package: nothing shows who holds it. AutoScene has not reviewed it. Its updates must be signed with the same key."
    case "unreviewed":
      return "Nobody AutoScene knows signed it, and AutoScene has not reviewed it."
  }
}

/** Whether AutoScene stands behind what a plugin says of itself, its author's name too: only its review does. */
export function reviewedPackage(p: Pick<PluginInfo, "trust" | "source">): boolean {
  return p.source === "package" && p.trust === "reviewed"
}

const PERMISSIONS = new Map<Permission, string>([
  ["exports:read", "Reads the exported clips"],
  ["exports:write", "Changes or adds files beside the exported clips"],
  ["project:read", "Reads the project: its episodes and characters"],
  ["network", "Talks to the internet"],
])

/** A permission a plugin declares, in plain words. */
export function permissionLabel(p: Permission): string {
  return PERMISSIONS.get(p) ?? p
}

/** Whether a plugin has a page of its own in Settings: its settings, or its export actions to run after every export. */
export function hasPage(p: Pick<PluginInfo, "settings" | "exportActions">): boolean {
  return p.settings.length > 0 || p.exportActions.length > 0
}

/** The plugins Settings lists a page of: each that has one, also while it can't run (its page says why). */
export function pluginPages(plugins: PluginInfo[] | undefined): PluginInfo[] {
  return (plugins ?? []).filter(hasPage)
}

/** An entry a plugin adds to each export's menu. */
export type ExportActionItem = {
  key: string
  pluginId: string
  pluginName: string
  icon: string | null
  actionId: string
  title: string
}

/** The export actions of the plugins that may run now, plugin by plugin, in the order each lists them. */
export function exportActionItems(plugins: PluginInfo[] | undefined): ExportActionItem[] {
  return (plugins ?? [])
    .filter((p) => p.active)
    .flatMap((p) =>
      p.exportActions.map((a) => ({
        key: `${p.id}.${a.id}`,
        pluginId: p.id,
        pluginName: p.name,
        icon: p.icon,
        actionId: a.id,
        title: a.title,
      })),
    )
}
