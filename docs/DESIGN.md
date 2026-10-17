# AutoScene design system — Cinematic

AutoScene turns series and movies into scenepacks of the characters an editor chooses. The UI should feel like a
premium film app — Apple TV, Infuse, a colour suite in a dark room — not like a web dashboard: the pictures (episode
stills, faces, shots) carry the screen, the chrome stays quiet and dark around them.

Tokens live in `app/src/styles/tokens.css` and are mapped into Tailwind in `app/src/styles/index.css`. Never use raw
hex values or Tailwind palette colours (`amber-500`, `zinc-800`…) in components; white at an opacity
(`bg-white/[0.06]`) is how surfaces step up on the dark and over pictures.

## Brand

- **Mark:** the gold "A" — `app/src/components/brand/AutoSceneMark.tsx` (also `app/public/autoscene-mark.svg`,
  `app/src-tauri/icons/*`). Geometry and colours are fixed: tile `#1C1A17`, chevron and dot `#C4A35A`.
- The mark sits at the top of the rail, in About and in the empty library. A soft gold drop shadow may light it;
  never outlines or rings.

## Colour

| Role | Token | Value |
|---|---|---|
| The theatre (window canvas) | `--bg-canvas` | `#09090B` |
| Wells, cards, raised controls, overlays | `--bg-inset` / `--bg-panel` / `--bg-raised` / `--bg-overlay` | `#050506` / `#111114` / `#18181C` / `#202025` |
| The rail | `--bg-rail` | solid `#0C0C0E`; translucent over the window material |
| Text | `--fg-1/2/3` | warm white `#F5F3EF` / `#B4B1AC` / `#8A8783` (≥ 4.5:1 on panels) |
| **The main action** | `--cta`, `--fg-on-cta` | warm white pill, near-black text |
| **Accent** | `--accent` | `#E0B45C` — the logo gold, brightened for dark screens |
| Errors, destructive | `--danger` | `#EF5F57` |
| Verified, done | `--ok` | `#5FC58F` (export checks only) |
| Over pictures | `--scrim-media`, `--fg-on-media`, `Badge tone="media"` | black scrims, white text, glass chips |

Rules:

- One **white pill** per screen is the main action (Build a scenepack, Export, Review): `Button variant="primary"`.
- **Gold** marks what is selected, running or in focus: the open project in the rail, progress (with a soft glow),
  in/out points, the focused character lane, toggles, focus rings. `variant="accent"` only for a confirmation
  that belongs to a selection ("All are Walter").
- Pictures bring the colour. A hero or the player takes its glow from the picture itself (`useAmbient`, the
  player's ambilight) — never a fixed decorative colour.

## Type

- **Inter Variable** (optical sizes) for the UI, base 14px. Scale: 11 / 12 / 13 / 14 / 16 / 20 / 26 / 34 / 48 / 64 /
  84. Section titles 20px semibold; labels in sentence case (`eyebrow`, 12px semibold) — no all-caps.
- **Instrument Serif** (`display` utility) for the big titles only: series and movie titles, character names,
  page titles (Cast, Episodes…), empty states. Unnamed characters are set in italic.
- **Geist Mono Variable** (`timecode` utility) for timecodes, frame numbers, paths, sizes.

## Shape, depth, light

- Pills for buttons, chips and switches; cards 14–20px radius, portraits and heroes 20–32px.
- Pictures sit like prints: `picture-edge` (a fine light edge and a deep shadow); on hover a card rises 4px and
  its picture slowly zooms (`shadow-lift`).
- **Glass** (`glass`, `glass-strong`) for everything that floats: menus, tooltips, dialogs, the palette, the
  activity drawer, the top bar once a page scrolls, buttons over pictures (`variant="glass"`).
- **Heroes** (`components/ui/hero.tsx`): the picture bleeds to the window's edges and fades into the page; text on
  its dark side. Projects use a still, characters their face blurred into a backdrop.

## Icons

- Phosphor, regular weight; `fill` marks the selected place. Import icons only from `app/src/components/icons.ts`
  — oxlint rejects `lucide-react` and direct Phosphor imports elsewhere.
- An icon says what the control does. No decorative "AI" clichés: sparkles, lightning, magic wands, robots, stars
  (banned in `icons.ts` by lint).

## Motion

- 120ms hovers, 200ms layers, 320ms drawers and page arrivals (`animate-rise-in`). Ease `cubic-bezier(.2,.8,.2,1)`;
  switches spring.
- Animate opacity and transform only; pictures fade in when loaded. Respect `prefers-reduced-motion`.

## Window & navigation

- A desktop app, not a web page: no site header or breadcrumbs, the arrow cursor on controls (a hand means
  "web link"), text not selectable outside fields and paths.
- **Rail** (76px, `components/shell/Rail.tsx`) on the window's left edge, on the native material where there is
  one: the mark, Library, each recent project as its picture (gold ring when open), New project, and at the bottom
  Search (`Ctrl/⌘ K`), Activity (a ring fills as work runs) and Settings. `Ctrl/⌘ B` hides it.
- **Top bar** (56px, `components/ui/topbar.tsx`) per view: back and title on the left, a project's sections
  (Overview · Episodes · Cast · Exports) as a pill in the middle, actions on the right. It is the drag region
  (double-click maximises); over a hero it is clear until the page scrolls, then glass with the title.
- **Native chrome** per platform: Windows 11 Mica behind the rail with our own caption buttons over the top-right
  corner (`--caption-inset`); macOS vibrancy with the traffic lights over the rail's top (`--traffic-inset`);
  Linux and Windows 10 solid. The material is only claimed where it exists (`detectMaterial`), so nothing turns
  see-through without it.
- The native window title follows the view (taskbar, Alt+Tab): "Cast · Walter — Dark — AutoScene".
- Background work is never page-bound: it shows in the rail's Activity ring and the Activity drawer (`Ctrl/⌘ J`).
- The palette (`Ctrl/⌘ K`) finds projects, places and commands, and in a project its characters and episodes
  (by name, `S01E03`, `1x03` or file). Back and Forward return a page to where it was scrolled.

## Menus & tooltips

- Nothing shows the browser's own chrome: no native tooltips, context menus, form controls, autofill, find bar or
  link drags. oxlint refuses `title` and native controls (`select`, `progress`, range and checkbox inputs…);
  browser keys the app has no use for are swallowed (`lib/browser-guards.ts`).
- **Tooltips** (`ui/tooltip.tsx`) are glass: the label, an optional second line in `--fg-3` (`hint`) and the key
  that does the same (`shortcut`). Text cut by its box (`TruncatedText`) shows the rest only when it is cut.
- **One context menu** for the window (`shell/AppMenu.tsx`). An element says what a right-click offers with
  `ContextArea`. The menu lists the clicked element's own entries first, innermost first: a shot's before its
  card's. The short tail follows: Command palette, Settings.
  - On a page's empty space (`Page menu`) the menu shows the page's entries, then the full app list: Back, Command
    palette, New project, Library, Activity, the rail, Keyboard shortcuts, Settings, Reload.
  - Dialogs and the drawer show only what they offer, never the app's entries, and no menu when they offer
    nothing.
  - On Windows the top bar's empty parts and its title are the window's caption: Minimize, Maximize, Close.
  - Selected text in a path or an error offers Copy first.
  - An entry runs once its menu has closed, so a field or a dialog it opens keeps the focus.
- Entries say what happens to the thing clicked ("Mark it as the clip", "Not Walter") and show the key that does
  the same.
  - An entry that can't apply to this thing is left out; one that can't apply yet is disabled.
  - Where the pointer picks the subject (a frame, a shot, an appearance on the timeline), a label names it first:
    "Shot 5 · 00:00:09:04".
  - A ⋯ menu and the right-click list the same actions from one list (the kits in `ui/menu-actions.tsx`).
- **Text fields** have their own menu (`shell/TextMenu.tsx`), in the same glass: Undo, Redo, Cut, Copy, Paste,
  Delete, Select all.
  - It never takes the focus from the field, and Escape closes only the menu.
  - Paste reads the clipboard natively, so the webview never asks for permission.
- While developing, Shift+right-click still opens the browser's menu (for Inspect).

## Views

- **Library:** the project touched last as a hero ("Continue"), then every project as a 16:9 picture card.
- **Project overview:** the hero, then shelves (`Section` + `Shelf`) of the cast as portraits and the episodes as
  stills. Files that can't be found show as a gold banner with Locate folder… here and on Episodes.
- **Cast:** portrait cards (3:4), Everyone else, hidden characters folded away; Review as the white pill.
- **Character:** blurred-face hero with the portrait, the name (click to rename) and facts as chips; Build a
  scenepack as the white pill; shots scrub on hover.
- **Scenepack:** the character's portrait and name, the filters in a glass bar that stays under the top bar (order,
  shot sizes as chips, length, episodes, limit, handles, "only them", flash cuts), then every shot of the character:
  the ones the pack takes bright with a gold tick, the rest dimmed with the reason. A click takes a shot in or out,
  Shift every shot since the last. A glass bar floats at the bottom with the count, length and size, the picks by
  hand and Export as the white pill; the export dialog makes one file with a chapter per shot (a switch at the top,
  on by default) or a file per shot, named from templates with name-part buttons.
- **Review:** one face against the suggestion, both big, the face lighting the room; answers on the keyboard.
- **Player:** a dark room — the picture on a rounded screen with its own ambilight, a large timecode and a white
  play button, the timeline in one card (ruler, scrubber, shots, character lanes), details on the right (shown or
  hidden from the top bar). The timeline zooms (Ctrl/⌘ + wheel, pinch, + and −, Shift+Z for the whole episode)
  and scrolls with the wheel; zoomed in, a thin overview of the episode shows the part in view. J/K/L shuttle
  like an editing suite (a gold chip shows the speed). Only the timecode, the playhead and the glow follow every
  video frame; shots and lanes are drawn on canvases.

## Copy

- Plain, confident, editor vocabulary: shot, cut, scene, cast, pack, export, handles, timecode.
- Never expose engine names in primary UI (CUDA, ONNX, TransNet, InsightFace, NVENC, ffmpeg). They belong in
  Settings → About only.
- No marketing tone, no emoji, no exclamation marks.
