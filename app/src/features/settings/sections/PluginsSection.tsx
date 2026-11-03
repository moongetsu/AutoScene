import { useMemo, useState } from "react"
import { toast } from "sonner"
import type { PluginInfo, PluginReview } from "@/bindings"
import { ArrowCounterClockwise, ArrowSquareOut, CircleNotch, Copy, DotsThree, FileText, FolderOpen, GearSix, GithubLogo, MagnifyingGlass, Plus, PuzzlePiece, SlidersHorizontal, Trash, User, X } from "@/components/icons"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { type MenuKit, contextKit, dropdownKit } from "@/components/ui/menu-actions"
import { EmptyState } from "@/components/ui/empty-state"
import { Input } from "@/components/ui/input"
import { Switch } from "@/components/ui/switch"
import { InstallReviewDialog } from "@/features/plugins/InstallReviewDialog"
import { PluginIcon } from "@/features/plugins/PluginIcon"
import { allowUnreviewed, keepPlugin } from "@/features/plugins/plugin-actions"
import { hasPage, reviewedPackage, trustLabel } from "@/features/plugins/plugin-items"
import { useAppActions } from "@/lib/actions"
import { commands, errorMessage } from "@/lib/ipc"
import { pickFolder, pickPluginFile } from "@/lib/media"
import { copyText, openExternalUrl, reveal } from "@/lib/os"
import { qk, queryClient, usePlugins, useSettings } from "@/lib/queries"
import { cn } from "@/lib/utils"
import { SettingRow, SettingsGroup } from "../primitives"

const ALLOW = "Allow plugins that AutoScene has not reviewed"


export function PluginsSection() {
  const { data: settings } = useSettings()
  const { data: plugins, error } = usePlugins()
  const allowed = settings?.allowUnreviewedPlugins ?? false
  // Which picker's choice is being checked: its button spins.
  const [checking, setChecking] = useState<"file" | "folder" | null>(null)
  const [review, setReview] = useState<PluginReview | null>(null)
  const [removing, setRemoving] = useState<PluginInfo | null>(null)
  // While it uninstalls it stays open: closed, it would uninstall all the same.
  const [uninstalling, setUninstalling] = useState(false)
  const [togglingAll, setTogglingAll] = useState(false)
  const [refreshing, setRefreshing] = useState(false)
  const [search, setSearch] = useState("")
  const [filter, setFilter] = useState<"all" | "enabled" | "disabled" | "gpu" | "folder">("all")

  const counts = useMemo(() => {
    if (!plugins) return { all: 0, enabled: 0, disabled: 0, gpu: 0, folder: 0 }
    return {
      all: plugins.length,
      enabled: plugins.filter((p) => p.enabled).length,
      disabled: plugins.filter((p) => !p.enabled).length,
      gpu: plugins.filter((p) => p.gpu).length,
      folder: plugins.filter((p) => p.source.type === "folder").length,
    }
  }, [plugins])

  const filteredPlugins = useMemo(() => {
    if (!plugins) return []
    const q = search.trim().toLowerCase()
    return plugins.filter((p) => {
      if (filter === "enabled" && !p.enabled) return false
      if (filter === "disabled" && p.enabled) return false
      if (filter === "gpu" && !p.gpu) return false
      if (filter === "folder" && p.source.type !== "folder") return false
      if (!q) return true
      return (
        p.name.toLowerCase().includes(q) ||
        p.id.toLowerCase().includes(q) ||
        p.author.toLowerCase().includes(q) ||
        p.description.toLowerCase().includes(q)
      )
    })
  }, [plugins, search, filter])

  const inspect = async (
    what: "file" | "folder",
    pick: () => Promise<string | null>,
    check: (path: string) => Promise<PluginReview>,
  ) => {
    const path = await pick()
    if (!path) return
    setChecking(what)
    try {
      setReview(await check(path))
    } catch (e) {
      toast.error("Couldn't open the plugin", { description: errorMessage(e) })
    } finally {
      setChecking(null)
    }
  }

  const toggleAll = async (targetState: boolean) => {
    if (!plugins || plugins.length === 0) return
    const targets = plugins.filter((p) => p.enabled !== targetState)
    if (targets.length === 0) return
    setTogglingAll(true)
    try {
      await Promise.all(
        targets.map(async (p) => {
          keepPlugin(await commands.pluginSetEnabled(p.id, targetState))
        }),
      )
    } catch (e) {
      toast.error("Couldn't change all plugins", { description: errorMessage(e) })
    } finally {
      setTogglingAll(false)
    }
  }

  const refreshList = async () => {
    setRefreshing(true)
    try {
      await queryClient.invalidateQueries({ queryKey: qk.plugins })
    } finally {
      setTimeout(() => setRefreshing(false), 300)
    }
  }

  return (
    <>
      <div className="-mt-4 mb-8 flex items-start justify-between gap-6">
        <p className="max-w-md text-md text-fg-2">
          Plugins add actions to exports and commands to the palette. Each is a program that runs on this computer.
        </p>
        <div className="flex shrink-0 items-center gap-2">
          {allowed && (
            <Button
              size="sm"
              variant="ghost"
              disabled={checking != null}
              onClick={() => void inspect("folder", () => pickFolder("Choose the plugin's folder"), commands.pluginInspectFolder)}
            >
              {checking === "folder" ? <CircleNotch className="animate-spin" /> : <FolderOpen />} Load a folder…
            </Button>
          )}
          <Button size="sm" disabled={checking != null} onClick={() => void inspect("file", pickPluginFile, commands.pluginInspect)}>
            {checking === "file" ? <CircleNotch className="animate-spin" /> : <Plus />} Install a plugin…
          </Button>
        </div>
      </div>

      <SettingsGroup title="Who may run code here">
        <SettingRow
          label={ALLOW}
          description="Unsigned plugins, and ones only their author signed, and plugins loaded from a folder. AutoScene can't check what they do."
        >
          {/* On, AutoScene asks in a dialog of the system's own: the page can't say yes alone. */}
          <Switch aria-label={ALLOW} checked={allowed} disabled={!settings} onCheckedChange={(on) => void allowUnreviewed(on)} />
        </SettingRow>
      </SettingsGroup>

      {error ? (
        <EmptyState icon={<PuzzlePiece />} tone="danger" title="Couldn't read the plugins" description={errorMessage(error)} />
      ) : plugins?.length === 0 ? (
        <EmptyState
          icon={<PuzzlePiece />}
          title="No plugins yet"
          description="Install a plugin from its .asp file. One AutoScene hasn't reviewed needs the setting above first. Its actions then show in each export's menu, and its commands in the palette."
        />
      ) : (
        plugins && (
          <section className="mb-10">
            <div className="mb-4 flex flex-col gap-3 px-1 sm:flex-row sm:items-center sm:justify-between">
              <div className="flex items-center gap-3">
                <div className="flex items-center gap-2">
                  <h3 className="text-lg font-semibold text-fg-1">Installed</h3>
                  <span className="rounded-full bg-white/[0.08] px-2 py-0.5 text-xs font-medium text-fg-2">
                    {filteredPlugins.length}
                  </span>
                </div>

                {plugins.length > 0 && (
                  <div className="flex items-center gap-1 border-l border-white/[0.08] pl-3">
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={togglingAll}
                      onClick={() => void toggleAll(true)}
                      className="h-7 px-2 text-xs text-fg-3 hover:text-fg-1"
                    >
                      Enable all
                    </Button>
                    <span className="text-xs text-white/20">·</span>
                    <Button
                      size="sm"
                      variant="ghost"
                      disabled={togglingAll}
                      onClick={() => void toggleAll(false)}
                      className="h-7 px-2 text-xs text-fg-3 hover:text-fg-1"
                    >
                      Disable all
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      title="Refresh plugins list"
                      aria-label="Refresh plugins list"
                      disabled={refreshing}
                      onClick={() => void refreshList()}
                      className="size-7 p-0 text-fg-3 hover:text-fg-1"
                    >
                      <ArrowCounterClockwise className={cn("size-3.5", refreshing && "animate-spin")} />
                    </Button>
                  </div>
                )}
              </div>

              {plugins.length > 0 && (
                <div className="relative w-full sm:w-64">
                  <MagnifyingGlass className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-fg-3" />
                  <Input
                    type="text"
                    placeholder="Search plugins…"
                    value={search}
                    onChange={(e) => setSearch(e.target.value)}
                    className="h-8 w-full bg-white/[0.04] pr-8 pl-9 text-xs placeholder:text-fg-3 focus:bg-white/[0.07]"
                  />
                  {search && (
                    <button
                      type="button"
                      onClick={() => setSearch("")}
                      className="absolute top-1/2 right-2.5 -translate-y-1/2 rounded p-0.5 text-fg-3 hover:text-fg-1"
                    >
                      <X className="size-3" />
                    </button>
                  )}
                </div>
              )}
            </div>

            {plugins.length > 0 && (
              <div className="mb-4 flex flex-wrap items-center gap-1.5 px-1">
                {(
                  [
                    { id: "all", label: "All", count: counts.all },
                    { id: "enabled", label: "Enabled", count: counts.enabled },
                    { id: "disabled", label: "Disabled", count: counts.disabled },
                    ...(counts.gpu > 0 ? [{ id: "gpu", label: "GPU", count: counts.gpu }] : []),
                    ...(counts.folder > 0 ? [{ id: "folder", label: "Folder", count: counts.folder }] : []),
                  ] as const
                ).map((tab) => (
                  <button
                    key={tab.id}
                    type="button"
                    onClick={() => setFilter(tab.id)}
                    className={cn(
                      "inline-flex items-center gap-1.5 rounded-lg px-2.5 py-1 text-xs font-medium transition-colors",
                      filter === tab.id
                        ? "bg-white/15 text-fg-1 shadow-sm"
                        : "bg-white/[0.04] text-fg-2 hover:bg-white/[0.08] hover:text-fg-1",
                    )}
                  >
                    <span>{tab.label}</span>
                    <span
                      className={cn(
                        "rounded px-1 text-[10px] tabular-nums",
                        filter === tab.id ? "bg-white/20 text-fg-1" : "bg-white/[0.06] text-fg-3",
                      )}
                    >
                      {tab.count}
                    </span>
                  </button>
                ))}
              </div>
            )}

            {filteredPlugins.length === 0 ? (
              <div className="rounded-2xl border border-white/[0.06] bg-white/[0.02] p-8 text-center">
                <p className="text-sm text-fg-2">No plugins found matching &ldquo;{search || filter}&rdquo;.</p>
                <Button
                  size="sm"
                  variant="ghost"
                  onClick={() => {
                    setSearch("")
                    setFilter("all")
                  }}
                  className="mt-3 text-xs"
                >
                  Clear search and filter
                </Button>
              </div>
            ) : (
              <div className="grid grid-cols-1 gap-4 md:grid-cols-2">
                {filteredPlugins.map((p) => (
                  <PluginCard key={p.id} p={p} onUninstall={() => setRemoving(p)} />
                ))}
              </div>
            )}
          </section>
        )
      )}

      <InstallReviewDialog review={review} onClose={() => setReview(null)} />
      <Dialog open={removing != null} onOpenChange={(open) => !open && !uninstalling && setRemoving(null)}>
        {removing && (
          <UninstallForm
            key={removing.id}
            p={removing}
            busy={uninstalling}
            setBusy={setUninstalling}
            onDone={() => setRemoving(null)}
          />
        )}
      </Dialog>
    </>
  )
}

/** What can be done with an installed plugin: its right-click menu and its ⋯ menu list the same. */
function PluginItems({ kit, p, onUninstall }: { kit: MenuKit; p: PluginInfo; onUninstall: () => void }) {
  const { goSettings } = useAppActions()
  const { Item, Separator } = kit
  return (
    <>
      <Item onSelect={() => reveal(p.folder)}>
        <FolderOpen /> Show its folder
      </Item>
      <Item onSelect={() => reveal(p.logPath)}>
        <FileText /> Show its log
      </Item>
      {hasPage(p) && (
        <Item onSelect={() => goSettings(`plugin.${p.id}`)}>
          <SlidersHorizontal /> Its settings
        </Item>
      )}
      <Separator />
      <Item destructive onSelect={onUninstall}>
        <Trash /> Uninstall…
      </Item>
    </>
  )
}

function PluginCard({ p, onUninstall }: { p: PluginInfo; onUninstall: () => void }) {
  const { goSettings } = useAppActions()
  const trust = trustLabel(p)
  // Only AutoScene's review stands behind the name a plugin gives its author.
  const by = p.author && (reviewedPackage(p) ? `By ${p.author}` : `Says it is by ${p.author}`)
  const facts = [by, `version ${p.version}`].filter(Boolean).join(", ")
  const why = p.broken ?? (p.enabled ? p.blockedReason : null)
  const isGithub = p.repository?.includes("github.com")
  const avatarUrl =
    p.authorAvatar ??
    (p.authorUrl?.includes("github.com/")
      ? `${p.authorUrl.replace(/\/$/, "")}.png`
      : undefined)
  const setEnabled = async (on: boolean) => {
    try {
      keepPlugin(await commands.pluginSetEnabled(p.id, on))
    } catch (e) {
      toast.error(on ? "Couldn't turn the plugin on" : "Couldn't turn the plugin off", { description: errorMessage(e) })
    }
  }
  return (
    <ContextArea menu={<PluginItems kit={contextKit} p={p} onUninstall={onUninstall} />}>
      <div
        role="group"
        aria-label={p.name}
        className="group relative flex flex-col justify-between overflow-hidden rounded-2xl border border-white/[0.08] bg-white/[0.03] shadow-sm transition-all duration-200 hover:border-white/20 hover:bg-white/[0.05]"
      >
        {p.banner && (
          <div className="relative aspect-[21/9] w-full overflow-hidden border-b border-white/[0.06] bg-black/40">
            <img src={p.banner} alt="" draggable={false} className="size-full object-cover transition-transform duration-300 group-hover:scale-105" />
            <div className="absolute inset-0 bg-gradient-to-t from-black/80 via-black/20 to-transparent" />
            <div className="absolute top-3 right-3 flex items-center gap-1.5">
              <Badge tone={trust.tone}>{trust.label}</Badge>
              {p.gpu && (
                <span className="rounded border border-accent/40 bg-black/60 px-1.5 py-0.5 text-[11px] font-medium text-accent backdrop-blur-md">
                  GPU
                </span>
              )}
            </div>
          </div>
        )}

        <div className="flex flex-1 flex-col p-5">
          <div className="flex items-start gap-3.5">
            <span
              style={p.color ? { backgroundColor: p.color } : undefined}
              className={cn(
                "flex size-11 shrink-0 items-center justify-center rounded-xl shadow-sm ring-1 ring-white/10 [&_img]:size-7 [&_svg]:size-6",
                p.color ? "text-fg-1" : "bg-white/[0.08] text-fg-1",
              )}
            >
              <PluginIcon src={p.icon} />
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center justify-between gap-2">
                <h4 className="truncate font-semibold text-fg-1 text-base leading-snug">{p.name}</h4>
                {!p.banner && (
                  <div className="flex items-center gap-1.5">
                    <Badge tone={trust.tone}>{trust.label}</Badge>
                    {p.gpu && (
                      <span className="rounded border border-accent/30 bg-accent/10 px-1.5 py-0.5 text-[11px] font-medium text-accent">
                        GPU
                      </span>
                    )}
                  </div>
                )}
                {p.banner && (
                  <span className="shrink-0 rounded border border-white/10 bg-white/[0.05] px-1.5 py-0.5 font-mono text-[11px] text-fg-3">
                    v{p.version}
                  </span>
                )}
              </div>
              <div className="mt-1 flex items-center gap-1.5 text-xs text-fg-3">
                {!p.banner && (
                  <span className="shrink-0 rounded border border-white/10 bg-white/[0.05] px-1.5 py-0.5 font-mono text-[11px] text-fg-3">
                    v{p.version}
                  </span>
                )}
                {p.author && (
                  avatarUrl ? (
                    <img
                      src={avatarUrl}
                      alt=""
                      loading="lazy"
                      className="size-3.5 shrink-0 rounded-full border border-white/10 bg-white/5 object-cover"
                      onError={(e) => (e.currentTarget.style.display = "none")}
                    />
                  ) : (
                    <span className="flex size-3.5 shrink-0 items-center justify-center rounded-full bg-white/10 text-fg-3">
                      <User className="size-2" />
                    </span>
                  )
                )}
                {p.authorUrl ? (
                  <button
                    type="button"
                    onClick={() => void openExternalUrl(p.authorUrl!)}
                    className="truncate font-medium text-fg-2 hover:text-fg-1 hover:underline"
                  >
                    {by}
                  </button>
                ) : (
                  <span className="truncate">{by}</span>
                )}
                <span className="hidden opacity-0">{facts}</span>
              </div>
            </div>
          </div>

          {p.description && (
            <p className="mt-3 text-sm text-fg-2 line-clamp-2 leading-relaxed">
              {p.description}
            </p>
          )}

          {why && <span className="mt-2 block text-xs text-danger">{why}</span>}
        </div>

        <div className="flex items-center justify-between border-t border-white/[0.06] bg-white/[0.01] px-5 py-3">
          <div className="flex items-center gap-1.5">
            {hasPage(p) && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => goSettings(`plugin.${p.id}`)}
                className="h-8 gap-1.5 px-2.5 text-xs text-fg-2 hover:text-fg-1"
              >
                <GearSix className="size-3.5" />
                Settings
              </Button>
            )}
            {p.repository && (
              <Button
                size="sm"
                variant="ghost"
                onClick={() => void openExternalUrl(p.repository!)}
                className="h-8 gap-1.5 px-2.5 text-xs text-fg-2 hover:text-fg-1"
              >
                {isGithub ? <GithubLogo className="size-3.5" /> : <ArrowSquareOut className="size-3.5" />}
                Repo
              </Button>
            )}
            <Button
              size="sm"
              variant="ghost"
              title="Show plugin folder"
              aria-label={`Open ${p.name} folder`}
              onClick={() => reveal(p.folder)}
              className="size-8 p-0 text-fg-3 hover:text-fg-1"
            >
              <FolderOpen className="size-3.5" />
            </Button>
            <Button
              size="sm"
              variant="ghost"
              title="Show plugin log"
              aria-label={`Open ${p.name} log`}
              onClick={() => reveal(p.logPath)}
              className="size-8 p-0 text-fg-3 hover:text-fg-1"
            >
              <FileText className="size-3.5" />
            </Button>
          </div>
          <div className="flex items-center gap-2">
            <Switch aria-label={`Enable ${p.name}`} checked={p.enabled} onCheckedChange={(on) => void setEnabled(on)} />
            <DropdownMenu>
              <DropdownMenuTrigger
                aria-label={`${p.name} options`}
                className="flex size-8 items-center justify-center rounded-full text-fg-3 transition-colors hover:bg-white/[0.08] hover:text-fg-1 data-[state=open]:bg-white/[0.08]"
              >
                <DotsThree className="size-5" weight="bold" />
              </DropdownMenuTrigger>
              <DropdownMenuContent>
                <PluginItems kit={dropdownKit} p={p} onUninstall={onUninstall} />
              </DropdownMenuContent>
            </DropdownMenu>
          </div>
        </div>
      </div>
    </ContextArea>
  )
}

function UninstallForm({
  p,
  busy,
  setBusy,
  onDone,
}: {
  p: PluginInfo
  busy: boolean
  setBusy: (busy: boolean) => void
  onDone: () => void
}) {
  const [deleteData, setDeleteData] = useState(false)
  const remove = async () => {
    setBusy(true)
    try {
      await commands.pluginUninstall(p.id, deleteData)
      queryClient.setQueryData<PluginInfo[]>(qk.plugins, (list) => list?.filter((x) => x.id !== p.id))
      onDone()
      toast(`Uninstalled ${p.name}`)
    } catch (e) {
      toast.error("Couldn't uninstall the plugin", { description: errorMessage(e) })
    } finally {
      setBusy(false)
    }
  }
  return (
    <DialogContent
      title={`Uninstall ${p.name}?`}
      description={
        p.source === "folder"
          ? "AutoScene forgets it and its settings. Its folder stays as it is."
          : "Its files and its settings go. What it made beside your exports stays."
      }
    >
      <label className="flex items-center justify-between gap-4 rounded-lg border border-hairline bg-inset px-3.5 py-3">
        <span>
          <span className="block text-sm text-fg-1">Delete what it kept too</span>
          <span className="block text-xs text-fg-3">The files it saved for itself between runs.</span>
        </span>
        <Switch aria-label="Delete what it kept too" checked={deleteData} onCheckedChange={setDeleteData} />
      </label>
      <DialogFooter>
        <Button variant="ghost" disabled={busy} onClick={onDone}>
          Cancel
        </Button>
        <Button variant="danger" disabled={busy} onClick={() => void remove()}>
          {busy && <CircleNotch className="animate-spin" />}
          Uninstall
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}
