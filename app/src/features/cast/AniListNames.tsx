import { useState } from "react"
import { toast } from "sonner"
import type { Cast, ProjectId } from "@/bindings"
import { ArrowsMerge, Check, CircleNotch, Users } from "@/components/icons"
import { Button } from "@/components/ui/button"
import { commands, errorMessage } from "@/lib/ipc"
import { assetUrl } from "@/lib/media"
import { qk, queryClient, useAniListNames, useProject, useSettings } from "@/lib/queries"
import { cn } from "@/lib/utils"
import { AniListDialog } from "./AniListDialog"
import { plural } from "./cast"

/**
 * Names from AniList for an anime project's cast. Once the characters are
 * found the app looks them up by itself (the series by the project's
 * title, then its cast's portraits matched to the characters); this shows
 * that it is looking, then the names it found, given with one click. Shown
 * while the AniList setting is on.
 */
export function AniListNames({ pid, cast, className }: { pid: ProjectId; cast: Cast | undefined; className?: string }) {
  const { data: settings } = useSettings()
  const { data: project } = useProject(pid)
  const on = !!settings?.anilist && project?.content === "anime"
  const { data } = useAniListNames(pid, on && !!cast)
  const [busy, setBusy] = useState(false)
  const [searching, setSearching] = useState(false)
  // Suggestions the person took out (a character id; for a merge, its
  // `from`), for the set of suggestions they were taken out of.
  const [left, setLeft] = useState<{ of: string; ids: ReadonlySet<string> }>({ of: "", ids: new Set() })
  if (!on || !data || !cast) return null

  const { names, merges } = data
  const shown = names.length > 0 || merges.length > 0
  const set = [data.seriesId, ...names.map((n) => `${n.characterId}:${n.anilistId}`), ...merges.map((m) => m.from)].join()
  const out = left.of === set ? left.ids : new Set<string>()
  const kept = names.filter((n) => !out.has(n.characterId))
  const keptMerges = merges.filter((m) => !out.has(m.from))
  const toggle = (id: string) => {
    const ids = new Set(out)
    if (!ids.delete(id)) ids.add(id)
    setLeft({ of: set, ids })
  }
  const character = (id: string) => cast.characters.find((c) => c.id === id)
  const refresh = () => void queryClient.invalidateQueries({ queryKey: qk.cast(pid) })
  const act = (failed: string, work: () => Promise<void>) => {
    setBusy(true)
    void work()
      .catch((e) => {
        toast.error(failed, { description: errorMessage(e) })
        refresh()
      })
      .finally(() => setBusy(false))
  }
  const give = () =>
    act("The names weren't given", async () => {
      // What was shown and kept, and what was left out: a look-up that ran
      // since gives nothing unseen.
      const leftNames = names.filter((n) => out.has(n.characterId))
      const leftMerges = merges.filter((m) => out.has(m.from))
      await commands.anilistAccept(pid, kept, keptMerges, leftNames, leftMerges)
      toast.success(kept.length > 0 ? `Named ${plural(kept.length, "character")} from AniList` : "Characters merged")
      refresh()
    })
  const dismiss = () =>
    act("The names are still waiting", async () => {
      await commands.anilistDismiss(pid)
      refresh()
    })
  // Not this series: it is turned down (its names go with it), and the
  // search by hand opens for the right one.
  const wrongSeries = () =>
    act("The series wasn't turned down", async () => {
      if (data.seriesId != null) await commands.anilistReject(pid, data.seriesId)
      refresh()
      setSearching(true)
    })

  return (
    <>
      {shown ? (
        <div role="status" className={cn("rounded-2xl border border-accent-line/50 bg-accent-subtle px-5 py-4", className)}>
          <div className="flex flex-wrap items-center gap-x-4 gap-y-3">
            <Users weight="fill" className="size-5 shrink-0 text-accent" />
            <div className="min-w-0 flex-1">
              <p className="text-md font-medium text-fg-1">
                {names.length > 0
                  ? `AniList has names for ${plural(names.length, "character")}`
                  : "AniList sees one person in two characters"}
              </p>
              <p className="text-sm text-fg-2">
                From the cast of {data.series ?? "the series"}. Click one that's wrong to leave it out.
              </p>
            </div>
            <div className="flex items-center gap-2">
              <Button variant="ghost" disabled={busy} onClick={wrongSeries}>
                Wrong series…
              </Button>
              <Button variant="ghost" disabled={busy} onClick={dismiss}>
                Not now
              </Button>
              <Button
                variant="primary"
                disabled={busy || (kept.length === 0 && keptMerges.length === 0)}
                onClick={give}
              >
                {busy ? <CircleNotch className="animate-spin" /> : <Check weight="bold" />}{" "}
                {names.length > 0 ? (kept.length === names.length ? "Give names" : `Give ${kept.length}`) : "Merge"}
              </Button>
            </div>
          </div>
          <ul className="mt-4 flex flex-wrap gap-2">
            {names.map((n) => {
              const c = character(n.characterId)
              const leftOut = out.has(n.characterId)
              return (
                <li key={n.characterId}>
                  <button
                    type="button"
                    aria-pressed={!leftOut}
                    onClick={() => toggle(n.characterId)}
                    className={cn(
                      "flex items-center gap-2.5 rounded-full bg-white/[0.06] py-1 pr-3.5 pl-1 transition-[opacity,background-color] hover:bg-white/[0.1]",
                      leftOut && "opacity-45",
                    )}
                  >
                    <span className="size-7 shrink-0 overflow-hidden rounded-full bg-raised">
                      {c?.coverSmall && <img src={assetUrl(c.coverSmall)} alt="" className="size-full object-cover" />}
                    </span>
                    <span className={cn("text-sm text-fg-1", leftOut && "line-through")}>{n.name}</span>
                    <span className="text-sm text-fg-3">{c?.label ?? "a character"}</span>
                  </button>
                </li>
              )
            })}
            {merges.map((m) => {
              const leftOut = out.has(m.from)
              return (
                <li key={m.from}>
                  <button
                    type="button"
                    aria-pressed={!leftOut}
                    onClick={() => toggle(m.from)}
                    className={cn(
                      "flex items-center gap-2 rounded-full bg-white/[0.06] px-3.5 py-1.5 text-sm text-fg-2 transition-[opacity,background-color] hover:bg-white/[0.1]",
                      leftOut && "line-through opacity-45",
                    )}
                  >
                    <ArrowsMerge className="size-4 text-fg-3" /> {character(m.from)?.label ?? "A character"} is{" "}
                    {character(m.into)?.label ?? "another"}
                  </button>
                </li>
              )
            })}
          </ul>
        </div>
      ) : (
        data.looking && (
          <p role="status" className={cn("flex items-center gap-2.5 text-sm text-fg-2", className)}>
            <CircleNotch className="size-4 animate-spin" /> Looking up the characters' names on AniList…
          </p>
        )
      )}
      {/* The search by hand outlives the banner: a series confirmed there
          makes the names here go. */}
      <AniListDialog pid={pid} characters={cast.characters} open={searching} onOpenChange={setSearching} />
    </>
  )
}
