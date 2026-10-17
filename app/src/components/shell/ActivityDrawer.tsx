import type * as React from "react"
import { useNavigate } from "@tanstack/react-router"
import {
  Check,
  Copy,
  Eye,
  FilmStrip,
  Image,
  Package,
  Play,
  Prohibit,
  Queue,
  Scan,
  Scissors,
  SquaresFour,
  Trash,
  UserFocus,
  Users,
  Warning,
  X,
} from "@/components/icons"
import type { JobInfo } from "@/bindings"
import { Button } from "@/components/ui/button"
import { ContextArea } from "@/components/ui/context-area"
import { ContextMenuItem } from "@/components/ui/context-menu"
import { EmptyState } from "@/components/ui/empty-state"
import { Progress } from "@/components/ui/progress"
import { Sheet, SheetContent } from "@/components/ui/sheet"
import { Tooltip, TruncatedText } from "@/components/ui/tooltip"
import { formatEta, formatPercent } from "@/lib/format"
import { commands } from "@/lib/ipc"
import { copyText } from "@/lib/os"
import { cn } from "@/lib/utils"
import { isFinished, useActivity, useJobs } from "@/state/activity"
import { useUi } from "@/state/ui"

const KIND_ICON: Record<JobInfo["kind"], React.ReactNode> = {
  ingest: <FilmStrip />,
  proxy: <Eye />,
  export: <Scissors />,
  analyze: <Scan />,
  posters: <Image />,
  portraits: <UserFocus />,
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
            icon={<Queue />}
            title="Nothing running"
            description="Indexing, previews and exports show up here with live progress while they run."
            className="h-full"
          />
        ) : (
          <ContextArea
            menu={
              finished.length > 0 && (
                <ContextMenuItem onSelect={clear}>
                  <Trash /> Clear finished
                </ContextMenuItem>
              )
            }
          >
            <div className="flex flex-col">
              {active.length > 0 && <Group title="In progress" jobs={active} />}
              {finished.length > 0 && <Group title="Finished" jobs={finished} />}
            </div>
          </ContextArea>
        )}
      </SheetContent>
    </Sheet>
  )
}

function Group({ title, jobs }: { title: string; jobs: JobInfo[] }) {
  return (
    <section className="px-3 pb-3">
      <h3 className="eyebrow px-2 pt-4 pb-2">{title}</h3>
      <ul className="flex flex-col gap-2">
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
  const go = (to: () => Promise<void>) => {
    useUi.getState().setActivityOpen(false)
    void to()
  }
  const pid = j.projectId
  const mid = j.mediaId
  const menu = (
    <>
      {!done && (
        <ContextMenuItem onSelect={() => void commands.jobCancel(j.id)}>
          <X /> Cancel
        </ContextMenuItem>
      )}
      {canOpen && (
        <ContextMenuItem onSelect={open}>
          <Package /> Show the exports
        </ContextMenuItem>
      )}
      {pid && mid && j.kind !== "export" && (
        <ContextMenuItem onSelect={() => go(() => navigate({ to: "/p/$pid/timeline/$mid", params: { pid, mid } }))}>
          <Play /> Open in the player
        </ContextMenuItem>
      )}
      {pid && j.kind === "portraits" && (
        <ContextMenuItem onSelect={() => go(() => navigate({ to: "/p/$pid/cast", params: { pid } }))}>
          <Users /> Open the cast
        </ContextMenuItem>
      )}
      {pid && !mid && j.kind === "posters" && (
        <ContextMenuItem onSelect={() => go(() => navigate({ to: "/p/$pid", params: { pid } }))}>
          <SquaresFour /> Open the project
        </ContextMenuItem>
      )}
      {j.state === "failed" && j.error && (
        <ContextMenuItem onSelect={() => void copyText(j.error ?? "", "error")}>
          <Copy /> Copy the error
        </ContextMenuItem>
      )}
    </>
  )
  return (
    <ContextArea menu={menu}>
      <li
        onClick={open}
        tabIndex={canOpen ? 0 : undefined}
        role={canOpen ? "link" : undefined}
        onKeyDown={(e) => {
          if (canOpen && e.key === "Enter" && e.target === e.currentTarget) open()
        }}
        className={cn(
          "group flex gap-3 rounded-2xl border border-white/[0.06] bg-white/[0.03] p-3.5 transition-colors focus-visible:bg-white/[0.06] focus-visible:outline-none",
          canOpen && "hover:bg-white/[0.06]",
        )}
      >
        <span
          className={cn(
            "flex size-9 shrink-0 items-center justify-center rounded-xl [&_svg]:size-[18px]",
            j.state === "running" ? "bg-accent-subtle text-accent" : "bg-white/[0.06] text-fg-3",
          )}
        >
          {KIND_ICON[j.kind]}
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-start gap-2">
            <div className="min-w-0 flex-1">
              <TruncatedText className="text-md font-medium text-fg-1">{j.title}</TruncatedText>
              {j.subtitle && <div className="truncate text-sm text-fg-3">{j.subtitle}</div>}
            </div>
            {!done ? (
              <Tooltip label="Cancel">
                <button
                  aria-label={`Cancel ${j.title}`}
                  onClick={(e) => {
                    e.stopPropagation()
                    void commands.jobCancel(j.id)
                  }}
                  className="flex size-7 items-center justify-center rounded-full text-fg-3 opacity-0 transition-opacity group-hover:opacity-100 hover:bg-white/[0.1] hover:text-fg-1 focus-visible:opacity-100"
                >
                  <X className="size-3.5" />
                </button>
              </Tooltip>
            ) : (
              <StateIcon job={j} />
            )}
          </div>
          {!done && (
            <div className="mt-3">
              <Progress value={j.state === "running" ? j.progress : null} />
              <div className="mt-1.5 flex justify-between gap-2 text-xs text-fg-3">
                <span className="truncate">{j.state === "queued" ? "Waiting" : (j.stage ?? "Working")}</span>
                <span className="timecode shrink-0">
                  {[formatEta(j.etaMs), formatPercent(j.progress)].filter(Boolean).join(" · ")}
                </span>
              </div>
            </div>
          )}
          {j.state === "failed" && j.error && <p className="mt-1.5 line-clamp-3 text-sm break-words text-danger">{j.error}</p>}
        </div>
      </li>
    </ContextArea>
  )
}

function StateIcon({ job }: { job: JobInfo }) {
  if (job.state === "done") return <Check className="mt-0.5 size-4 shrink-0 text-fg-3" aria-label="Done" />
  if (job.state === "failed") return <Warning className="mt-0.5 size-4 shrink-0 text-danger" aria-label="Failed" />
  return <Prohibit className="mt-0.5 size-4 shrink-0 text-fg-3" aria-label="Cancelled" />
}
