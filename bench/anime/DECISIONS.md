# Anime bench decisions

One row per decision, in order. Every constant in `Params::anime()` and
`TrackerParams::anime()` traces back to a row here. Columns: step, rule (see
`README.md`), config hash, commit, data (media SHA-256 prefixes), result
table, outcome.

| # | Date | Step | Decision | Config | Commit | Data | Results | Outcome |
|---|---|---|---|---|---|---|---|---|
| 1 | 2026-10-30 | A3 | Pre-registration: split, labels, metrics, rules, targets, decision tree | – | (this commit) | `media.tsv` | – | fixed |
| 2 | 2026-10-30 | A4 | Estimators. Stratum reweighting: overall detection numbers use random keyframes only, strata use random + targeted. E1 picks one threshold per variant: the highest macro-mean recall over projects while every project keeps precision ≥ p; the bootstrap redraws the threshold in each resample (shots within each project are the clusters). Keyframes are uniform random frames over 1–99 % of each episode (seed 1), not shot-aware. Merges to clean per episode via `EPISODES=` (first frame of each later episode on the project timeline). | – | (this commit) | – | – | fixed |
