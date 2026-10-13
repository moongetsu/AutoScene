# AutoScene design system — Cinema Graphite

AutoScene is a professional tool for editors who live in Premiere, Resolve and After Effects. The UI should feel like
a calm, dense post-production suite: DaVinci Resolve's seriousness with Linear's precision. The footage is the
colour; the chrome stays neutral.

Tokens live in `app/src/styles/tokens.css` and are mapped into Tailwind in `app/src/styles/index.css`. Never use raw
hex values or Tailwind palette colours (`amber-500`, `zinc-800`…) in components.

## Brand

- **Mark:** the gold "A" — `app/src/components/brand/AutoSceneMark.tsx` (also `app/public/autoscene-mark.svg`,
  `app/src-tauri/icons/*`). Geometry and colours are fixed: tile `#1C1A17`, chevron and dot `#C4A35A`.
- The mark appears once in the chrome (title bar) and as the hero of empty states / About. Never add glows,
  outlines or rings around it.

## Colour

| Role | Token | Value |
|---|---|---|
| Inset wells (inputs, timeline track) | `--bg-inset` | `#0A0A0C` |
| Window canvas | `--bg-canvas` | `#0D0D0F` |
| Panels, sidebar, cards | `--bg-panel` | `#141417` |
| Raised controls | `--bg-raised` | `#1A1A1E` |
| Overlays (menus) | `--bg-overlay` | `#202025` |
| Hairline borders | `--line-hairline` | white 7.5% |
| Primary / secondary / tertiary text | `--fg-1/2/3` | `#EDEDEF` / `#A3A3AB` / `#707078` |
| **Accent (only one)** | `--accent` | `#C4A35A` (logo gold) |
| Destructive / errors only | `--danger` | `#D2564B` |

Rules:

- **Gold is the only accent.** Use it for the primary action on a screen, the active-nav indicator, focus rings,
  progress fills, selection, and the selected character in timelines. Everything else is graphite.
- No second accent: no blue for "manual", no green for "ready". Success is a neutral check; warnings use gold.
- Greys are neutral (no violet, brown or blue tint).
- Red is semantic only (delete, failure), never decorative.

## Type

- **Geist Variable** for UI; **Geist Mono Variable** for timecodes, frame numbers, paths and file sizes (`timecode`
  utility, tabular numerals).
- Base size 13px. Scale: 11 / 12 / 13 / 14 / 16 / 20 / 24 / 32. Headings 600 weight, −0.02em tracking.
- Section labels use the `eyebrow` utility (11px, uppercase, +0.06em, `--fg-3`).

## Space, shape, depth

- 4px grid. Page gutters 32px; panel padding 16–20px; control height 28/32/40px.
- Radii 4 / 6 / 8 / 12 / 16 — controls 6–8, cards 12, dialogs 16.
- Depth comes from surface steps plus hairlines and `--shadow-1` (a 1px top highlight). `--shadow-2/3` are for
  floating layers only.
- **Glass** (`glass` utility: blur + translucent graphite) is reserved for floating layers: command palette,
  popovers, tooltips, dialogs, the Activity drawer.

## Motion

- 120ms for hovers, 180ms for opening layers, 260ms for drawers. Ease `cubic-bezier(.2,.8,.2,1)`.
- Animate opacity and transform only. Respect `prefers-reduced-motion`.
- No bouncing, scaling buttons, or decorative loops. Progress and shimmer are the only continuous motion.

## Chrome & layout

- Title bar (40px): mark + breadcrumbs left, command search centred (`Ctrl/⌘ K`), Activity right; Windows caption
  buttons on Windows, traffic lights on macOS.
- Sidebar (232px, collapses to 56px): workspace navigation; Settings and collapse at the bottom.
- Background work is never page-bound: everything long-running appears in the **Activity** drawer (`Ctrl/⌘ J`).
- Keyboard first: every navigation and common action is in the command palette, with its shortcut shown.

## Copy

- Plain, confident, editor vocabulary: shot, cut, scene, cast, pack, export, handles, timecode.
- Never expose engine names in primary UI (CUDA, ONNX, TransNet, InsightFace, NVENC, ffmpeg). They belong in
  Settings → Hardware/About only.
- No marketing tone, no emoji, no exclamation marks.
