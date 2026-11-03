import { DotsThree, FolderOpen, Play, Trash } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { Tooltip } from "@/components/ui/tooltip"
import {
  DropdownMenu,
  DropdownMenuTrigger,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
} from "@/components/ui/dropdown-menu"
import { toast } from "sonner"

export function OverlaysShowcase() {
  return (
    <div className="flex flex-col gap-10">
      <div>
        <h2 className="text-3xl font-bold text-fg-1">Overlays & Menus</h2>
        <p className="text-fg-2 mt-1">
          Floating tooltips, glass dropdown menus, and toast notifications.
        </p>
      </div>

      {/* Tooltips */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Tooltips</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-6 items-center">
          <Tooltip label="Plays the source video at full resolution">
            <Button variant="secondary">Hover for Tooltip</Button>
          </Tooltip>

          <Tooltip label="Accent-styled highlight information">
            <Button variant="accent">Accent Button Tooltip</Button>
          </Tooltip>
        </div>
      </section>

      {/* Dropdown Menu */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Dropdown Menus</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-6 items-center">
          <DropdownMenu>
            <DropdownMenuTrigger asChild>
              <Button variant="secondary">
                Actions Menu <DotsThree weight="bold" />
              </Button>
            </DropdownMenuTrigger>
            <DropdownMenuContent>
              <DropdownMenuItem onSelect={() => toast("Opening source…")}>
                <Play /> Watch source
              </DropdownMenuItem>
              <DropdownMenuItem onSelect={() => toast("Revealing in folder…")}>
                <FolderOpen /> Reveal in Folder
              </DropdownMenuItem>
              <DropdownMenuSeparator />
              <DropdownMenuItem onSelect={() => toast.error("Clip removed")} className="text-danger">
                <Trash /> Delete Export
              </DropdownMenuItem>
            </DropdownMenuContent>
          </DropdownMenu>
        </div>
      </section>

      {/* Toasts */}
      <section className="flex flex-col gap-4">
        <h3 className="text-lg font-semibold text-fg-1">Toasts (Sonner)</h3>
        <div className="rounded-2xl border border-white/[0.08] bg-canvas p-6 flex flex-wrap gap-4 items-center">
          <Button variant="secondary" onClick={() => toast("Project saved", { description: "AutoScene project saved locally." })}>
            Trigger Standard Toast
          </Button>
          <Button variant="accent" onClick={() => toast.success("Export finished", { description: "Filtered 42 vocal scenes." })}>
            Trigger Success Toast
          </Button>
          <Button variant="danger" onClick={() => toast.error("Export failed", { description: "Frame dropped during verification." })}>
            Trigger Error Toast
          </Button>
        </div>
      </section>
    </div>
  )
}
