import { useRef, useState } from "react"
import type { Character } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { castEdit } from "./cast"

export function RenameCharacterDialog({
  pid,
  character,
  open,
  onOpenChange,
}: {
  pid: string
  character: Character | null
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  if (!open || !character) return null

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <RenameForm pid={pid} character={character} onDone={() => onOpenChange(false)} />
    </Dialog>
  )
}

function RenameForm({
  pid,
  character,
  onDone,
}: {
  pid: string
  character: Character
  onDone: () => void
}) {
  const [name, setName] = useState(character.name ?? "")
  const [busy, setBusy] = useState(false)
  const inputRef = useRef<HTMLInputElement>(null)

  const save = async () => {
    const trimmed = name.trim()
    if (trimmed === (character.name ?? "")) {
      onDone()
      return
    }
    setBusy(true)
    try {
      const res = await castEdit(pid, {
        op: "rename",
        characterId: character.id,
        name: trimmed || null,
      })
      if (res !== undefined) {
        onDone()
      }
    } finally {
      setBusy(false)
    }
  }

  return (
    <DialogContent
      title={`Rename ${character.label}`}
      description="Give this character a name, or clear it to keep their numbered label."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault()
          void save()
        }}
        className="space-y-4"
      >
        <Input
          ref={inputRef}
          autoFocus
          value={name}
          placeholder={character.label}
          onChange={(e) => setName(e.target.value)}
          aria-label="Character name"
          disabled={busy}
        />
        <DialogFooter>
          <Button type="button" variant="ghost" onClick={onDone} disabled={busy}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" disabled={busy}>
            Save
          </Button>
        </DialogFooter>
      </form>
    </DialogContent>
  )
}
