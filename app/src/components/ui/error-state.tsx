import { ArrowCounterClockwise, FileText, Warning } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { commands, errorMessage, isAppError } from "@/lib/ipc"

/**
 * A view whose data couldn't be loaded: what went wrong in plain words, and a
 * way to try again. Failures of the app itself (its data, its files) also
 * offer the logs, where their technical side is written.
 */
export function ErrorState({
  error,
  title = "This couldn't be loaded",
  onRetry,
  className,
}: {
  error: unknown
  title?: string
  onRetry?: () => void
  className?: string
}) {
  const ownFault = !isAppError(error) || error.code === "internal" || error.code === "io" || error.code === "media"
  return (
    <EmptyState
      tone="danger"
      icon={<Warning />}
      title={title}
      description={isAppError(error) ? error.message : "Something went wrong while showing this."}
      className={className}
      actions={
        <>
          {onRetry && (
            <Button onClick={onRetry}>
              <ArrowCounterClockwise /> Try again
            </Button>
          )}
          {ownFault && (
            <Button variant="ghost" onClick={() => void commands.openLogsDir().catch(() => {})}>
              <FileText /> Open logs
            </Button>
          )}
        </>
      }
    >
      {/* A bug in the view itself: its own words, small and copyable, for a report. */}
      {!isAppError(error) && (
        <p data-selectable className="timecode mt-6 max-w-lg text-xs break-words text-fg-3">
          {errorMessage(error)}
        </p>
      )}
    </EmptyState>
  )
}
