import { useQueryErrorResetBoundary } from "@tanstack/react-query"
import type { ErrorComponentProps } from "@tanstack/react-router"
import { ErrorState } from "@/components/ui/error-state"
import { Page } from "@/components/ui/page"
import { TopBar } from "@/components/ui/topbar"

/** A view that failed while showing itself: the rest of the window keeps working. */
export function RouteError({ error, reset }: ErrorComponentProps) {
  const queries = useQueryErrorResetBoundary()
  return (
    <Page topbar={<TopBar />}>
      <ErrorState
        error={error}
        title="Something went wrong here"
        className="h-full"
        onRetry={() => {
          queries.reset()
          reset()
        }}
      />
    </Page>
  )
}
