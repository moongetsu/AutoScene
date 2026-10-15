import { Link } from "@tanstack/react-router"
import { Compass } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"
import { ErrorState } from "@/components/ui/error-state"
import { isAppError } from "@/lib/ipc"

/**
 * A project view whose data couldn't be loaded. A project that is gone
 * (removed, or its folder moved) says so and leads back to the library;
 * anything else can be tried again.
 */
export function ProjectLoadError({ error, onRetry }: { error: unknown; onRetry: () => void }) {
  if (isAppError(error) && error.code === "not_found") {
    return (
      <EmptyState
        icon={<Compass />}
        title="This project isn't here any more"
        description={error.message}
        className="h-full"
        actions={
          <Button asChild>
            <Link to="/">Back to Library</Link>
          </Button>
        }
      />
    )
  }
  return <ErrorState error={error} className="h-full" onRetry={onRetry} />
}
