import { useNavigate } from "@tanstack/react-router"
import { ArrowsMerge, Eye, EyeSlash, Image, Package, PencilSimple, User, UserFocus } from "@/components/icons"
import type { Character } from "@/bindings"
import type { MenuKit } from "@/components/ui/menu-actions"
import { castEdit } from "./cast"

/**
 * What a character offers wherever it is shown — its card, its page's hero,
 * the ⋯ menu there: open it, its scenepack, the faces to review, rename,
 * choose its picture, merge, hide. `onRename`, `onChoosePicture` and
 * `onMerge` only where those can be done; `open` off on the character's
 * page itself.
 */
export function CharacterItems({
  kit,
  pid,
  character: c,
  open = true,
  onRename,
  onChoosePicture,
  onMerge,
}: {
  kit: MenuKit
  pid: string
  character: Character
  open?: boolean
  onRename?: () => void
  onChoosePicture?: () => void
  onMerge?: () => void
}) {
  const navigate = useNavigate()
  const { Item, Separator } = kit
  return (
    <>
      {open && (
        <Item onSelect={() => void navigate({ to: "/p/$pid/cast/$cid", params: { pid, cid: c.id } })}>
          <User /> Open character
        </Item>
      )}
      <Item
        disabled={c.shots === 0}
        onSelect={() => void navigate({ to: "/p/$pid/cast/$cid/pack", params: { pid, cid: c.id } })}
      >
        <Package /> Build a scenepack
      </Item>
      {c.unsure > 0 && (
        <Item onSelect={() => void navigate({ to: "/p/$pid/cast/review", params: { pid }, search: { c: c.id } })}>
          <UserFocus /> Review {c.unsure} {c.unsure === 1 ? "face" : "faces"}
        </Item>
      )}
      <Separator />
      {onRename && (
        <Item onSelect={onRename}>
          <PencilSimple /> Rename
        </Item>
      )}
      {onChoosePicture && (
        <Item disabled={c.tracks === 0} onSelect={onChoosePicture}>
          <Image /> Choose picture…
        </Item>
      )}
      {onMerge && (
        <Item onSelect={onMerge}>
          <ArrowsMerge /> Same person as…
        </Item>
      )}
      <Item onSelect={() => void castEdit(pid, { op: "set_hidden", characterId: c.id, hidden: !c.hidden })}>
        {c.hidden ? <Eye /> : <EyeSlash />} {c.hidden ? "Show in the cast" : "Hide from the cast"}
      </Item>
    </>
  )
}
