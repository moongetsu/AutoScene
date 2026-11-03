import { useState } from "react"
import { toast } from "sonner"
import type { PluginInfo, PluginReview } from "@/bindings"
import { CircleNotch, DotsThree, FileText, FolderOpen, Plus, PuzzlePiece, SlidersHorizontal, Trash } from "@/components/icons"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { DropdownMenu, DropdownMenuContent, DropdownMenuTrigger } from "@/components/ui/dropdown-menu"
import { type MenuKit, contextKit, dropdownKit } from "@/components/ui/menu-actions"
import { EmptyState } from "@/components/ui/empty-state"
import { Switch } from "@/components/ui/switch"
import { InstallReviewDialog } from "@/features/plugins/InstallReviewDialog"
import { PluginIcon } from "@/features/plugins/PluginIcon"
import { allowUnreviewed, keepPlugin } from "@/features/plugins/plugin-actions"
import { hasPage, reviewedPackage, trustLabel } from "@/features/plugins/plugin-items"
import { useAppActions } from "@/lib/actions"
import { commands, errorMessage } from "@/lib/ipc"
import { pickFolder, pickPluginFile } from "@/lib/media"
import { reveal } from "@/lib/os"
import { qk, queryClient, usePlugins, useSettings } from "@/lib/queries"
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
          <SettingsGroup title="Installed">
            {plugins.map((p) => (
              <PluginRow key={p.id} p={p} onUninstall={() => setRemoving(p)} />
            ))}
          </SettingsGroup>
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

function PluginRow({ p, onUninstall }: { p: PluginInfo; onUninstall: () => void }) {
  const { goSettings } = useAppActions()
  const trust = trustLabel(p)
  // Only AutoScene's review stands behind the name a plugin gives its author.
  const by = p.author && (reviewedPackage(p) ? `By ${p.author}` : `Says it is by ${p.author}`)
  const facts = [by, `version ${p.version}`].filter(Boolean).join(", ")
  const why = p.broken ?? (p.enabled ? p.blockedReason : null)
  const setEnabled = async (on: boolean) => {
    try {
      keepPlugin(await commands.pluginSetEnabled(p.id, on))
    } catch (e) {
      toast.error(on ? "Couldn't turn the plugin on" : "Couldn't turn the plugin off", { description: errorMessage(e) })
    }
  }
  return (
    <ContextArea menu={<PluginItems kit={contextKit} p={p} onUninstall={onUninstall} />}>
      <div role="group" aria-label={p.name}>
        <SettingRow
          leading={
            <span className="flex size-10 shrink-0 items-center justify-center rounded-xl bg-white/[0.06] text-fg-2 [&_svg]:size-5">
              <PluginIcon src={p.icon} className="size-6" />
            </span>
          }
          label={
            <span className="flex items-center gap-2.5">
              <span className="truncate">{p.name}</span>
              <Badge tone={trust.tone}>{trust.label}</Badge>
            </span>
          }
          description={
            <>
              {p.description && <span className="block">{p.description}</span>}
              <span className="block text-fg-3">{facts}</span>
              {why && <span className="mt-1 block text-danger">{why}</span>}
            </>
          }
        >
          {hasPage(p) && (
            <Button size="sm" variant="ghost" onClick={() => goSettings(`plugin.${p.id}`)}>
              Settings
            </Button>
          )}
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
        </SettingRow>
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
