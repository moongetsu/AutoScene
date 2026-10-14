import type * as React from "react"
import { useNavigate } from "@tanstack/react-router"
import { Activity, AlertTriangle, Check, CircleSlash, Eye, Film, ScanSearch, Scissors, X } from "lucide-react"
import type { JobInfo } from "@/bindings"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { Progress } from "@/components/ui/progress"
import { Sheet, SheetContent } from "@/components/ui/sheet"
import { Tooltip } from "@/components/ui/tooltip"
import { formatEta, formatPercent } from "@/lib/format"
import { commands } from "@/lib/ipc"
import { cn } from "@/lib/utils"
import { isFinished, useActivity, useJobs } from "@/state/activity"
import { useUi } from "@/state/ui"

const KIND_ICON: Record<JobInfo["kind"], React.ReactNode> = {
  ingest: <Film />,
  proxy: <Eye />,
  export: <Scissors />,
  analyze: <ScanSearch />,
}

/** Global view of background work (indexing, previews, analysis, exports). */
export function ActivityDrawer() {
  const open = useUi((s) => s.activityOpen)
  const setOpen = useUi((s) => s.setActivityOpen)
  const jobs = useJobs()
  const active = jobs.filter((j) => !isFinished(j))
  const finished = jobs.filter(isFinished)

  const clear = () => {
    void commands.jobsClearFinished()
    useActivity.getState().clearFinished()
  }

  return (
    <Sheet open={open} onOpenChange={setOpen}>
      <SheetContent
        title="Activity"
        actions={
          finished.length > 0 && (
            <Button size="sm" variant="ghost" onClick={clear}>
              Clear finished
            </Button>
          )
        }
      >
        {jobs.length === 0 ? (
          <EmptyState
            icon={<Activity />}
            title="Nothing running"
            description="Indexing, previews and exports show up here with live progress while they run."
            className="h-full"
          />
        ) : (
          <div className="flex flex-col">
            {active.length > 0 && <Group title="In progress" jobs={active} />}
            {finished.length > 0 && <Group title="Finished" jobs={finished} />}
          </div>
        )}
      </SheetContent>
    </Sheet>
  )
}

function Group({ title, jobs }: { title: string; jobs: JobInfo[] }) {
  return (
    <section className="pb-2">
      <h3 className="eyebrow px-4 pt-3 pb-1.5">{title}</h3>
      <ul>
        {jobs.map((j) => (
          <JobRow key={j.id} job={j} />
        ))}
      </ul>
    </section>
  )
}

function JobRow({ job: j }: { job: JobInfo }) {
  const navigate = useNavigate()
  const done = isFinished(j)
  const canOpen = j.kind === "export" && j.projectId
  const open = () => {
    if (!canOpen) return
    useUi.getState().setActivityOpen(false)
    void navigate({ to: "/p/$pid/exports", params: { pid: j.projectId! } })
  }
  return (
    <li
      onClick={open}
      tabIndex={canOpen ? 0 : undefined}
      role={canOpen ? "link" : undefined}
      onKeyDown={(e) => {
        if (canOpen && e.key === "Enter" && e.target === e.currentTarget) open()
      }}
      className={cn(
        "group flex gap-3 px-4 py-2.5 transition-colors focus-visible:bg-hover focus-visible:outline-none",
        canOpen && "cursor-pointer hover:bg-hover",
      )}
    >
      <span
        className={cn(
          "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-md border border-hairline bg-raised [&_svg]:size-3.5",
          j.state === "running" ? "text-accent" : "text-fg-3",
        )}
      >
        {KIND_ICON[j.kind]}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm text-fg-1" title={j.title}>
              {j.title}
            </div>
            {j.subtitle && <div className="truncate text-2xs text-fg-3">{j.subtitle}</div>}
          </div>
          {!done ? (
            <Tooltip label="Cancel">
              <button
                aria-label={`Cancel ${j.title}`}
                onClick={(e) => {
                  e.stopPropagation()
                  void commands.jobCancel(j.id)
                }}
                className="rounded-sm p-1 text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-hover hover:text-fg-1 focus-visible:opacity-100"
              >
                <X className="size-3.5" />
              </button>
            </Tooltip>
          ) : (
            <StateIcon job={j} />
          )}
        </div>
        {!done && (
          <div className="mt-2">
            <Progress value={j.state === "running" ? j.progress : null} />
            <div className="mt-1 flex justify-between gap-2 text-2xs text-fg-3">
              <span className="truncate">{j.state === "queued" ? "Waiting" : (j.stage ?? "Working")}</span>
              <span className="timecode shrink-0">
                {[formatEta(j.etaMs), formatPercent(j.progress)].filter(Boolean).join(" · ")}
              </span>
            </div>
          </div>
        )}
        {j.state === "failed" && j.error && <p className="mt-1 line-clamp-3 text-2xs break-words text-danger">{j.error}</p>}
      </div>
    </li>
  )
}

function StateIcon({ job }: { job: JobInfo }) {
  if (job.state === "done") return <Check className="mt-0.5 size-4 shrink-0 text-fg-3" aria-label="Done" />
  if (job.state === "failed") return <AlertTriangle className="mt-0.5 size-4 shrink-0 text-danger" aria-label="Failed" />
  return <CircleSlash className="mt-0.5 size-4 shrink-0 text-fg-3" aria-label="Cancelled" />
}
