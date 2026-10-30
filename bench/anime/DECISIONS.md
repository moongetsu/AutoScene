# Anime bench decisions

One row per decision, in order. Every constant in `Params::anime()` and
`TrackerParams::anime()` traces back to a row here. Columns: step, rule (see
`README.md`), config hash, commit, data (media SHA-256 prefixes), result
table, outcome.

| # | Date | Step | Decision | Config | Commit | Data | Results | Outcome |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-10-30 | A3 | Pre-registration: split, labels, metrics, rules, targets, decision tree | – | (this commit) | `media.tsv` | – | fixed |
| 2 | 2026-10-30 | A4 | Estimators. Stratum reweighting: overall detection numbers use random keyframes only, strata use random + targeted. E1 picks one threshold per variant: the highest macro-mean recall over projects while every project keeps precision ≥ p; the bootstrap redraws the threshold in each resample (shots within each project are the clusters). Keyframes are uniform random frames over 1–99 % of each episode (seed 1), not shot-aware. Merges to clean per episode via `EPISODES=` (first frame of each later episode on the project timeline). | – | (this commit) | – | – | fixed |
| 3 | 2026-10-30 | A5 | Labeller calibration on AOT Characters (140 items, 14 characters; 2 items are dataset label errors: #72 shows Mikasa, #48 Titan Eren). Gate ≥ 98 % missed: one labeller 92–96 % on coded items; codes both labellers chose independently 81/83 = 97.6 % (both took Historia for Armin/Erwin); adjudicated codes 28/32. Response: every keyframe batch gets two labellers; a code counts as sure only when both chose it; adjudicated codes are marked `adjudicated` and fill the human audit first; look-alike pairs (blonde characters, matching uniforms) get extra gallery exemplars. | – | – | `E:/as-anime/aot/calib2` | answers_A/B/final.json | gate missed, procedure changed |
| 4 | 2026-10-30 | A5 | Unnamed creature heads (class `Hn` with `?e`/`?u`/`x`: curse swarms in JJK, monster mobs) become ignore regions `I` in the keyframe ground truth; named creatures (a gallery code) stay heads. Reason: in a 50-frame trial on JJK S2E03 one curse swarm made 60 of 94 heads and set recall to 0.34; clip packs never need unnamed swarm creatures. Detector boxes squeezed to zero area are dropped. Set before E1. | – | – | `gt_trial` | detect_bench score (trial) | fixed |
