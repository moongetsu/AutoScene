import type * as React from "react"
import { useId, useState } from "react"
import { toast } from "sonner"
import type { PluginInfo, PluginReview, PluginUpdate } from "@/bindings"
import { CircleNotch } from "@/components/icons"
import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Field, Input } from "@/components/ui/input"
import { useAppActions } from "@/lib/actions"
import { commands, errorMessage, isAppError } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"
import { PluginIcon } from "./PluginIcon"
import { permissionLabel, trustLabel, trustLine } from "./plugin-items"

/**
 * A plugin package (or folder) checked for install, before the person says
 * yes: who stands behind it, what it says it does, and what it adds. One
 * AutoScene hasn't reviewed needs the setting, and its name typed. Closed
 * without installing, the review goes, and the files it unpacked.
 */
export function InstallReviewDialog({ review, onClose }: { review: PluginReview | null; onClose: () => void }) {
  // While it installs it stays open: closed, it would install all the same.
  const [busy, setBusy] = useState(false)
  const cancel = () => {
    // One gone already is no error; one left goes when it expires.
    if (review) void commands.pluginDiscard(review.ticketId).catch(() => undefined)
    onClose()
  }
  return (
    <Dialog open={review != null} onOpenChange={(open) => !open && !busy && cancel()}>
      {review && (
        <ReviewForm key={review.ticketId} review={review} busy={busy} setBusy={setBusy} onCancel={cancel} onInstalled={onClose} />
      )}
    </Dialog>
  )
}

/**
 * A plugin's name as it is typed to confirm it: the spaces around it, and an
 * ellipsis at its end (a long name is cut to one), don't count.
 */
function typedName(s: string): string {
  return s.normalize("NFC").trim().replace(/(?:…|\.\.\.)$/u, "").trimEnd()
}

function ReviewForm({
  review,
  busy,
  setBusy,
  onCancel,
  onInstalled,
}: {
  review: PluginReview
  busy: boolean
  setBusy: (busy: boolean) => void
  onCancel: () => void
  onInstalled: () => void
}) {
  const p = review.plugin
  const { goSettings } = useAppActions()
  const [typed, setTyped] = useState("")
  const nameId = useId()
  const confirmed = !review.needsUnreviewed || (typedName(typed) !== "" && typedName(typed) === typedName(p.name))
  // A developer's folder is loaded where it is, not installed.
  const verb = review.update ? "Update" : p.source === "folder" ? "Load" : "Install"
  const install = async () => {
    setBusy(true)
    try {
      const installed = await commands.pluginInstall(review.ticketId, review.needsUnreviewed)
      void queryClient.invalidateQueries({ queryKey: qk.plugins })
      onInstalled()
      toast(`${verb === "Update" ? "Updated" : verb === "Load" ? "Loaded" : "Installed"} ${installed.name}`)
    } catch (e) {
      toast.error("Couldn't install the plugin", { description: errorMessage(e) })
      // Only a refusal while the plugin runs keeps the review, to try again.
      if (!(isAppError(e) && e.code === "not_ready")) onCancel()
    } finally {
      setBusy(false)
    }
  }
  const trust = trustLabel(p)
  return (
    <DialogContent
      title={`${verb} ${p.name}?`}
      description={p.description || undefined}
      className="max-h-[calc(100vh-48px)] overflow-y-auto"
    >
      <div className="flex flex-col gap-5">
        <div className="flex items-start gap-4">
          <span className="flex size-12 shrink-0 items-center justify-center rounded-2xl bg-white/[0.06] text-fg-1 [&_svg]:size-6">
            <PluginIcon src={p.icon} className="size-7" />
          </span>
          <dl className="grid min-w-0 flex-1 grid-cols-[auto_1fr] gap-x-4 gap-y-1 text-sm">
            <Fact label="Id">
              <span data-selectable className="font-mono">
                {p.id}
              </span>
            </Fact>
            <Fact label="Version">
              {p.version}
              {review.update && <span className="text-fg-3"> · {review.update.fromVersion} is installed</span>}
            </Fact>
            <Fact label="Author">{p.author || "Not named"}</Fact>
            {p.authorKeyId && (
              <Fact label="Key">
                <span data-selectable className="font-mono">
                  {p.authorKeyId}
                </span>
              </Fact>
            )}
            {p.source === "folder" && (
              <Fact label="Folder">
                <span data-selectable className="font-mono break-all">
                  {p.folder}
                </span>
              </Fact>
            )}
          </dl>
        </div>

        <div className="flex flex-col gap-2 rounded-lg border border-hairline bg-inset px-3.5 py-3">
          <Badge tone={trust.tone} className="self-start">
            {trust.label}
          </Badge>
          <p className="text-sm text-fg-1">{trustLine(p)}</p>
          {review.update && <Before update={review.update} p={p} />}
        </div>

        <Permissions review={review} />
        <Adds p={p} />

        {p.warnings.length > 0 && (
          <section>
            <h3 className="text-sm font-medium text-fg-2">Skipped in its plugin.json</h3>
            <ul className="mt-1.5 list-disc space-y-1 pl-5 text-sm text-fg-2">
              {p.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          </section>
        )}

        {review.needsUnreviewed &&
          (review.allowed ? (
            <Field
              label={
                <>
                  Type <span data-selectable>{p.name}</span> to {verb.toLowerCase()} it
                </>
              }
              hint="AutoScene can't check what it does."
              htmlFor={nameId}
            >
              <Input id={nameId} value={typed} onChange={(e) => setTyped(e.target.value)} />
            </Field>
          ) : (
            <div className="flex items-center justify-between gap-4 rounded-lg border border-danger/30 bg-danger-subtle px-3.5 py-3">
              <p className="text-sm text-fg-1">Plugins that AutoScene has not reviewed are turned off in Settings.</p>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  onCancel()
                  goSettings("plugins")
                }}
              >
                Go to Plugins
              </Button>
            </div>
          ))}
      </div>
      <DialogFooter>
        <Button variant="ghost" disabled={busy} onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant={review.needsUnreviewed ? "danger" : "primary"}
          disabled={busy || !review.allowed || !confirmed}
          onClick={() => void install()}
        >
          {busy && <CircleNotch className="animate-spin" />}
          {verb}
        </Button>
      </DialogFooter>
    </DialogContent>
  )
}

/** What the installed version was, when the update changes who stands behind it or where its files are. */
function Before({ update, p }: { update: PluginUpdate; p: PluginInfo }) {
  if (update.trustBefore === p.trust && update.sourceBefore === p.source) return null
  const before = trustLabel({ trust: update.trustBefore, source: update.sourceBefore })
  return (
    <p className="text-sm text-fg-2">
      The installed version: {before.label}.
      {update.sourceBefore === "folder" && p.source === "package" && " This package takes its place; that folder stays as it is."}
    </p>
  )
}

function Fact({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <>
      <dt className="text-fg-3">{label}</dt>
      <dd className="min-w-0 text-fg-1">{children}</dd>
    </>
  )
}

/** What the plugin says it does, with what an update adds marked. */
function Permissions({ review }: { review: PluginReview }) {
  const p = review.plugin
  const added = review.update?.addedPermissions ?? []
  const newHosts = review.update?.addedHosts ?? []
  const hosts = p.networkHosts.filter((h) => !newHosts.includes(h))
  return (
    <section>
      <h3 className="text-sm font-medium text-fg-2">What it says it does</h3>
      <ul className="mt-1.5 flex flex-col gap-1.5 text-md text-fg-1">
        {p.permissions.map((perm) => (
          <li key={perm} className="flex flex-wrap items-center gap-x-2 gap-y-1">
            {permissionLabel(perm)}
            {added.includes(perm) && <Badge tone="accent">New</Badge>}
            {perm === "network" && hosts.length > 0 && <span className="text-sm text-fg-2">{hosts.join(", ")}</span>}
            {perm === "network" && newHosts.length > 0 && (
              <span className="text-sm text-accent">New: {newHosts.join(", ")}</span>
            )}
          </li>
        ))}
        {p.permissions.length === 0 && <li className="text-fg-2">It asks for no permissions.</li>}
        {p.gpu && <li>Uses the graphics card: its runs wait while an analysis runs</li>}
      </ul>
      <p className="mt-2 text-xs text-fg-3">Declared by the author, not enforced. The plugin runs with your user rights.</p>
    </section>
  )
}

/** The menu entries, palette commands and settings it adds. */
function Adds({ p }: { p: PluginInfo }) {
  const adds = [
    ...p.exportActions.map((a) => ({ key: `action-${a.id}`, where: "Each export's menu", what: a.title })),
    ...p.commands.map((c) => ({ key: `command-${c.id}`, where: "Command palette", what: c.title })),
  ]
  return (
    <section>
      <h3 className="text-sm font-medium text-fg-2">What it adds</h3>
      <ul className="mt-1.5 flex flex-col gap-1 text-md text-fg-1">
        {adds.map((a) => (
          <li key={a.key}>
            <span className="text-fg-2">{a.where}:</span> {a.what}
          </li>
        ))}
        {p.settings.length > 0 && (
          <li>
            <span className="text-fg-2">Settings:</span> {p.settings.length === 1 ? "1 setting" : `${p.settings.length} settings`}
          </li>
        )}
        {adds.length === 0 && p.settings.length === 0 && <li className="text-fg-2">No menu entries, commands or settings.</li>}
      </ul>
    </section>
  )
}
