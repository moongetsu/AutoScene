import { motion } from "motion/react"
import { Check, Users } from "lucide-react"
import { FacePortrait } from "@/components/shared/FacePortrait"
import { api, appendQuery, formatDuration, type Person, type Scene } from "@/lib/api"
import { cn } from "@/lib/utils"

type CastTileProps = {
  projectId: string
  person: Person
  scenes: Scene[]
  cacheRev: string
  index: number
  selected: boolean
  selectMode: boolean
  picked: boolean
  onClick: () => void
}

export function CastTile({
  projectId,
  person,
  scenes,
  cacheRev,
  index,
  selected,
  selectMode,
  picked,
  onClick,
}: CastTileProps) {
  const thumb = person.thumbnail_path
    ? appendQuery(api.mediaUrl(projectId, person.thumbnail_path), {
        v: `${cacheRev}-${person.detection_count}`,
      })
    : null
  const sharedCount = scenes.filter(
    (s) => s.person_ids.includes(person.person_id) && s.person_ids.length > 1,
  ).length

  return (
    <motion.button
      type="button"
      onClick={onClick}
      initial={{ opacity: 0, y: 8 }}
      animate={{ opacity: 1, y: 0 }}
      transition={{
        type: "spring",
        stiffness: 380,
        damping: 28,
        delay: Math.min(index, 16) * 0.018,
      }}
      className={cn(
        "group relative flex flex-col overflow-hidden rounded-lg border text-left transition duration-200",
        selectMode && picked
          ? "border-primary/55 bg-primary/5 ring-1 ring-primary/30"
          : selected
            ? "border-primary/45 bg-card ring-1 ring-primary/25"
            : "border-border/35 bg-card/30 hover:border-border/70 hover:bg-card/55",
      )}
    >
      {selectMode ? (
        <span
          className={cn(
            "absolute left-1.5 top-1.5 z-10 flex size-5 items-center justify-center rounded border backdrop-blur-sm",
            picked
              ? "border-primary/60 bg-primary/25 text-primary"
              : "border-border/50 bg-background/60 text-transparent",
          )}
        >
          {picked ? <Check className="size-3" /> : null}
        </span>
      ) : (
        <span className="timecode absolute left-1.5 top-1.5 z-10 rounded bg-background/70 px-1 py-0.5 text-[9px] tabular-nums text-muted-foreground backdrop-blur-sm">
          {String(index + 1).padStart(2, "0")}
        </span>
      )}

      <div className="relative">
        <FacePortrait
          src={thumb}
          alt={person.label}
          size="tile"
          fit="contain"
          className="border-0 bg-[oklch(0.09_0.012_280)]"
          imgClassName="transition duration-500 group-hover:scale-[1.04]"
        />
        <div className="pointer-events-none absolute inset-x-0 bottom-0 bg-gradient-to-t from-background/95 via-background/50 to-transparent px-2 pb-2 pt-8">
          <p className="truncate font-heading text-[13px] leading-tight tracking-tight">
            {person.label}
          </p>
          <p className="timecode mt-0.5 truncate text-[10px] tabular-nums text-muted-foreground">
            {person.clip_count} scenes · {formatDuration(person.screen_time_sec)}
          </p>
        </div>
        {sharedCount > 0 ? (
          <span className="absolute right-1.5 top-1.5 inline-flex items-center gap-0.5 rounded border border-border/40 bg-background/75 px-1 py-0.5 text-[9px] text-muted-foreground backdrop-blur-sm">
            <Users className="size-2.5" />
            {sharedCount}
          </span>
        ) : null}
      </div>
    </motion.button>
  )
}
