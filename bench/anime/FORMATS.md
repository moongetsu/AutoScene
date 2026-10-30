# Anime bench file formats

All frames are indexes on the media's decoded frame grid. Boxes are
`x0,y0,x1,y1` as shares of the frame width and height. Text files are UTF-8,
tab-separated, with a header line; `#` starts a comment line.

## `gallery.tsv`, one per series

```
code	person	type	note
TAN	tanjiro	base	burgundy hair, hanafuda earrings, checked haori
NEZ_D	nezuko	transformation	demon form, vines on the face
```

`type` is one of `base`, `outfit`, `hairstyle`, `age`, `chibi`,
`transformation`, `mask`. The **figure** of a code is its `person` unless the
type is `transformation` or `mask`; then the figure is the code itself. The
three scoring levels are `look` (the code), `figure` and `person`.

## Track labels, `<project>.labels.tsv`

The format of `bench/identity`, plus an optional fifth column:

```
start	end	name	boxes	flags
```

`name` is a gallery code, `?e` (a real character outside the gallery, scored
as its own person), `?u` (unsure between codes, excluded and reported), `n`
(not a character's head, scored as its own person) or `x` (a real head nobody
can identify, excluded). Legacy files: `?` reads as `?e`, and `x` stays `x`.
`flags` is a comma list of `back`, `chibi`, `mask`, `graded`, `mono`, `impact`,
`flashback` (empty allowed). For multi-episode projects, frames run on one
project timeline: each media's frames are offset by the summed frame counts of
the media before it, in project order (`media.tsv`).

## Shot presence, `<project>.presence.tsv`

```
start	end	codes
```

One row per shot (`start` inclusive, `end` exclusive), `codes` is a comma list
of every gallery code visible in the shot in any view; empty means nobody.

## Repeated spans, `<project>.spans.tsv`

```
start	end	kind
```

`kind` is `op`, `ed`, `eyecatch`, `recap` or `preview`.

## Keyframe ground truth, `<project>.keyframes.json`

```json
{"frames": {"1234": {"heads": [
  {"box": [0.1, 0.2, 0.2, 0.4], "class": "H", "code": "TAN", "flags": ["back"]}
]}}}
```

`class` is `H`, `Hm`, `Hk`, `Hb` or `Hn` for a character's head, `O` for a
head that is not a character in the scene (poster, screen, statue), `N` for a
candidate box on no head, `S:<id>` for a second box on head `<id>`, and `I` for an ignore region: an unnamed creature head (swarms, monster mobs; class `Hn` with code `?e`, `?u` or `x`), which is neither a head to find nor a false alarm when detected. Every
frame lists all heads, including those no candidate found. Flags are those of
the track labels plus `dark`. Strata come from flags or class (`Hb` = back,
`Hk` = chibi, `Hn` = creature; `Hm` covers transformations too, so `mask` is
flag-only), box height (`small` < 0.08, `close` ≥ 0.33 of the frame height)
and the head count (`crowd` ≥ 5). A frame with `"targeted": true` was added to
fill a rare stratum: it counts in its strata, never in overall recall,
precision or threshold choice. Boxes may reach 0.02 outside the frame.

All `end` columns (`spans.tsv`, `presence.tsv`, `bb.segments.tsv`) are
exclusive.

## Detections, `<run>.dets.json`

```json
{"model": "head_detect_v2.0_s", "input": "letterbox640", "frames": {"1234": [[x0, y0, x1, y1, score]]}}
```
