# Anime bench (pre-registration)

How AutoScene's anime character pipeline is chosen and judged. Everything in
this file is fixed **before** the first experiment (E1) runs; a change after
that is a new row in `DECISIONS.md` with its reason, and it may not touch the
test set. The media are not in the repository; only labels, rules and result
tables are.

## Material and split

Episodes are named by series and episode and identified by the SHA-256 of the
file used (list in `media.tsv`, written when the media were staged). Every
multi-episode row is analysed as one project, so cross-episode grouping is
measured.

| Role | Projects | Use |
|---|---|---|
| Tuning | Jujutsu Kaisen S2E03+E04; Re:Zero 31+33; One Piece, three Wano-arc episodes | every choice in E0–E6, by leave-one-franchise-out |
| Test (sealed) | Demon Slayer S4E6+E7; Dragon Ball Super, two Future Trunks arc episodes (Goku Black, Zamasu; files labelled "S04 EP05" and "S04 E15") | exactly one run, after the freeze, with the Rust build that ships |
| Reserve (sealed) | Demon Slayer S4E8, S3E11, S2E3; Re:Zero 26; One Piece East Blue (4:3), Marineford, Wano flashback, Egghead | a second test, only after a documented change; labelled only when opened |
| Scale / clones | Naruto: The Last (film) | speed, memory, labelled clone scenes; never a gate |
| Control (optional) | a western 2D episode | report only (CCIP's training leak on popular franchises) |

Nothing runs on test or reserve media before their event: no detection, no
error analysis, no app checks.

## Labels

Three levels from one label file: **look → figure → person**. Each look code
has a person and a type: base, outfit, hairstyle, age, chibi, transformation,
mask/persona. Figure map (product decision 2026-10-30): outfit, hairstyle,
age and chibi are their person's figure; transformation and mask/persona are
figures of their own. If that decision changes, labels are remapped, never
relabelled.

Track and keyframe codes:

| Code | Meaning | Scoring |
|---|---|---|
| look code | a known look | named |
| `?e` | a real character outside the gallery | its own person |
| `?u` | unsure between gallery codes | excluded, reported |
| `n` | not a character's head (object, poster, screen, statue, drawing, hair, hand) | its own person (costs precision when attached to a character) |
| `x` | a real head nobody can identify | excluded |

The back of a known character's head is their code plus the flag `back`,
never `x`. Flags: `back`, `chibi`, `mask`, `graded`, `mono`, `impact`,
`flashback`. OP/ED, eyecatch, recap and preview are labelled once per episode
as time spans; primary metrics exclude them and they are reported apart.

Kinds of ground truth:

1. **Keyframes**: 150 random shot frames per episode, plus targeted frames
   until each stratum (dark, smear/impact, crowd of 5 or more, chibi,
   creature, back of head) holds 50 heads; recall is reweighted by stratum
   frequency. Candidates are the union of all detector variants (merged at
   IoU 0.5) plus one source of another lineage; missed heads are boxed on
   3×3 zoomed tiles. Classes H, Hm (mask/helmet/transformation), Hk (chibi),
   Hb (back of head), Hn (creature/mascot), N, O (poster/screen/statue), S:id
   (second box on a head). Every head also carries its look code.
2. **Shot presence** (independent of any detector): every figure visible in a
   shot, in any view, from sheets of 480 px cells (8 per shot plus one per
   further second). On all test episodes and two tuning episodes. The main
   cast and presence recall come from here.
3. **Tracks**: from the E1 winner at score ≥ 0.10, stride 1, linked by box
   only (IoU ≥ 0.5 to the last box, gap ≤ 6 frames, an ambiguous match splits
   the track). Sheets show the head crop, the person crop, the frame with the
   box and three moments (≥ 256 px). `mix@<frame>` splits a track.

Galleries start from the AniList cast (MAIN and SUPPORTING, with AniList id)
plus one pass over the sheets for recurring characters. Sheets are ordered by
time, never by a candidate clustering. Test episodes get two labellers and an
adjudicator; tuning episodes one labeller plus a second on a 25 % sample.
Labellers are first calibrated on AOT Characters (CC BY 4.0, human-labelled):
code accuracy at least 98 %.

**Label freeze** follows a human audit of 150 items (50 adjudicated
disagreements, 50 look-alike codes, 50 random). Accept when code errors are
at most 1 % on random items and at most 5 % on look-alikes, else relabel that
stratum. Corrections are symmetric: every relabel triggered by a system error
triggers a blind audit of an equal random sample of agreements.

## Metrics

- BCubed P/R/F per track and per screen time, at figure level (primary),
  person and look level (reported).
- Main cast: figures with at least 30 s of presence in the project, from the
  shot-presence labels, fixed once.
- Coverage over time ("in a track", "on their figure").
- Over-segmentation: groups needed for 90 % of each main figure's time;
  "merges to clean" = that number − 1.
- Pack metrics with the app's shot semantics (a shot belongs to a character
  when any of their rows is in it): clip precision, clip recall, precision at
  10 and 25 in pack order.
- Presence recall per main figure.
- Confidence intervals: paired cluster bootstrap, 2,000 resamples, by shot
  for detection and by figure for identity. Objectives are per-series macro
  means; constraints hold per episode.

## Rules

Budget formula (RTX 3060 Ti, DirectML, static sessions, batch 4, measured in
Rust): `decode + det_ms × samples + emb_ms × crops ≤ 12 min` per 24-minute
episode (15 min hard limit).

| Step | Rule |
|---|---|
| E0 budget | candidates over the budget are dropped |
| E1 detector | recall over H, Hm, Hk, Hb, Hn (O, N, S are false alarms) at precision ≥ 0.95; take the fastest variant whose paired recall gap to the best has an upper 95 % bound below 2 points; threshold at precision 0.97 |
| E1b parity | box IoU ≥ 0.95 and ≤ 0.5 % unpaired boxes; CCIP cosine ≥ 0.9999 on identical tensors (CPU), ≥ 0.999 (DirectML) |
| Q0 quality | a factor stays only with Spearman ρ ≥ 0.1 against the identifiability margin; default score × size |
| E2 crop/embedder | out if the worst named look-alike pair has AUC < 0.90; top 3 by AUC and TAR@FAR on ground-truth negatives, within budget, stable under p95/p99; colour probe: global grading moves a crop ≤ 0.25 and a character-only colour change ≥ 0.5 of the stranger distance |
| E4 tracking/sampling | smallest stride with Track-F loss ≤ 0.005 and coverage loss ≤ 1 point; held-frame dedupe keeps ≥ 99 % of copied boxes at IoU ≥ 0.9 to a fresh detection |
| E3 grouping | at most 200 configs, listed in `DECISIONS.md` before the run; choose by the minimum over held-out franchises, at the centre of a plateau (neighbours within 0.01 F); constraints: the precision and clip-precision targets below, coverage ≥ best − 0.02 |
| E5 naming | per cluster; automatic names only when the upper Wilson bound of wrong names is ≤ 1 % over ≥ 300 decisions, otherwise suggestions only |
| E6 runtime | ≤ 15 min per episode; spurious OP/ED characters = 0, else repeated-segment detection; robustness loss (telecine, 4:3 pillarbox, burnt-in subtitles) ≤ 1 point |

Targets (test, figure level):

- Track-F ≥ 0.90 and time-F ≥ 0.95.
- Precision ≥ 0.97, every main figure ≥ 0.95.
- Clip precision ≥ 0.95.
- Presence recall per main figure ≥ 0.8, else listed as a known limit.
- Merges to clean ≤ 1 per main figure and episode.
- Stress: 0 merges. This counts only once a deliberately weakened config
  (link at the stranger quantile 0.80) does merge.
- `drop_copies` removes no main figure.
- Within budget.

Floor: Track-F ≥ 0.85 and precision ≥ 0.97.

## Decision tree after the test

1. Targets met: ship.
2. Floor met: ship as beta with the published numbers.
3. Below the floor: the feature stays behind its switch, and the oracle
   ladder on tuning decides the next work. Then the reserve test runs once.

Oracle ladder: substitute ground-truth boxes, then ground-truth tracks, then
label-oracle embeddings, and attribute the F gap to each stage. A failure is
"embedder-bound" when the embedder oracle closes at least two thirds of the
gap. Only then may an embedder be trained (one-week timebox; every bench
franchise excluded from its data; it must beat the best CCIP configuration by
≥ 0.02 F with a bootstrap lower bound > 0 on tuning cross-validation).

## Pinned models

Candidates are loaded from fixed Hugging Face commits (paths and SHA-256 in
`models.tsv`):

- `deepghs/anime_head_detection` @ `06604feee81983792a57c21081e539c0ae229833`
- `deepghs/anime_person_detection` @ `e39c744c22432ad01f91dd254fe2b02c8d878b8c`
- `deepghs/anime_face_detection` @ `784dc4c0bb692351ddcdbe6131a050b17d3025d5`
- `deepghs/anime_halfbody_detection` @ `95e21f43f13403930a78a87002f63b6d94c829e8`
- `deepghs/ccip_onnx` @ `eb2acdd29af1703388d3d0c04221add322bc9110`
