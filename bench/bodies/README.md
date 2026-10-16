# Body bench

What following people by their bodies adds to the face pass: characters
found while they face away, are too small or blurred for a face, or wear a
gas mask. Measured on the same episode and labels as `bench/identity`
(Breaking Bad S01E01, BluRay remux, 58 min, 794 shots, glintr100 face pass).

Three stages, from sure to unsure (`as_analysis::{bodies, body_tracker,
link}`, `as_identity::bodies`):

- **(a)** People are tracked through each shot; a body track tied to a face
  track is that face's character in its samples without a face too. More
  screen time and coverage — no new shots.
- **(b)** Faces the face pass left unassigned because they say too little
  (quality < 0.45), or unsure, take the character of the other faces on
  their body, where that character's face isn't on screen already.
  Measurable with the identity labels.
- **(c)** Bodies with no named face look for their person among the bodies
  with a face within ±3 min (one outfit per scene) by appearance (person
  ReID): the best character at ≥ 0.65 and ≥ 0.15 ahead of any other there —
  and no one if that character is on screen then. **Only this stage adds
  shots.**

## Models

InsightFace's `cheetah_l.zip` (model-zoo release, non-commercial licence
like the face models):

| File | SHA256 | What |
|---|---|---|
| `pp_det_med.onnx` | `6e9842a1…e22801` | PaddleDetection PP-YOLOE pedestrian detector, input `image` + `scale_factor` |
| `reid_0265.onnx` | `c15084e7…efef0f` | OpenVINO person-reidentification-retail-0277: 256×128 crop → 256-d |
| `reid_0265_b.onnx` | `9b51fe72…02ce939c` | its copy that batches: `tools/model-prep/inspect_onnx.py free-batch` (27 Reshapes pinned the batch at 1; outputs identical) |

Both exports run one frame at a time: `pp_det_med` returns every frame's
boxes as one list when given a batch. `cheetah_s`'s PicoDet
(`pp_det_small_320.onnx`) returns nothing on DirectML (103 people on the
CPU, 0 on the GPU over the same 100 frames); SCRFD's `scrfd_person_2.5g`
misses close-ups (2 % of faces taller than 30 % of the frame sit on a body).

## Commands

```sh
# Graph check (inputs, batch-1 shapes, DirectML-risky ops, batch parity)
uv run --project tools/model-prep python -I tools/model-prep/inspect_onnx.py info <model.onnx>
# Speed and GPU/CPU parity of the detectors and the ReID on 5 × 200 samples
cargo run --release -p as-analysis --example persons_bench -- <ort_dir> <persons_dir> <ffmpeg_dir> <bb_dir> 6000,20000,40000,60000,75000 200
# Every person of the episode's face-pass samples, once (≈ 4 min)
DETECTOR=pp_det_med:640x384:pad cargo run --release -p as-analysis --example bodies_run -- <ort_dir> <persons_dir> <ffmpeg_dir> <bb_dir> <out.bodies>
# Analysis on the CPU in a second: recall | tracks | score | reid | presence
BODY_STRIDE=6 cargo run --release -p as-analysis --example bodies_bench -- presence <faces.tracks> <out.bodies> <bb_dir> bench/identity/bb_s01e01.labels.tsv
# Sheets to look at, and their verdicts
cargo run --release -p as-analysis --example bodies_bench -- sheets <faces.tracks> <out.bodies> <bb_dir> <labels.tsv> <out_dir>
cargo run --release -p as-analysis --example bodies_render -- <ffmpeg_dir> <bb_dir> <out_dir>/items.tsv <out_dir>
cargo run --release -p as-analysis --example bodies_bench -- audit <out_dir>/items.tsv <verdicts.tsv>
```

`<faces.tracks>` is `faces_run`'s output (see `bench/identity`).

## Results (2026-10-16, RTX 3060 Ti, DirectML)

**Detectors** (`persons_bench`, 1000 samples at 1280×720; GPU and CPU boxes
identical for all but PicoDet):

| Detector | GPU ms/frame | CPU ms/frame | people/frame |
|---|---|---|---|
| pp_det_med 640×640, stretched | 7.35 | 41.9 | 1.11 |
| **pp_det_med 640×384, shape kept** | **4.75** | 25.3 | 1.00 |
| pp_det_small 320×320 | 2.51 | 7.0 | 0 (broken on DirectML) |
| scrfd_person 640×384 (batch 4) | 1.34 | 8.6 | 0.33 |
| reid_0265 (per crop, batch 32 / 1) | 0.66 / 1.53 | 5.35 | – |

**Labelled main-cast faces sitting on a detected body** (`recall`,
26,153 faces): pp_det_med 95.7 % at score ≥ 0.5, 99.3 % at ≥ 0.3; small
faces (< 8 % of the frame) 94.4 %. A third of all people found (score ≥ 0.5)
has no face of the face pass at that moment.

**Body tracks** (`tracks`, shape-kept detector): 2151 tracks, 855 carry a
face track; none of the 27 bodies with two or more named faces mixes two
people (0 of 60 faces on the wrong body; 1 of 83 before the review below
made the ties stricter). Every 2nd sample (stride 6): 1943 tracks, 0 of 60.

**(b) faces named by their body** (`score`, main cast, per track; only
faces too unclear to stand on their own, quality < 0.45, or unsure about
the body's character):

| | P | R | F | screen time F | on their character |
|---|---|---|---|---|---|
| faces only | 0.984 | 0.873 | 0.925 | 0.958 | 0.973 |
| + unassigned faces (1, 0 wrong) | 0.984 | 0.876 | 0.927 | 0.958 | 0.973 |
| + unassigned and unsure (8, 0 wrong) | 0.980 | 0.886 | 0.931 | 0.961 | 0.975 |

Without the quality gate it named 16 (R 0.898), among them a clear face in
a poster.

**(c) calibration** (`reid`): asking face-anchored bodies by their sightings
without a face (84 queries) at ≥ 0.65 / margin 0.15 / ±3 min: 88 % right,
25 % answered; 5 of 80 extras get a cast name. Pessimistic — the query's own
character is on screen, so the occupancy rule can't be used there; on the
real bodies without a face it rules out everyone already in the shot.

**More of the cast** (`presence`, stride 6, (b) with unsure, (c) at 0.65 /
0.15):

| | face s | face shots | +(a) s | +(c) s | +(c) shots |
|---|---|---|---|---|---|
| walter | 1522.9 | 302 | 142.5 | 101.0 | 26 (+8.6 %) |
| jesse | 466.4 | 117 | 42.9 | 44.2 | 10 (+8.5 %) |
| skyler | 436.2 | 66 | 28.5 | 3.8 | 1 |
| hank | 173.7 | 39 | 10.4 | 3.9 | 2 (+5.1 %) |
| krazy8 | 125.9 | 46 | 6.8 | 19.4 | 6 (+13 %) |
| main cast | | 713 | | | 45 (+6.3 %) |

**Looked at** (`bb_s01e01.audit.tsv`: frame, box, claim, verdict; judged by
Claude from the sheets). First round, all strata (stride 3, before the
review's fixes):

| | right | wrong | can't tell | precision (95 %) |
|---|---|---|---|---|
| (a) body without its face | 28 | 0 | 2 | 100 % (88–100 %) |
| (b) face named by its body | 13 | 1 (a face in a poster) | 2 | 93 % (69–99 %) |
| (c) body found by its clothes | 57 | 0 | 9 (tiny extras) | 100 % (94–100 %) |
| shots: who is in them | 38 | 0 | 1 | 100 % (91–100 %) |

Second round, the final code (stride 6), every (b) and (c) answer: 58 judged
in the first round (same person, nearby frame), 14 new.

| | right | wrong | can't tell | precision (95 %) |
|---|---|---|---|---|
| (b) face named by its body | 7 | 0 | 0 | 100 % (65–100 %) |
| (c) body found by its clothes | 57 | 1 | 7 (tiny extras) | 98 % (91–100 %) |

Main cast (c): 50 right, 1 wrong — a false person (chemical containers) in a
dark close-up in the RV, matched to an equally dark anchor of Walter. Two
right answers are a hand and an arm only: identity right, but no clip for a
pack (a prominence gate is needed). Still missed: backs of heads in the
foreground of over-the-shoulder shots, tiny or dark figures (7 of 39 shots
in the first round).

**Cost**: the detector is the cost — 5.5 ms per sample (with the resize).
Running in the face pass (decoding shared), every 2nd sample: ≈ 75 s for the
58-min episode, plus ≈ 15 s for ReID on ≤ 8 crops per body track — about
+40 % on the 225 s face pass. As its own pass it also decodes (119 s).
