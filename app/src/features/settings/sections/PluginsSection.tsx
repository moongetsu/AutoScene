import { useState } from "react"
import { toast } from "sonner"
import type { PluginInfo, PluginReview } from "@/bindings"
import { CircleNotch, FileText, FolderOpen, Plus, PuzzlePiece, SlidersHorizontal, Trash } from "@/components/icons"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem, ContextMenuSeparator } from "@/components/ui/context-menu"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { EmptyState } from "@/components/ui/empty-state"
import { Switch } from "@/components/ui/switch"
import { InstallReviewDialog } from "@/features/plugins/InstallReviewDialog"
import { PluginIcon } from "@/features/plugins/PluginIcon"
import { keepPlugin } from "@/features/plugins/plugin-actions"
import { hasPage, trustLabel } from "@/features/plugins/plugin-items"
import { useAppActions } from "@/lib/actions"
import { commands, errorMessage } from "@/lib/ipc"
import { pickFolder, pickPluginFile } from "@/lib/media"
import { reveal } from "@/lib/os"
import { qk, queryClient, usePlugins, useSettings } from "@/lib/queries"
import { SettingRow, SettingsGroup } from "../primitives"

const ALLOW = "Allow plugins that AutoScene has not reviewed"

/** Let plugins AutoScene hasn't reviewed run, or not (off, their runs stop). */
async function allow(on: boolean) {
  try {
    queryClient.setQueryData(qk.settings, await commands.pluginsAllowUnreviewed(on))
    void queryClient.invalidateQueries({ queryKey: qk.plugins })
  } catch (e) {
    toast.error("Couldn't save the setting", { description: errorMessage(e) })
  }
}

export function PluginsSection() {
  const { data: settings } = useSettings()
  const { data: plugins, error } = usePlugins()
  const allowed = settings?.allowUnreviewedPlugins ?? false
  const [asking, setAsking] = useState(false)
  const [checking, setChecking] = useState(false)
  const [review, setReview] = useState<PluginReview | null>(null)
  const [removing, setRemoving] = useState<PluginInfo | null>(null)

  const inspect = async (pick: () => Promise<string | null>, check: (path: string) => Promise<PluginReview>) => {
    const path = await pick()
    if (!path) return
    setChecking(true)
    try {
      setReview(await check(path))
    } catch (e) {
      toast.error("Couldn't open the plugin", { description: errorMessage(e) })
    } finally {
      setChecking(false)
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
              disabled={checking}
              onClick={() => void inspect(() => pickFolder("Choose the plugin's folder"), commands.pluginInspectFolder)}
            >
              <FolderOpen /> Load a folder…
            </Button>
          )}
          <Button size="sm" disabled={checking} onClick={() => void inspect(pickPluginFile, commands.pluginInspect)}>
            {checking ? <CircleNotch className="animate-spin" /> : <Plus />} Install a plugin…
          </Button>
        </div>
      </div>

      <SettingsGroup title="Who may run code here">
        <SettingRow
          label={ALLOW}
          description="Unsigned plugins, and ones only their author signed, and plugins loaded from a folder. AutoScene can't check what they do."
        >
          <Switch
            aria-label={ALLOW}
            checked={allowed}
            disabled={!settings}
            onCheckedChange={(on) => (on ? setAsking(true) : void allow(false))}
          />
        </SettingRow>
      </SettingsGroup>

      {error ? (
        <EmptyState icon={<PuzzlePiece />} tone="danger" title="Couldn't read the plugins" description={errorMessage(error)} />
      ) : plugins?.length === 0 ? (
        <EmptyState
          icon={<PuzzlePiece />}
          title="No plugins yet"
          description="Install a plugin from its .asp file. Its actions then show in each export's menu, and its commands in the palette."
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

      <Dialog open={asking} onOpenChange={setAsking}>
        <DialogContent
          title="Allow plugins AutoScene has not reviewed?"
          description="They run with your user rights: they can read, change and send anything you can."
        >
          <p className="text-md text-fg-2">
            Install only plugins from people you trust. Each asks you to type its name before it installs, and turning this
            off again stops them.
          </p>
          <DialogFooter>
            <Button variant="ghost" onClick={() => setAsking(false)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              onClick={() => {
                setAsking(false)
                void allow(true)
              }}
            >
              Allow them
            </Button>
          </DialogFooter>
        </DialogContent>
      </Dialog>
      <InstallReviewDialog review={review} onClose={() => setReview(null)} />
      <Dialog open={removing != null} onOpenChange={(open) => !open && setRemoving(null)}>
        {removing && <UninstallForm key={removing.id} p={removing} onDone={() => setRemoving(null)} />}
      </Dialog>
    </>
  )
}

function PluginRow({ p, onUninstall }: { p: PluginInfo; onUninstall: () => void }) {
  const { goSettings } = useAppActions()
  const trust = trustLabel(p)
  const facts = [p.author && `By ${p.author}`, `version ${p.version}`].filter(Boolean).join(", ")
  const why = p.broken ?? (p.enabled ? p.blockedReason : null)
  const setEnabled = async (on: boolean) => {
    try {
      keepPlugin(await commands.pluginSetEnabled(p.id, on))
    } catch (e) {
      toast.error(on ? "Couldn't turn the plugin on" : "Couldn't turn the plugin off", { description: errorMessage(e) })
    }
  }
  return (
    <ContextArea
      menu={
        <>
          <ContextMenuItem onSelect={() => reveal(p.folder)}>
            <FolderOpen /> Show its folder
          </ContextMenuItem>
          <ContextMenuItem onSelect={() => reveal(p.logPath)}>
            <FileText /> Show its log
          </ContextMenuItem>
          {hasPage(p) && (
            <ContextMenuItem onSelect={() => goSettings(`plugin.${p.id}`)}>
              <SlidersHorizontal /> Its settings
            </ContextMenuItem>
          )}
          <ContextMenuSeparator />
          <ContextMenuItem destructive onSelect={onUninstall}>
            <Trash /> Uninstall…
          </ContextMenuItem>
        </>
      }
    >
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
        </SettingRow>
      </div>
    </ContextArea>
  )
}

function UninstallForm({ p, onDone }: { p: PluginInfo; onDone: () => void }) {
  const [deleteData, setDeleteData] = useState(false)
  const [busy, setBusy] = useState(false)
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
        <Button variant="ghost" onClick={onDone}>
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
