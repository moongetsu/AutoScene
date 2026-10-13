import { Link } from "@tanstack/react-router"
import { Compass } from "lucide-react"
import { Button } from "@/components/ui/button"
import { EmptyState } from "@/components/ui/empty-state"

/** Unknown route (a stale link, a removed project). */
export function NotFound() {
  return (
    <EmptyState
      icon={<Compass />}
      title="Nothing here"
      description="This page doesn't exist anymore. The project or episode may have been removed."
      className="h-full"
      actions={
        <Button asChild>
          <Link to="/">Back to Library</Link>
        </Button>
      }
    />
  )
}
