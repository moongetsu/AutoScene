import { useEffect, useState } from "react"
import { toast } from "sonner"
import type { AniListMedia, AniListMerge, AniListSuggestion, Character, ProjectId } from "@/bindings"
import { Button } from "@/components/ui/button"
import { Dialog, DialogContent, DialogFooter } from "@/components/ui/dialog"
import { Input } from "@/components/ui/input"
import { CircleNotch } from "@/components/icons"
import { commands, errorMessage } from "@/lib/ipc"
import { qk, queryClient } from "@/lib/queries"

type Stage = "search" | "chain" | "suggestions"

/** Fetch names from AniList: the person confirms the series, then the
 *  series' cast portraits are matched to the project's characters and the
 *  names are suggested (or given at once when the project's auto flag is
 *  set). Only ever opened from the Cast page of an anime project while the
 *  AniList setting is on. */
export function AniListDialog({
  pid,
  characters,
  open,
  onOpenChange,
}: {
  pid: ProjectId
  characters: Character[]
  open: boolean
  onOpenChange: (open: boolean) => void
}) {
  const [stage, setStage] = useState<Stage>("search")
  const [text, setText] = useState("")
  const [busy, setBusy] = useState(false)
  const [results, setResults] = useState<AniListMedia[]>([])
  const [chain, setChain] = useState<AniListMedia[]>([])
  const [rev, setRev] = useState(0)
  const [names, setNames] = useState<AniListSuggestion[]>([])
  const [merges, setMerges] = useState<AniListMerge[]>([])
  const [auto, setAuto] = useState(false)
  const nameOf = (id: string) => characters.find((c) => c.id === id)?.label ?? "a character"

  // The suggested search text is filled once the dialog opens; nothing is
  // sent before the Search click.
  useEffect(() => {
    if (open && text === "") {
      commands.anilistSearchText(pid).then(setText).catch(() => {})
    }
  }, [open, pid, text])

  const run = (work: () => Promise<void>) => {
    setBusy(true)
    void work()
      .catch((e) => toast.error("AniList couldn't answer", { description: errorMessage(e) }))
      .finally(() => setBusy(false))
  }

  const search = () =>
    run(async () => {
      setResults(await commands.anilistSearch(pid, text))
    })

  const confirm = (m: AniListMedia) =>
    run(async () => {
      setChain(await commands.anilistConfirm(pid, m.id))
      setStage("chain")
    })

  const suggest = () =>
    run(async () => {
      const s = await commands.anilistSuggest(pid)
      setRev(s.rev)
      setNames(s.names)
      setMerges(s.merges)
      setAuto(s.auto)
      setStage("suggestions")
      if (s.auto) {
        toast.success(`Named ${s.names.length} characters from AniList`)
        void queryClient.invalidateQueries({ queryKey: qk.cast(pid) })
      }
    })

  const apply = () =>
    run(async () => {
      await commands.anilistApply(pid, rev, names, merges)
      toast.success("Names applied")
      void queryClient.invalidateQueries({ queryKey: qk.cast(pid) })
      onOpenChange(false)
    })

  const title = { search: "Names from AniList", chain: "This series?", suggestions: "Name suggestions" }[stage]

  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent title={title} description="AniList sees only what you send: the search text, then the cast and images of the series you confirm.">
        {stage === "search" && (
          <>
            <div className="flex gap-2">
              <Input value={text} onChange={(e) => setText(e.target.value)} placeholder="Series to search for" aria-label="Search text" />
              <Button disabled={busy || !text.trim()} onClick={search}>
                {busy ? <CircleNotch className="animate-spin" /> : null} Search
              </Button>
            </div>
            <ul className="mt-4 max-h-72 space-y-1 overflow-y-auto">
              {results.map((m) => (
                <li key={m.id}>
                  <button
                    type="button"
                    className="w-full rounded-lg px-3 py-2 text-left transition-colors hover:bg-white/[0.06]"
                    disabled={busy}
                    onClick={() => confirm(m)}
                  >
                    <span className="text-md text-fg-1">{m.title}</span>
                    <span className="ml-2 text-sm text-fg-3">
                      {[m.format, m.year, m.episodes != null ? `${m.episodes} ep` : null].filter(Boolean).join(" · ")}
                    </span>
                  </button>
                </li>
              ))}
            </ul>
          </>
        )}
        {stage === "chain" && (
          <>
            <p className="text-md text-fg-2">
              The whole chain is kept ({chain.length} {chain.length === 1 ? "series" : "series"} — sequels share their
              cast):
            </p>
            <ul className="mt-3 space-y-1">
              {chain.map((m) => (
                <li key={m.id} className="rounded-lg bg-white/[0.04] px-3 py-2 text-md text-fg-1">
                  {m.title}
                </li>
              ))}
            </ul>
            <DialogFooter className="mt-5">
              <Button variant="ghost" disabled={busy} onClick={() => setStage("search")}>
                Back
              </Button>
              <Button disabled={busy} onClick={suggest}>
                {busy ? <CircleNotch className="animate-spin" /> : null} Match the cast
              </Button>
            </DialogFooter>
          </>
        )}
        {stage === "suggestions" && (
          <>
            {names.length === 0 && merges.length === 0 ? (
              <p className="text-md text-fg-2">No portrait matched a character closely enough.</p>
            ) : (
              <ul className="max-h-72 space-y-1 overflow-y-auto">
                {names.map((n) => (
                  <li key={n.characterId} className="flex items-center justify-between rounded-lg bg-white/[0.04] px-3 py-2">
                    <span className="text-md text-fg-1">{n.name}</span>
                    <span className="text-sm text-fg-3">
                      {nameOf(n.characterId)} · {Math.round((n.score ?? 0) * 100)}%
                    </span>
                  </li>
                ))}
                {merges.map((m, i) => (
                  <li key={i} className="rounded-lg bg-white/[0.04] px-3 py-2 text-sm text-fg-2">
                    Merge {nameOf(m.from)} into {nameOf(m.into)}
                  </li>
                ))}
              </ul>
            )}
            <DialogFooter className="mt-5">
              <Button variant="ghost" onClick={() => onOpenChange(false)}>
                {auto ? "Close" : "Not now"}
              </Button>
              {!auto && (names.length > 0 || merges.length > 0) && (
                <Button disabled={busy} onClick={apply}>
                  {busy ? <CircleNotch className="animate-spin" /> : null} Apply
                </Button>
              )}
            </DialogFooter>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
