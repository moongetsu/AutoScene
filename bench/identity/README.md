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

Results on S01E01 (main cast; defaults of each embedder; labels before the
corrections of 2026-10-17):

| Embedder | per track P / R / F | screen time F | in a track | on their character |
|---|---|---|---|---|
| w600k_r50 (buffalo_l) | 0.987 / 0.824 / 0.898 | 0.953 | 1.000 | 0.976 |
| glintr100 (antelopev2) | 0.984 / 0.873 / 0.925 | 0.958 | 0.991 | 0.973 |
| AdaFace IR-101 (tools/model-prep) | 0.980 / 0.811 / 0.887 | 0.948 | – | – |

Label corrections (2026-10-17, checked on the thumbnails): the face in bed
at 79806–80393 is Skyler, not Walter; `brownhair_blurry` and `teary` are one
woman (a guest at the party), now both `teary`; the two `?` faces
at 50976 and 50988 are Walter (his reflection in a glass door, and him in
the dark).

## One person, one character

Two faces found in the same frame are two people — except when they are
one person seen twice: Walt Jr. and Walter in front of the clothing store's
mirror, Krazy-8 found twice by the tracker in one gap. Before
`as_identity::VERSION` 2 a single such pair kept two groups of one person
apart for good: Walt Jr. was two characters (32 + 15 tracks). Now faces
count as seen together only when both were found in the same frame, and
after the strict clustering a second one joins whole groups whose mean
faces are clearly one person's (`Params::copy_link`) when only a few of
their faces were seen together (`copy_share`: 15 % of the smaller group,
at least one) — one face of each such pair steps aside (see
`as_identity::ahc`). How alike two faces side by side are doesn't count:
a twin beside their twin in the same good light looks like a mirror image.
A group seen (nearly) only with a bigger one, most of whose faces are the
face beside them seen twice, is a reflection, not a character. Twins who
are on screen together in 4 or more of 20 shots stay two — also when
one of them is blurrier, or clearer in the shots they share, and one
episode at a time in a big series. Unit tests in `as-identity` cover each
rule: switching any of 18 off (or the share to 25 % or 0) fails one.

Identical twins (or one actor in two roles) who are never seen apart
look like a reflection and lose their character; they can be given one
by hand (a character made by hand is pinned and stays).

glintr100, corrected labels:

| Grouping | characters | per track P / R / F | screen time F | in a track | on their character |
|---|---|---|---|---|---|
| before (VERSION 1) | 41 | 0.988 / 0.874 / 0.927 | 0.972 | 0.991 | 0.980 |
| VERSION 2 | 40 | 0.988 / 0.902 / 0.943 | 0.982 | 0.991 | 0.978 |

(`on their character` drops a little: the reflection of a mirror shot is
nobody now — its person is on screen in the same frames anyway.)

`identity_bench stress` on S01E01 — 120 mirror copies of random faces,
eight reflections shot after shot (0.53 – 0.65 like the face beside them,
as real ones are), the episode cut into 2, 4 and 8 episodes, 70 % of the
faces, link 0.25 – 0.4 and copy_link 0.5 – 0.7, the cast built up episode
by episode with characters kept: 27 runs, before 19 people split (Walt Jr.
in nearly every run), now 2 — the runs with 60 and 120 copies, where Walt
Jr.'s two halves get three or more mirror moments between them, as often
as twins seen together; characters holding two people: 0 before and after.
Reflections less than 0.5 like the face beside them stay a character of
their own: below that, identical twins always seen together look the same.

`identity_bench scale <tracks> <labels> <n>` simulates a series: the
episode `n` times over as `n` episodes, every named face keeping how much
it says about who it is but looking like another shot of that person.
People split (characters holding two people: 0 everywhere):

| episodes (faces) | 3 (3,414) | 5 (5,690) | 10 (11,380) | 20 (22,760) |
|---|---|---|---|---|
| before (VERSION 1) | 4 | 2 | 4 | 9 |
| VERSION 2 | 0 | 0 | 2 | 5 |

(A face seen twice at once stays a copy of its other face in every
simulated episode.) The groups left over at 10 and 20 episodes are rare
looks — mostly Jesse's — whose mean faces are 0.06 – 0.29 like the
person's, where two people's are up to 0.31: no grouping rule tells those
apart, a merge by hand does (it pins every face, so it holds through every
later grouping). Grouping 20 episodes takes about 6 s, as before.

What separates them, measured here: mean faces of one person's groups held
apart by a mirror or a stray face 0.71 – 0.83 (one face of 11 – 15 seen
together), of two people's groups 0.31 at most; people on screen together
80 % of the time or more (a class, a party): a face and the one beside it
0.24 at most — a face and its reflection 0.53 – 0.64.

When a grouping joins a character into another, its scenepack goes with
it (filters, shots picked or dropped). "Not them" doesn't: said of a
second character of one person, it may well have been said of that
person's own faces.

Lowering `FaceParams::min_track_quality` (0.05 → 0.02) or the norm floor
(14 → 13) for glintr100 moves `in a track` by at most 0.003 and costs
per-track F; the faces it drops are below any usable quality.

Characters without a visible face (bodies tracked through shots, found
again by their clothes): see `bench/bodies`.
