"""Precision gates for face identity.

A detection may carry a named identity only when every gate passes:

    1. detect quality   — det score, min face side, finite non-degenerate emb
    2. embedding sanity — unit-normalizable, no NaN/Inf
    3. temporal         — tracklet membership; single-frame orphans stay noise
    4. purity           — assign floor + top-2 ambiguity margin + member self-sim
    5. confirm          — median template of the cleanest members must agree
    6. merge veto       — same-frame co-presence AND reverse-shot adjacency

Ambiguity always resolves to noise (-1), never to the nearest person.
Presets: animation/anime are stricter than live-action because stylized
faces compress the embedding space (distinct characters sit closer).
"""

from __future__ import annotations

from dataclasses import dataclass

import numpy as np

NOISE = -1

# Bump when identity semantics change (gates, segments, teach rebuild).
# Projects below this version may set needs_rescan on load.
# v3: full scene-cut export plates (face windows only in preview_* / WITH).
IDENTITY_PIPELINE_VERSION = 4

# Cosine floor below which an embedding row is considered degenerate.
_MIN_EMB_NORM = 1e-3


@dataclass(frozen=True)
class GateConfig:
    # Gate 1 — detector quality
    min_det_score: float
    # Minimum face side (px) when bboxes are available at the quality gate.
    min_face_side: float
    # Laplacian variance floor on BGR thumbnails (soft-face reject). Skipped when
    # no thumb is available for a detection. Refs match faces/quality.py.
    min_laplacian_var: float
    # Gate 3 — temporal consistency (tracklets)
    min_track_len: int
    max_track_gap_sec: float
    # Link radius: max center distance as a fraction of the larger box side.
    track_center_dist_frac: float
    track_size_ratio_max: float
    # Gate 4 — assignment purity
    assign_min_sim: float
    assign_margin: float
    member_min_sim: float
    # Gate 5 — identity confirmation (median template of cleanest members)
    confirm_min_sim: float
    confirm_top_k: int
    # Tracklet voting: dominant label share required; rescue slack for
    # borderline frames of the same physical track.
    track_vote_frac: float
    track_rescue_slack: float
    # Gate 6 — merge veto: clusters sharing >= this many frames are two people.
    merge_cooccur_veto: int
    # Reverse-shot merge veto: adjacent label switches within this gap.
    merge_adjacency_gap_sec: float
    merge_adjacency_min_switches: int
    # Gate 7 — co-cast requires this much face-window overlap.
    cocast_min_overlap_sec: float


GATE_PRESETS: dict[str, GateConfig] = {
    # Precision-biased: prefer missed face / split cast over wrong merge.
    "live_action": GateConfig(
        min_det_score=0.62,
        min_face_side=40.0,
        min_laplacian_var=20.0,
        min_track_len=2,
        max_track_gap_sec=0.45,
        track_center_dist_frac=0.55,
        track_size_ratio_max=2.0,
        assign_min_sim=0.63,
        assign_margin=0.08,
        member_min_sim=0.65,
        confirm_min_sim=0.63,
        confirm_top_k=12,
        track_vote_frac=0.70,
        track_rescue_slack=0.04,
        merge_cooccur_veto=2,
        merge_adjacency_gap_sec=2.5,
        merge_adjacency_min_switches=2,
        cocast_min_overlap_sec=0.20,
    ),
    # CGI / stylized: same ArcFace stack as live; tighter because lookalikes
    # and plastic skin compress identity space. Reject-to-noise on ties.
    "animation": GateConfig(
        min_det_score=0.66,
        min_face_side=42.0,
        min_laplacian_var=10.0,
        min_track_len=3,
        max_track_gap_sec=0.40,
        track_center_dist_frac=0.50,
        track_size_ratio_max=1.9,
        assign_min_sim=0.67,
        assign_margin=0.10,
        member_min_sim=0.69,
        confirm_min_sim=0.67,
        confirm_top_k=12,
        track_vote_frac=0.72,
        track_rescue_slack=0.03,
        merge_cooccur_veto=2,
        merge_adjacency_gap_sec=2.5,
        merge_adjacency_min_switches=2,
        cocast_min_overlap_sec=0.20,
    ),
    # 2D cel via AnimeCV OML embeddings (EfficientDet already thresholds 0.58).
    "anime": GateConfig(
        min_det_score=0.64,
        min_face_side=36.0,
        min_laplacian_var=6.0,
        min_track_len=3,
        max_track_gap_sec=0.40,
        track_center_dist_frac=0.50,
        track_size_ratio_max=1.9,
        assign_min_sim=0.66,
        assign_margin=0.11,
        member_min_sim=0.68,
        confirm_min_sim=0.66,
        confirm_top_k=12,
        track_vote_frac=0.72,
        track_rescue_slack=0.03,
        merge_cooccur_veto=2,
        merge_adjacency_gap_sec=2.5,
        merge_adjacency_min_switches=2,
        cocast_min_overlap_sec=0.20,
    ),
}


def gate_config_for(preset: str | None, backend: str | None = None) -> GateConfig:
    """Resolve gates by content preset; anime backend forces the anime table.
    Unknown / custom presets fall back to the strictest table (animation)."""
    if (backend or "").strip().lower() in ("animecv", "anime"):
        return GATE_PRESETS["anime"]
    key = (preset or "").strip().lower()
    return GATE_PRESETS.get(key, GATE_PRESETS["animation"])


def unit_rows(X: np.ndarray) -> np.ndarray:
    X = np.asarray(X, dtype=np.float64)
    norms = np.linalg.norm(X, axis=1, keepdims=True)
    norms = np.maximum(norms, 1e-12)
    return X / norms


def embedding_ok(emb: np.ndarray) -> bool:
    """Gate 2: reject NaN/Inf and near-zero embeddings before they can vote."""
    v = np.asarray(emb, dtype=np.float64).reshape(-1)
    if v.size == 0 or not np.all(np.isfinite(v)):
        return False
    return float(np.linalg.norm(v)) >= _MIN_EMB_NORM


def quality_mask(
    det_scores: np.ndarray,
    embeddings: np.ndarray,
    cfg: GateConfig,
    *,
    bboxes: np.ndarray | None = None,
    laplacian_vars: np.ndarray | None = None,
) -> np.ndarray:
    """Gates 1+2 as a vectorized mask over the raw detection list.

    When bboxes are provided, faces smaller than ``min_face_side`` are
    rejected here so tiny / unstable crops never enter clustering.

    Blur reject uses ``min_laplacian_var`` against the parallel
    ``laplacian_vars`` array. Callers pass the values measured once at crop
    time (``FaceDetection.laplacian_var``) — this gate runs on every live
    publish, so it must never touch thumbnail pixels itself. Rows with no
    measurement (NaN) skip the blur gate so nothing is mass-rejected.
    """
    scores = np.asarray(det_scores, dtype=np.float64).reshape(-1)
    X = np.asarray(embeddings, dtype=np.float64)
    finite = np.all(np.isfinite(X), axis=1)
    norms = np.linalg.norm(X, axis=1)
    ok = (scores >= cfg.min_det_score) & finite & (norms >= _MIN_EMB_NORM)
    if bboxes is not None and len(bboxes):
        bb = np.asarray(bboxes, dtype=np.float64).reshape(-1, 4)
        if bb.shape[0] == ok.shape[0]:
            side = np.minimum(bb[:, 2] - bb[:, 0], bb[:, 3] - bb[:, 1])
            ok = ok & (side >= float(cfg.min_face_side))

    lv: np.ndarray | None = None
    if laplacian_vars is not None:
        lv = np.asarray(laplacian_vars, dtype=np.float64).reshape(-1)
    if lv is not None and lv.shape[0] == ok.shape[0]:
        measured = np.isfinite(lv)
        blur_ok = np.ones(ok.shape[0], dtype=bool)
        blur_ok[measured] = lv[measured] >= float(cfg.min_laplacian_var)
        ok = ok & blur_ok
    return ok


def shot_ids_for(timestamps: np.ndarray, shots: list | None) -> np.ndarray:
    """Shot index per detection; one implicit shot when no cuts exist."""
    ts = np.asarray(timestamps, dtype=np.float64).reshape(-1)
    if not shots:
        return np.zeros(ts.size, dtype=int)
    starts = np.asarray([float(s.start_sec) for s in shots])
    # searchsorted: shot whose start is the last one <= t
    idx = np.searchsorted(starts, ts, side="right") - 1
    return np.clip(idx, 0, len(shots) - 1)


def build_tracklets(
    timestamps: np.ndarray,
    bboxes: np.ndarray,
    shot_ids: np.ndarray,
    cfg: GateConfig,
    eligible: np.ndarray | None = None,
) -> list[list[int]]:
    """
    Gate 3: greedy spatio-temporal linking inside a shot.
    Two detections in the same frame can never share a track, so tracks
    cannot chain across two different faces standing close together.
    Returns index lists into the original detection order (all lengths).
    """
    ts = np.asarray(timestamps, dtype=np.float64).reshape(-1)
    n = ts.size
    if n == 0:
        return []
    bb = np.asarray(bboxes, dtype=np.float64).reshape(n, 4)
    sid = np.asarray(shot_ids, dtype=int).reshape(-1)
    ok = (
        np.ones(n, dtype=bool)
        if eligible is None
        else np.asarray(eligible, dtype=bool).reshape(-1)
    )

    cx = (bb[:, 0] + bb[:, 2]) / 2.0
    cy = (bb[:, 1] + bb[:, 3]) / 2.0
    side = np.maximum(bb[:, 2] - bb[:, 0], bb[:, 3] - bb[:, 1])
    side = np.maximum(side, 1.0)

    order = sorted((i for i in range(n) if ok[i]), key=lambda i: (ts[i], -side[i]))

    tracks: list[list[int]] = []
    # Open track state: (track_idx, last det idx)
    open_tracks: list[tuple[int, int]] = []

    for i in order:
        still_open: list[tuple[int, int]] = []
        for t_idx, last in open_tracks:
            if sid[last] == sid[i] and ts[i] - ts[last] <= cfg.max_track_gap_sec:
                still_open.append((t_idx, last))
        open_tracks = still_open

        best_track = -1
        best_score = -1.0
        for t_idx, last in open_tracks:
            if ts[i] - ts[last] <= 1e-6:
                continue  # same frame: must be a different face
            ratio = max(side[i], side[last]) / max(1.0, min(side[i], side[last]))
            if ratio > cfg.track_size_ratio_max:
                continue
            # IoU link first (robust to fast motion), center distance as the
            # fallback for zooms / re-detections where boxes stop overlapping.
            ix1 = max(bb[i, 0], bb[last, 0])
            iy1 = max(bb[i, 1], bb[last, 1])
            ix2 = min(bb[i, 2], bb[last, 2])
            iy2 = min(bb[i, 3], bb[last, 3])
            inter = max(0.0, ix2 - ix1) * max(0.0, iy2 - iy1)
            area_i = max(0.0, bb[i, 2] - bb[i, 0]) * max(0.0, bb[i, 3] - bb[i, 1])
            area_l = max(0.0, bb[last, 2] - bb[last, 0]) * max(
                0.0, bb[last, 3] - bb[last, 1]
            )
            union = area_i + area_l - inter
            iou = inter / union if union > 1e-6 else 0.0
            dist = float(np.hypot(cx[i] - cx[last], cy[i] - cy[last]))
            radius = cfg.track_center_dist_frac * max(side[i], side[last])
            if iou >= 0.25:
                score = 1.0 + iou
            elif dist <= radius:
                score = 1.0 - dist / max(radius, 1e-6)
            else:
                continue
            if score > best_score:
                best_score = score
                best_track = t_idx
        if best_track >= 0:
            tracks[best_track].append(i)
            open_tracks = [
                (t_idx, i if t_idx == best_track else last)
                for t_idx, last in open_tracks
            ]
        else:
            tracks.append([i])
            open_tracks.append((len(tracks) - 1, i))
    return tracks


def identity_tracks(tracks: list[list[int]], cfg: GateConfig) -> list[list[int]]:
    """Tracks long enough to carry identity. Orphans stay noise forever."""
    return [t for t in tracks if len(t) >= cfg.min_track_len]


def gated_assign(
    embeddings: np.ndarray,
    centroids: np.ndarray,
    cfg: GateConfig,
    *,
    tracks: list[list[int]] | None,
    quality: np.ndarray | None = None,
) -> np.ndarray:
    """
    Gates 4+5. Returns labels (len == n detections) with centroid ROW indices,
    NOISE elsewhere.

    tracks: identity-eligible tracklets (already length-filtered). Only their
    members can be labeled. tracks=None is the degraded mode for legacy
    detection caches without bboxes: per-detection gating without the
    temporal vote (still floor + margin + member + confirm).
    """
    X = unit_rows(embeddings)
    n = X.shape[0]
    labels = np.full(n, NOISE, dtype=int)
    if n == 0 or centroids is None or len(centroids) == 0:
        return labels
    C = unit_rows(np.asarray(centroids, dtype=np.float64))
    k = C.shape[0]

    sims = X @ C.T
    best = np.argmax(sims, axis=1)
    best_sim = sims[np.arange(n), best]
    if k >= 2:
        masked = sims.copy()
        masked[np.arange(n), best] = -1.0
        second_sim = np.max(masked, axis=1)
    else:
        second_sim = np.full(n, -1.0)
    # Margin applies even with a single known person: a lone cluster must not
    # swallow everything above the floor (the old n_people<2 shortcut did).
    confident = (best_sim >= cfg.assign_min_sim) & (
        (best_sim - second_sim) >= cfg.assign_margin
    )

    if tracks is None:
        member_lists = [[i] for i in range(n)]
        vote_mode = False
    else:
        member_lists = [list(t) for t in tracks]
        vote_mode = True

    for members in member_lists:
        conf_members = [m for m in members if confident[m]]
        if not conf_members:
            continue
        if not vote_mode:
            labels[members[0]] = int(best[members[0]])
            continue
        counts: dict[int, int] = {}
        for m in conf_members:
            counts[int(best[m])] = counts.get(int(best[m]), 0) + 1
        dom = max(counts, key=lambda key: counts[key])
        frac = counts[dom] / float(len(conf_members))
        if frac < cfg.track_vote_frac:
            continue  # identity conflict inside one physical track → all noise
        rescue_floor = cfg.assign_min_sim - cfg.track_rescue_slack
        for m in members:
            if confident[m]:
                # A member confident for a DIFFERENT person is a hard
                # conflict: noise, never outvoted into the dominant label.
                if int(best[m]) == dom:
                    labels[m] = dom
            elif float(sims[m, dom]) >= rescue_floor:
                # Same physical face across frames: temporal evidence
                # substitutes for the margin, never for the floor region.
                labels[m] = dom

    q = (
        np.ones(n, dtype=np.float64)
        if quality is None
        else np.asarray(quality, dtype=np.float64).reshape(-1)
    )

    # Purity pass: rebuild each person from its members, drop drifters.
    for row in range(k):
        idx = np.where(labels == row)[0]
        if idx.size < 2:
            labels[idx] = NOISE
            continue
        cent = X[idx].mean(axis=0)
        cn = float(np.linalg.norm(cent))
        if cn <= 0:
            labels[idx] = NOISE
            continue
        cent = cent / cn
        own = X[idx] @ cent
        drop = idx[own < cfg.member_min_sim]
        labels[drop] = NOISE

        # Confirm pass: median template of the cleanest members must agree.
        idx = np.where(labels == row)[0]
        if idx.size < 2:
            labels[idx] = NOISE
            continue
        top = idx[np.argsort(-q[idx])][: max(1, cfg.confirm_top_k)]
        template = np.median(X[top], axis=0)
        tn = float(np.linalg.norm(template))
        if tn <= 0:
            labels[idx] = NOISE
            continue
        template = template / tn
        agree = X[idx] @ template
        labels[idx[agree < cfg.confirm_min_sim]] = NOISE
        idx = np.where(labels == row)[0]
        if idx.size < 2:
            labels[idx] = NOISE
    return labels


def cooccurrence_forbid(
    labels: np.ndarray,
    timestamps: np.ndarray,
    min_frames: int,
) -> set[tuple[int, int]]:
    """
    Gate 6: label pairs that appear in the same sampled frame >= min_frames
    times are two distinct on-screen people — merging them is forbidden no
    matter how similar their centroids look.
    """
    lb = np.asarray(labels).reshape(-1)
    ts = np.asarray(timestamps, dtype=np.float64).reshape(-1)
    frames: dict[float, set[int]] = {}
    for label, t in zip(lb.tolist(), ts.tolist()):
        if label < 0:
            continue
        frames.setdefault(round(t, 3), set()).add(int(label))
    counts: dict[tuple[int, int], int] = {}
    for present in frames.values():
        ordered = sorted(present)
        for i, a in enumerate(ordered):
            for b in ordered[i + 1 :]:
                counts[(a, b)] = counts.get((a, b), 0) + 1
    return {pair for pair, c in counts.items() if c >= max(1, min_frames)}


def temporal_adjacency_forbid(
    labels: np.ndarray,
    timestamps: np.ndarray,
    *,
    max_gap_sec: float = 2.5,
    min_pair_events: int = 2,
) -> set[tuple[int, int]]:
    """
    Gate 6b: reverse-shot / ping-pong veto.

    Distinct labels that never share a frame can still be the same dialogue
    (A shot, B reverse, A, B). Centroid similarity alone must not merge them.
    Count consecutive timeline switches between a pair within ``max_gap_sec``;
    same-frame co-presence is ignored here (handled by cooccurrence_forbid).
    """
    lb = np.asarray(labels).reshape(-1)
    ts = np.asarray(timestamps, dtype=np.float64).reshape(-1)
    events = sorted(
        (
            (float(t), int(label))
            for t, label in zip(ts.tolist(), lb.tolist())
            if int(label) >= 0
        ),
        key=lambda row: (row[0], row[1]),
    )
    if len(events) < 2:
        return set()
    counts: dict[tuple[int, int], int] = {}
    for i in range(1, len(events)):
        t0, a = events[i - 1]
        t1, b = events[i]
        if a == b:
            continue
        if (t1 - t0) > float(max_gap_sec):
            continue
        # Same sampled frame: co-occurrence veto owns this pair.
        if round(t0, 3) == round(t1, 3):
            continue
        key = pair_key(a, b)
        counts[key] = counts.get(key, 0) + 1
    return {pair for pair, c in counts.items() if c >= max(1, int(min_pair_events))}


def merge_forbid_set(
    labels: np.ndarray,
    timestamps: np.ndarray,
    cfg: GateConfig,
) -> set[tuple[int, int]]:
    """Union of same-frame and reverse-shot merge vetoes."""
    forbid = cooccurrence_forbid(
        labels, timestamps, min_frames=cfg.merge_cooccur_veto
    )
    forbid |= temporal_adjacency_forbid(
        labels,
        timestamps,
        max_gap_sec=cfg.merge_adjacency_gap_sec,
        min_pair_events=cfg.merge_adjacency_min_switches,
    )
    return forbid


def pair_key(a: int, b: int) -> tuple[int, int]:
    return (a, b) if a <= b else (b, a)


def remap_forbid(
    forbid: set[tuple[int, int]],
    survivor: int,
    absorbed: int,
) -> set[tuple[int, int]]:
    """After merging `absorbed` into `survivor`, inherit its vetoes."""
    out: set[tuple[int, int]] = set()
    for a, b in forbid:
        a2 = survivor if a == absorbed else a
        b2 = survivor if b == absorbed else b
        if a2 == b2:
            continue
        out.add(pair_key(a2, b2))
    return out


def overlap_sec(a_start: float, a_end: float, b_start: float, b_end: float) -> float:
    return min(float(a_end), float(b_end)) - max(float(a_start), float(b_start))
