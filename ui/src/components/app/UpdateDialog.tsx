import {
  AlertTriangle,
  ArrowDownToLine,
  CheckCircle2,
  Clock,
  RotateCw,
  Sparkles,
} from "lucide-react"

import { Badge } from "@/components/ui/badge"
import { Button } from "@/components/ui/button"
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog"
import { Progress } from "@/components/ui/progress"
import { ScrollArea } from "@/components/ui/scroll-area"
import { useUpdates } from "@/state/updates"
import { formatBytes, formatReleaseDate } from "@/lib/updates"

/**
 * The single update surface — raised automatically when a new version is
 * found on launch, and by "Check for Updates" in the macOS menu or Settings.
 *
 * The three choices are deliberately different, not three ways to say no:
 *
 * - **Update Now** — download, install, restart.
 * - **Update Later** — stop offering *this* version. It stays installable from
 *   Check for Updates, and a newer version will still prompt.
 * - **Remind Me Later** — nothing is remembered; ask again in 24 hours.
 */
export function UpdateDialog() {
  const {
    status,
    phase,
    progress,
    error,
    checking,
    dialogOpen,
    manual,
    supported,
    updateNow,
    updateLater,
    remindMeLater,
    restart,
    closeDialog,
  } = useUpdates()

  // Esc / overlay click during a download would leave the install running with
  // no way back to it, so the dialog locks itself down while working.
  const locked = phase === "working"

  if (!supported) return null

  const available = status?.state === "available"
  const notes = status?.notes?.trim()
  const released = formatReleaseDate(status?.date ?? null)

  return (
    <Dialog
      open={dialogOpen}
      onOpenChange={(open) => {
        if (!open && !locked) closeDialog()
      }}
    >
      <DialogContent
        showCloseButton={!locked}
        className="sm:max-w-lg"
        onEscapeKeyDown={(e) => locked && e.preventDefault()}
        onPointerDownOutside={(e) => locked && e.preventDefault()}
        onInteractOutside={(e) => locked && e.preventDefault()}
      >
        {phase === "error" ? (
          <ErrorFace error={error} onClose={closeDialog} />
        ) : phase === "staged" ? (
          <StagedFace version={status?.version ?? null} onRestart={restart} />
        ) : phase === "simulated" ? (
          <SimulatedFace version={status?.version ?? null} onClose={closeDialog} />
        ) : phase === "working" ? (
          <WorkingFace version={status?.version ?? null} progress={progress} />
        ) : available ? (
          <AvailableFace
            version={status?.version ?? ""}
            currentVersion={status?.currentVersion ?? ""}
            notes={notes}
            released={released}
            simulated={status?.simulated ?? false}
            onUpdateNow={() => void updateNow()}
            onUpdateLater={updateLater}
            onRemindMeLater={remindMeLater}
          />
        ) : (
          <UpToDateFace
            currentVersion={status?.currentVersion ?? ""}
            checking={checking}
            manual={manual}
            onClose={closeDialog}
          />
        )}
      </DialogContent>
    </Dialog>
  )
}

function AvailableFace({
  version,
  currentVersion,
  notes,
  released,
  simulated,
  onUpdateNow,
  onUpdateLater,
  onRemindMeLater,
}: {
  version: string
  currentVersion: string
  notes?: string
  released: string | null
  simulated: boolean
  onUpdateNow: () => void
  onUpdateLater: () => void
  onRemindMeLater: () => void
}) {
  return (
    <>
      <DialogHeader>
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-primary/12 text-primary ring-1 ring-primary/25">
            <Sparkles className="size-3.5" />
          </span>
          <DialogTitle className="text-sm">AutoScene {version} is available</DialogTitle>
          {simulated ? (
            <Badge variant="secondary" className="text-[10px]">
              Simulated
            </Badge>
          ) : null}
        </div>
        <DialogDescription>
          You are on {currentVersion || "an earlier build"}
          {released ? ` · released ${released}` : ""}.
        </DialogDescription>
      </DialogHeader>

      {notes ? (
        <div className="rounded-lg border border-border/45 bg-background/40">
          <p className="border-b border-border/40 px-3 py-2 text-[10px] uppercase tracking-[0.18em] text-muted-foreground">
            What's new
          </p>
          <ScrollArea className="max-h-56">
            <div className="whitespace-pre-wrap px-3 py-2.5 text-xs leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
              {notes}
            </div>
          </ScrollArea>
        </div>
      ) : null}

      <DialogFooter className="sm:justify-between">
        <Button variant="ghost" size="sm" onClick={onUpdateLater}>
          Update Later
        </Button>
        <div className="flex flex-col-reverse gap-2 sm:flex-row">
          <Button variant="outline" size="sm" onClick={onRemindMeLater}>
            <Clock className="size-3" />
            Remind Me Later
          </Button>
          <Button size="sm" onClick={onUpdateNow}>
            <ArrowDownToLine className="size-3" />
            Update Now
          </Button>
        </div>
      </DialogFooter>

      <p className="text-[11px] leading-relaxed text-muted-foreground">
        <span className="text-foreground/70">Update Later</span> stops offering this
        version — you can still install it from Check for Updates.{" "}
        <span className="text-foreground/70">Remind Me Later</span> asks again
        tomorrow.
      </p>
    </>
  )
}

function WorkingFace({
  version,
  progress,
}: {
  version: string | null
  progress: { downloaded: number; total: number | null } | null
}) {
  const total = progress?.total ?? null
  const downloaded = progress?.downloaded ?? 0
  // No content-length means an indeterminate bar rather than a fake number.
  const pct = total && total > 0 ? Math.min(100, (downloaded / total) * 100) : null

  return (
    <>
      <DialogHeader>
        <DialogTitle className="text-sm">
          Downloading AutoScene {version ?? ""}
        </DialogTitle>
        <DialogDescription>
          Keep AutoScene open. It will restart itself once the update is ready.
        </DialogDescription>
      </DialogHeader>

      <div className="space-y-2">
        <Progress value={pct ?? 100} className={pct === null ? "animate-pulse" : undefined} />
        <p className="timecode text-[11px] tabular-nums text-muted-foreground">
          {pct === null || total === null
            ? `${formatBytes(downloaded)} downloaded`
            : `${formatBytes(downloaded)} of ${formatBytes(total)} · ${Math.round(pct)}%`}
        </p>
      </div>
    </>
  )
}

function StagedFace({
  version,
  onRestart,
}: {
  version: string | null
  onRestart: () => void | Promise<void>
}) {
  return (
    <>
      <DialogHeader>
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-primary/12 text-primary ring-1 ring-primary/25">
            <CheckCircle2 className="size-3.5" />
          </span>
          <DialogTitle className="text-sm">
            AutoScene {version ?? ""} is ready
          </DialogTitle>
        </div>
        <DialogDescription>
          Restart to finish installing. Any scan in progress will be stopped.
        </DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button size="sm" onClick={() => void onRestart()}>
          <RotateCw className="size-3" />
          Restart Now
        </Button>
      </DialogFooter>
    </>
  )
}

function SimulatedFace({
  version,
  onClose,
}: {
  version: string | null
  onClose: () => void
}) {
  return (
    <>
      <DialogHeader>
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-primary/12 text-primary ring-1 ring-primary/25">
            <CheckCircle2 className="size-3.5" />
          </span>
          <DialogTitle className="text-sm">Simulation complete</DialogTitle>
        </div>
        <DialogDescription>
          A real update would have installed {version ?? "the new version"} and
          restarted AutoScene here. Nothing was changed — this run came from{" "}
          <code className="font-mono">~/.autoscene/updates.json</code>. Delete
          that file to go back to the live update feed.
        </DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button size="sm" onClick={onClose}>
          Done
        </Button>
      </DialogFooter>
    </>
  )
}

function UpToDateFace({
  currentVersion,
  checking,
  manual,
  onClose,
}: {
  currentVersion: string
  checking: boolean
  manual: boolean
  onClose: () => void
}) {
  return (
    <>
      <DialogHeader>
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-primary/12 text-primary ring-1 ring-primary/25">
            <CheckCircle2 className="size-3.5" />
          </span>
          <DialogTitle className="text-sm">
            {checking ? "Checking for updates…" : "AutoScene is up to date"}
          </DialogTitle>
        </div>
        <DialogDescription>
          {checking
            ? "Asking the update server what the latest version is."
            : `You are running ${currentVersion || "the latest build"}${
                manual ? "." : " — the newest version available."
              }`}
        </DialogDescription>
      </DialogHeader>
      <DialogFooter>
        <Button size="sm" variant="outline" disabled={checking} onClick={onClose}>
          Close
        </Button>
      </DialogFooter>
    </>
  )
}

function ErrorFace({ error, onClose }: { error: string | null; onClose: () => void }) {
  return (
    <>
      <DialogHeader>
        <div className="flex items-center gap-2">
          <span className="flex size-7 items-center justify-center rounded-lg bg-destructive/12 text-destructive ring-1 ring-destructive/25">
            <AlertTriangle className="size-3.5" />
          </span>
          <DialogTitle className="text-sm">Could not check for updates</DialogTitle>
        </div>
        <DialogDescription>
          AutoScene still works — this only affects updating. Check your
          connection and try again.
        </DialogDescription>
      </DialogHeader>
      {error ? (
        <p className="rounded-lg border border-border/45 bg-background/40 px-3 py-2 font-mono text-[11px] leading-relaxed text-muted-foreground [overflow-wrap:anywhere]">
          {error}
        </p>
      ) : null}
      <DialogFooter>
        <Button size="sm" variant="outline" onClick={onClose}>
          Close
        </Button>
      </DialogFooter>
    </>
  )
}
