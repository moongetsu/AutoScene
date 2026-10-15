import type { Segment } from "@/bindings"

export const SEGMENT_LABEL: Record<Segment["kind"], string> = {
  shot: "Shot",
  dissolve: "Dissolve",
  fade_out: "Fade out",
  fade_in: "Fade in",
  black: "Black",
  flash: "Flash",
}
