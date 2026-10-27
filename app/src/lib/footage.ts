import type { ContentKind } from "@/bindings"

/**
 * What a project's footage can be, with example stills for the picker's
 * hover preview (`public/footage`: cropped and scaled stills from open
 * films and artwork under Creative Commons Attribution licences — credited
 * under them and in full, with the licences, in Settings → About).
 */
export type Footage = {
  value: ContentKind
  label: string
  /** One line: what it is and what the app does with it. */
  blurb: string
  /** A wide still, then two 16:9 close-ups. */
  stills: [string, string, string]
  credit: string
}

export const FOOTAGE: Footage[] = [
  {
    value: "live",
    label: "Live action",
    blurb: "Filmed with real actors. Characters are found by their faces.",
    stills: ["/footage/live-1.webp", "/footage/live-2.webp", "/footage/live-3.webp"],
    credit: "Tears of Steel · Blender Foundation · CC BY 3.0",
  },
  {
    value: "animation",
    label: "Animation",
    blurb:
      "Computer-animated (3D) films and series. Characters are found by their faces, tuned for animated characters drawn in one style.",
    stills: ["/footage/animation-1.webp", "/footage/animation-2.webp", "/footage/animation-3.webp"],
    credit: "Sprite Fright, Spring · Blender Studio · CC BY 4.0",
  },
  {
    value: "anime",
    label: "Anime",
    blurb: "Anime and other hand-drawn 2D animation. Shots are found now; characters come in a later update.",
    stills: ["/footage/anime-1.webp", "/footage/anime-2.webp", "/footage/anime-3.webp"],
    credit: "Pepper&Carrot artwork · David Revoy · Hero · Blender Studio · CC BY 4.0",
  },
  {
    value: "mixed",
    label: "Mixed",
    blurb: "Live action and animation in one project. Characters are found by their faces, whichever they are.",
    stills: ["/footage/mixed-1.webp", "/footage/mixed-2.webp", "/footage/mixed-3.webp"],
    credit: "Tears of Steel · CC BY 3.0 · Sprite Fright · CC BY 4.0 · Blender Foundation",
  },
]

export const CONTENT_LABEL = Object.fromEntries(FOOTAGE.map((f) => [f.value, f.label])) as Record<ContentKind, string>

/** The footage's line: what it is and what the app does with it. */
export const footageBlurb = (v: ContentKind) => FOOTAGE.find((f) => f.value === v)?.blurb
