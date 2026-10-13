import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Button } from "@/components/ui/button"
import { Badge } from "@/components/ui/badge"
import { Progress } from "@/components/ui/progress"
import { formatBytes } from "@/lib/format"
import { useUpdates } from "@/state/updates"

export function UpdateDialog() {
  const u = useUpdates()
  const status = u.status
  if (!status) return null

  const busy = u.phase === "downloading" || u.phase === "installing"
  const fraction = u.total ? u.downloaded / u.total : null

  return (
    <Dialog open={u.dialogOpen} onOpenChange={(o) => !busy && u.setDialogOpen(o)}>
      <DialogContent
        hideClose={busy}
        title={
          <span className="flex items-center gap-2">
            AutoScene {status.version} is available
            {status.simulated && <Badge tone="outline">Simulated</Badge>}
          </span>
        }
        description={`You have ${status.currentVersion}${status.date ? ` · released ${status.date}` : ""}`}
      >
        {status.notes && (
          <div
            data-selectable
            className="max-h-56 overflow-y-auto rounded-md border border-hairline bg-inset p-3 text-sm whitespace-pre-wrap text-fg-2"
          >
            {status.notes}
          </div>
        )}

        {(busy || u.phase === "ready") && (
          <div className="mt-4 space-y-2">
            <Progress value={u.phase === "installing" || u.phase === "ready" ? 1 : fraction} />
            <p className="text-xs text-fg-3 tabular">
              {u.phase === "downloading" &&
                (u.total ? `${formatBytes(u.downloaded)} of ${formatBytes(u.total)}` : formatBytes(u.downloaded))}
              {u.phase === "installing" && "Installing…"}
              {u.phase === "ready" && "Installed. Restart to finish."}
            </p>
          </div>
        )}

        {u.phase === "error" && u.error && <p className="mt-4 text-sm text-danger">{u.error}</p>}

        <DialogFooter>
          {u.phase === "ready" ? (
            <Button variant="primary" onClick={() => void u.relaunch()}>
              Restart now
            </Button>
          ) : (
            <>
              <Button variant="ghost" disabled={busy} onClick={u.skipVersion}>
                Skip this version
              </Button>
              <Button disabled={busy} onClick={u.remindLater}>
                Remind me later
              </Button>
              <Button variant="primary" disabled={busy} onClick={() => void u.install()}>
                {busy ? "Updating…" : "Update now"}
              </Button>
            </>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  )
}
