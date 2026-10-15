# Identity bench

How well AutoScene turns faces into characters, measured against labels a
person gave by looking at every face track.

## Breaking Bad S01E01 (`bb_s01e01.labels.tsv`)

Labels for the BluRay remux of the pilot (58 min, 1080p, 23.976 fps, 83,593
frames) — the file itself is not in the repository. One row per face track
of the face pass of 2026-10-14: `start end name boxes` (frames on the
decoded frame grid; `boxes` holds every detection as `frame:x0,y0,x1,y1`, a
share of the frame). `?` is someone unnamed (an extra, a photo, not a face),
`x` a face nobody could tell (gas masks, dark backs of heads) — left out of
the score.

Labels are anchored by time and place, not by track id, the way the app
re-anchors the user's decisions after a re-analysis: a later face pass whose
tracker joins or splits tracks still finds its names. A track takes the name
most of its frames have, among labels it shares most of the shorter of the
two with and whose face is where the track's face is during those frames.

```sh
# 1. Face pass over the episode (needs the files of `faces_run`'s doc comment)
cargo run --release -p as-analysis --example faces_run -- <ort_dir> <models_dir> <ffmpeg_dir> <bb_dir> <out>
# 2. Score (link, assign, margin, unsure override the defaults)
cargo run --release -p as-analysis --example identity_bench -- score <out>.tracks bench/identity/bb_s01e01.labels.tsv
```

The main cast is everyone named with at least 30 s on screen. BCubed is
reported per track and per frame on screen (a long, clear shot counts more
than a blurry glimpse).

BCubed only sees faces that became a track. The line `main cast faces
(labels)` measures end to end over the labelled faces instead: the share a
track still holds (`in a track`), and the share held by a track of the
character named like them (`on their character`). It catches a model or
tracker change that drops faces. The labels came from the w600k_r50 face
pass, so that pass holds every labelled face by construction.

Results on S01E01 (main cast; defaults of each embedder):

| Embedder | per track P / R / F | screen time F | in a track | on their character |
|---|---|---|---|---|
| w600k_r50 (buffalo_l) | 0.987 / 0.824 / 0.898 | 0.953 | 1.000 | 0.976 |
| glintr100 (antelopev2) | 0.984 / 0.873 / 0.925 | 0.958 | 0.991 | 0.973 |
| AdaFace IR-101 (tools/model-prep) | 0.980 / 0.811 / 0.887 | 0.948 | – | – |

Lowering `FaceParams::min_track_quality` (0.05 → 0.02) or the norm floor
(14 → 13) for glintr100 moves `in a track` by at most 0.003 and costs
per-track F; the faces it drops are below any usable quality.
