import { FilePlus, FilmStrip, FolderOpen, FolderPlus, Package, PencilSimple, Play, Plus, Trash, Users } from "@/components/icons"
import type { ProjectSummary } from "@/bindings"
import type { MenuKit } from "@/components/ui/menu-actions"
import { useAppActions } from "@/lib/actions"
import { pickFolder, pickVideoFiles } from "@/lib/media"
import { reveal } from "@/lib/os"
import { shortcutKeys } from "@/lib/shortcuts"
import { useNewProject } from "@/state/new-project"

/**
 * What a project offers wherever it is shown — its card, its rail tile, its
 * ⋯ menu: open it or one of its places, its folder, rename, remove.
 */
export function ProjectItems({
  kit,
  project: p,
  onRename,
  onDelete,
}: {
  kit: MenuKit
  project: ProjectSummary
  onRename: () => void
  onDelete: () => void
}) {
  const actions = useAppActions()
  const { Item, Separator } = kit
  return (
    <>
      <Item onSelect={() => actions.goProject(p.id)}>
        <Play /> Open project
      </Item>
      <Item onSelect={() => actions.goProject(p.id, "episodes")}>
        <FilmStrip /> {p.kind === "movie" ? "Files" : "Episodes"}
      </Item>
      <Item onSelect={() => actions.goProject(p.id, "cast")}>
        <Users /> Cast
      </Item>
      <Item onSelect={() => actions.goProject(p.id, "exports")}>
        <Package /> Exports
      </Item>
      <Separator />
      <Item onSelect={() => reveal(p.dir)}>
        <FolderOpen /> Show project folder
      </Item>
      <Item onSelect={onRename}>
        <PencilSimple /> Edit details…
      </Item>
      <Separator />
      <Item destructive onSelect={onDelete}>
        <Trash /> Remove…
      </Item>
    </>
  )
}

/**
 * A new project: from nothing, from a season folder, from video files.
 * `page`: on the library's empty space, where "New project…" is already in
 * the app's own entries below.
 */
export function NewProjectItems({ kit, page = false }: { kit: MenuKit; page?: boolean }) {
  const { Item } = kit
  const show = (paths: string[]) => useNewProject.getState().show(paths)
  return (
    <>
      {!page && (
        <Item onSelect={() => show([])} shortcut={shortcutKeys("newProject")}>
          <Plus /> New project…
        </Item>
      )}
      <Item onSelect={() => void pickFolder().then((p) => p && show([p]))}>
        <FolderPlus /> {page ? "New project from a folder…" : "From a folder…"}
      </Item>
      <Item onSelect={() => void pickVideoFiles().then((files) => files.length > 0 && show(files))}>
        <FilePlus /> {page ? "New project from files…" : "From files…"}
      </Item>
    </>
  )
}
