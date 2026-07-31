from __future__ import annotations

import logging
from dataclasses import dataclass

import numpy as np

from autoscene.faces.base import FaceDetection
from autoscene.faces.quality import avatar_score_from_bgr, pick_best_avatar
from autoscene.identity_gates import (
    GATE_PRESETS,
    merge_forbid_set,
    pair_key,
    remap_forbid,
)

log = logging.getLogger(__name__)


@dataclass
class PersonCluster:
    person_id: int
    label: str
    detection_indices: list[int]
    centroid: np.ndarray
    detection_count: int
    clip_estimate: int
    screen_time_sec: float
    first_sec: float
    last_sec: float
    thumbnail_bgr: np.ndarray | None
    avatar_score: float = 0.0


@dataclass
class Segment:
    person_id: int
    # Export / pack plate: full scene-cut shot (cut → next cut).
    start_sec: float
    end_sec: float
    # Face-presence window inside the shot (WITH / co-cast honesty).
    preview_start_sec: float | None = None
    preview_end_sec: float | None = None
    # Timestamp of an actual detection of THIS person inside the shot,
    # used for thumbnails so a still never shows the opposing shot plate.
    face_time_sec: float | None = None


@dataclass
class SharedScene:
    """
    Timeline window for one co-presence component.

    Membership means each listed person has a segment that pairwise-overlaps
    another member's segment by >= min_presence_sec (connected components).
    This is NOT co-cast for clips: use cocast_pairs() for honest WITH edges.
    """

    scene_id: int
    start_sec: float
    end_sec: float
    person_ids: list[int]


def _face_presence_bounds(segment: Segment) -> tuple[float, float] | None:
    """Face-presence interval for WITH (prefer preview_*; else full plate)."""
    if segment.preview_start_sec is not None and segment.preview_end_sec is not None:
        a = float(segment.preview_start_sec)
        b = float(segment.preview_end_sec)
    else:
        a = float(segment.start_sec)
        b = float(segment.end_sec)
    if b < a:
        a, b = b, a
    if b - a < 1e-3:
        return None
    return a, b


def _segment_intervals(
    segments: list[Segment],
    *,
    face_presence: bool = False,
) -> list[tuple[float, float, int]]:
    rows: list[tuple[float, float, int]] = []
    for s in segments:
        if face_presence:
            bounds = _face_presence_bounds(s)
            if bounds is None:
                continue
            a, b = bounds
        else:
            a, b = float(s.start_sec), float(s.end_sec)
            if b <= a:
                continue
        rows.append((a, b, int(s.person_id)))
    return sorted(rows, key=lambda row: (row[0], row[1], row[2]))


def cocast_pairs(
    segments: list[Segment],
    min_overlap_sec: float = 0.2,
) -> list[tuple[int, int, float]]:
    """
    Honest co-cast graph: person pairs whose face-presence windows overlap
    by at least min_overlap_sec (sum per unordered pair).

    Uses preview_* when set so full-shot export plates do not falsely WITH
    two characters who only appear in different halves of the same cut.
    """
    intervals = _segment_intervals(segments, face_presence=True)
    if not intervals:
        return []

    pair_overlap: dict[tuple[int, int], float] = {}
    for i, (sa, ea, pa) in enumerate(intervals):
        for sb, eb, pb in intervals[i + 1 :]:
            if pa == pb:
                continue
            ov = min(ea, eb) - max(sa, sb)
            if ov >= min_overlap_sec:
                key = pair_key(pa, pb)
                pair_overlap[key] = pair_overlap.get(key, 0.0) + ov

    return sorted(
        (a, b, float(sec))
        for (a, b), sec in pair_overlap.items()
    )


def build_shared_scenes(
    segments: list[Segment],
    gap_sec: float = 0.35,
    min_segment_sec: float = 0.5,
    min_presence_sec: float = 0.2,
) -> list[SharedScene]:
    """
    Build timeline scenes from pairwise face-presence overlap.

    Segments from different people link when their face windows overlap by
    >= min_presence_sec. Each connected component becomes one scene; its
    time range is the union of member face windows. Single-person components
    are kept. Reverse-shot segments with no simultaneous face overlap never
    share a multi-person scene (union glue alone cannot create WITH).

    gap_sec is retained for API compatibility but no longer glues scenes;
    co-presence requires actual overlap, not adjacency.
    """
    del gap_sec  # legacy param; overlap graph replaces time-union glue

    intervals = _segment_intervals(segments, face_presence=True)
    if not intervals:
        return []

    n = len(intervals)
    parent = list(range(n))

    def find(i: int) -> int:
        while parent[i] != i:
            parent[i] = parent[parent[i]]
            i = parent[i]
        return i

    def union(i: int, j: int) -> None:
        ri, rj = find(i), find(j)
        if ri != rj:
            parent[ri] = rj

    for i, (sa, ea, pa) in enumerate(intervals):
        for j in range(i + 1, n):
            sb, eb, pb = intervals[j]
            if pa == pb:
                continue
            ov = min(ea, eb) - max(sa, sb)
            if ov >= min_presence_sec:
                union(i, j)

    buckets: dict[int, list[int]] = {}
    for i in range(n):
        buckets.setdefault(find(i), []).append(i)

    raw: list[SharedScene] = []
    for indices in buckets.values():
        starts = [intervals[i][0] for i in indices]
        ends = [intervals[i][1] for i in indices]
        us, ue = min(starts), max(ends)
        if ue - us < min_segment_sec:
            continue
        pids = sorted({intervals[i][2] for i in indices})
        raw.append(
            SharedScene(
                scene_id=0,
                start_sec=us,
                end_sec=ue,
                person_ids=pids,
            )
        )

    raw.sort(key=lambda s: (s.start_sec, s.end_sec, s.person_ids))
    for idx, scene in enumerate(raw):
        scene.scene_id = idx
    return raw


def _normalize_rows(X: np.ndarray) -> np.ndarray:
    X = np.asarray(X, dtype=np.float64)
    norms = np.linalg.norm(X, axis=1, keepdims=True)
    norms = np.maximum(norms, 1e-12)
    return X / norms


def merge_labels_by_centroid_sim(
    labels: np.ndarray,
    X: np.ndarray,
    min_sim: float,
    forbid: set[tuple[int, int]] | None = None,
) -> np.ndarray:
    """
    Greedily merge cluster pairs whose centroids have cosine similarity >= min_sim.
    Fixes HDBSCAN over-splits of the same identity (angle/lighting/expression).
    Pairs in `forbid` (co-present on screen) are never merged.
    """
    if labels.size == 0 or min_sim <= 0:
        return labels
    Xn = _normalize_rows(X)
    out = labels.copy()
    veto = set(forbid or ())
    while True:
        ids = sorted({int(i) for i in out.tolist() if i >= 0})
        if len(ids) < 2:
            break
        cents: dict[int, np.ndarray] = {}
        for cid in ids:
            mask = out == cid
            c = Xn[mask].mean(axis=0)
            n = float(np.linalg.norm(c))
            cents[cid] = c / n if n > 0 else c
        best_pair: tuple[int, int] | None = None
        best_sim = float(min_sim)
        for i, a in enumerate(ids):
            ca = cents[a]
            for b in ids[i + 1 :]:
                if pair_key(a, b) in veto:
                    continue
                sim = float(np.dot(ca, cents[b]))
                if sim >= best_sim:
                    best_sim = sim
                    best_pair = (a, b)
        if best_pair is None:
            break
        keep, drop = best_pair
        # Keep the denser / lower id as survivor
        keep_n = int(np.sum(out == keep))
        drop_n = int(np.sum(out == drop))
        if drop_n > keep_n or (drop_n == keep_n and drop < keep):
            keep, drop = drop, keep
        out[out == drop] = keep
        veto = remap_forbid(veto, keep, drop)
    return out


def _is_generic_label(label: str | None) -> bool:
    return not label or label.startswith("Person ")


def merge_person_clusters(
    people: list[PersonCluster],
    min_sim: float,
    *,
    anime_soft: bool = False,
    forbid: set[tuple[int, int]] | None = None,
) -> list[PersonCluster]:
    """
    Merge PersonCluster rows with highly similar centroids (post-stabilize).
    person_id pairs in `forbid` (seen together in the same frames) never merge:
    two faces on screen at once are two people regardless of embedding sim.
    """
    if len(people) < 2 or min_sim <= 0:
        return people
    remaining = list(people)
    veto = set(forbid or ())
    changed = True
    while changed and len(remaining) > 1:
        changed = False
        best_i = best_j = -1
        best_sim = float(min_sim)
        for i in range(len(remaining)):
            for j in range(i + 1, len(remaining)):
                if pair_key(remaining[i].person_id, remaining[j].person_id) in veto:
                    continue
                sim = float(np.dot(remaining[i].centroid, remaining[j].centroid))
                if sim >= best_sim:
                    best_sim = sim
                    best_i, best_j = i, j
        if best_i < 0:
            break
        a, b = remaining[best_i], remaining[best_j]
        if b.detection_count > a.detection_count or (
            b.detection_count == a.detection_count and b.person_id < a.person_id
        ):
            a, b = b, a
        idxs = sorted(set(a.detection_indices) | set(b.detection_indices))
        # Recompute centroid from unit blend of the two
        blend = a.centroid * a.detection_count + b.centroid * b.detection_count
        n = float(np.linalg.norm(blend))
        if n > 0:
            blend = blend / n
        # Keep clearer / more visible avatar (not denser-only)
        score_a = a.avatar_score or avatar_score_from_bgr(
            a.thumbnail_bgr, anime_soft=anime_soft
        )
        score_b = b.avatar_score or avatar_score_from_bgr(
            b.thumbnail_bgr, anime_soft=anime_soft
        )
        if score_b > score_a:
            thumb = b.thumbnail_bgr
            av_score = float(score_b)
        else:
            thumb = a.thumbnail_bgr
            av_score = float(score_a)
        # Keep a user-taught name from either side over a generic "Person N".
        label = a.label
        if _is_generic_label(a.label) and not _is_generic_label(b.label):
            label = b.label
        merged = PersonCluster(
            person_id=a.person_id,
            label=label,
            detection_indices=idxs,
            centroid=blend.astype(np.float64),
            detection_count=len(idxs),
            clip_estimate=0,
            screen_time_sec=max(a.screen_time_sec, b.screen_time_sec),
            first_sec=min(a.first_sec, b.first_sec),
            last_sec=max(a.last_sec, b.last_sec),
            thumbnail_bgr=thumb,
            avatar_score=av_score,
        )
        veto = remap_forbid(veto, a.person_id, b.person_id)
        remaining = [
            p
            for k, p in enumerate(remaining)
            if k not in (best_i, best_j)
        ]
        remaining.append(merged)
        changed = True
    remaining.sort(key=lambda p: (-p.detection_count, p.first_sec, p.person_id))
    return remaining


def cluster_faces(
    detections: list[FaceDetection],
    min_cluster_size: int = 8,
    min_samples: int = 3,
    merge_sim: float = 0.45,
    *,
    anime_soft: bool = False,
    cooccur_veto: int = 2,
    adjacency_gap_sec: float | None = None,
    adjacency_min_switches: int | None = None,
) -> tuple[np.ndarray, list[PersonCluster]]:
    if not detections:
        return np.array([], dtype=int), []

    X = _normalize_rows(np.stack([d.embedding for d in detections], axis=0))
    try:
        import hdbscan

        # epsilon=0: never pre-glue close CGI characters before purity gates.
        # Old 0.4 silently merged distinct stylized faces (Ahsoka/Bail class).
        clusterer = hdbscan.HDBSCAN(
            min_cluster_size=max(2, min_cluster_size),
            min_samples=max(1, min_samples),
            metric="euclidean",
            cluster_selection_method="eom",
            cluster_selection_epsilon=0.0,
        )
        labels = clusterer.fit_predict(X)
    except Exception:
        # Fallback if hdbscan binary missing.
        # Unit-vector euclidean: ||a-b|| = sqrt(2(1-cos)); eps 0.75 ≈ cos 0.72.
        # Old eps 0.95 (cos 0.55) chained distinct CGI characters together.
        from sklearn.cluster import DBSCAN

        log.warning("hdbscan unavailable — strict DBSCAN fallback (eps=0.75)")
        labels = DBSCAN(
            eps=0.75,
            min_samples=max(2, min_samples),
            metric="euclidean",
        ).fit_predict(X)

    # Same-frame + reverse-shot vetoes before any centroid-similarity merge.
    ts = np.asarray([float(d.timestamp_sec) for d in detections])
    anim = GATE_PRESETS["animation"]
    from dataclasses import replace

    veto_cfg = replace(
        anim,
        merge_cooccur_veto=max(1, int(cooccur_veto)),
        merge_adjacency_gap_sec=(
            float(adjacency_gap_sec)
            if adjacency_gap_sec is not None
            else anim.merge_adjacency_gap_sec
        ),
        merge_adjacency_min_switches=(
            int(adjacency_min_switches)
            if adjacency_min_switches is not None
            else anim.merge_adjacency_min_switches
        ),
    )
    forbid = merge_forbid_set(labels, ts, veto_cfg)
    labels = merge_labels_by_centroid_sim(labels, X, min_sim=merge_sim, forbid=forbid)

    people: list[PersonCluster] = []
    for label in sorted(set(labels.tolist())):
        if label < 0:
            continue
        idxs = [i for i, lb in enumerate(labels) if lb == label]
        subset = [detections[i] for i in idxs]
        emb = X[idxs]
        centroid = emb.mean(axis=0)
        n = np.linalg.norm(centroid)
        if n > 0:
            centroid = centroid / n
        times = sorted(d.timestamp_sec for d in subset)
        # rough screen time: unique sampled seconds
        unique_t = sorted(set(round(t, 2) for t in times))
        screen = 0.0
        if unique_t:
            screen = max(0.1, unique_t[-1] - unique_t[0])
            # refine via gaps later in segments
        best, av_score = pick_best_avatar(subset, anime_soft=anime_soft)
        thumb = best.thumbnail_bgr if best is not None else None
        people.append(
            PersonCluster(
                person_id=label,
                label=f"Person {label + 1}",
                detection_indices=idxs,
                centroid=centroid,
                detection_count=len(idxs),
                clip_estimate=0,
                screen_time_sec=screen,
                first_sec=times[0],
                last_sec=times[-1],
                thumbnail_bgr=thumb,
                avatar_score=float(av_score) if av_score >= 0 else 0.0,
            )
        )
    return labels, people


def coalesce_segments(
    segments: list[Segment],
    gap_sec: float = 2.5,
    *,
    same_plate_only: bool = False,
) -> list[Segment]:
    """
    Merge overlapping / nearly-adjacent same-person segments.
    same_plate_only=True: only merge identical start/end (shot plates); never cross cuts.
    """
    if not segments:
        return []
    by_person: dict[int, list[Segment]] = {}
    for seg in segments:
        by_person.setdefault(int(seg.person_id), []).append(seg)

    out: list[Segment] = []
    for person_id, segs in by_person.items():
        ordered = sorted(segs, key=lambda s: (s.start_sec, s.end_sec))
        cur_s = float(ordered[0].start_sec)
        cur_e = float(ordered[0].end_sec)
        cur_ps = float(
            ordered[0].preview_start_sec
            if ordered[0].preview_start_sec is not None
            else cur_s
        )
        cur_pe = float(
            ordered[0].preview_end_sec
            if ordered[0].preview_end_sec is not None
            else cur_e
        )
        cur_ft = ordered[0].face_time_sec
        for s in ordered[1:]:
            s_s = float(s.start_sec)
            s_e = float(s.end_sec)
            ps = float(s.preview_start_sec if s.preview_start_sec is not None else s_s)
            pe = float(s.preview_end_sec if s.preview_end_sec is not None else s_e)
            same_plate = abs(s_s - cur_s) < 1e-3 and abs(s_e - cur_e) < 1e-3
            if same_plate:
                cur_ps = min(cur_ps, ps)
                cur_pe = max(cur_pe, pe)
                cur_ft = cur_ft if cur_ft is not None else s.face_time_sec
            elif not same_plate_only and s_s <= cur_e + gap_sec:
                cur_e = max(cur_e, s_e)
                cur_ps = min(cur_ps, ps)
                cur_pe = max(cur_pe, pe)
                cur_ft = cur_ft if cur_ft is not None else s.face_time_sec
            else:
                out.append(
                    Segment(
                        person_id,
                        cur_s,
                        cur_e,
                        preview_start_sec=cur_ps,
                        preview_end_sec=cur_pe,
                        face_time_sec=cur_ft,
                    )
                )
                cur_s, cur_e, cur_ps, cur_pe = s_s, s_e, ps, pe
                cur_ft = s.face_time_sec
        out.append(
            Segment(
                person_id,
                cur_s,
                cur_e,
                preview_start_sec=cur_ps,
                preview_end_sec=cur_pe,
                face_time_sec=cur_ft,
            )
        )
    out.sort(key=lambda s: (s.person_id, s.start_sec))
    return out


def _median_time(hits: list[float], start: float, end: float) -> float:
    """Timestamp of a real detection inside the window (median hit, clamped)."""
    ordered = sorted(hits)
    mid = ordered[len(ordered) // 2]
    return float(min(max(mid, start), end))


def build_segments(
    detections: list[FaceDetection],
    labels: np.ndarray,
    gap_sec: float = 2.5,
    min_segment_sec: float = 0.5,
    shots: list | None = None,
    min_hits_per_shot: int = 2,
) -> list[Segment]:
    """
    Build per-person segments.

    With `shots` (scene-cut spans): one segment per person per shot where they
    appear at least `min_hits_per_shot` times. Export plate = full shot
    (cut → next cut). Face-presence is stored in preview_* for honest WITH.
    A single sampled frame never becomes a clip.

    Without shots: legacy gap-based presence windows (no cross-shot awareness).
    """
    by_person: dict[int, list[float]] = {}
    for i, lb in enumerate(labels.tolist()):
        if lb < 0:
            continue
        by_person.setdefault(lb, []).append(float(detections[i].timestamp_sec))

    if shots:
        from autoscene.scene_cuts import ShotSpan, shot_index_for_time

        shot_list: list[ShotSpan] = list(shots)
        segments: list[Segment] = []
        for person_id, times in by_person.items():
            times = sorted(times)
            if not times:
                continue
            # Group hits by shot index
            buckets: dict[int, list[float]] = {}
            for t in times:
                idx = shot_index_for_time(shot_list, t)
                buckets.setdefault(idx, []).append(t)
            for idx, hits in sorted(buckets.items()):
                if idx < 0 or idx >= len(shot_list):
                    continue
                # Gate: single-frame evidence cannot create a clip.
                if len(hits) < max(1, min_hits_per_shot):
                    continue
                sh = shot_list[idx]
                start = float(sh.start_sec)
                end = float(sh.end_sec)
                if end - start < 0.12:
                    continue
                # Face presence inside the shot (WITH / thumb honesty).
                pad = 0.12
                face_s = max(start, float(min(hits)) - pad)
                face_e = min(end, max(float(max(hits)) + pad, face_s + 0.2))
                if face_e < face_s:
                    face_s, face_e = face_e, face_s
                segments.append(
                    Segment(
                        person_id,
                        start,
                        end,
                        preview_start_sec=float(face_s),
                        preview_end_sec=float(face_e),
                        face_time_sec=_median_time(hits, start, end),
                    )
                )
        segments.sort(key=lambda s: (s.person_id, s.start_sec))
        return coalesce_segments(segments, gap_sec=0.0, same_plate_only=True)

    # Legacy path (no shot detector)
    segments = []
    for person_id, times in by_person.items():
        times = sorted(times)
        if not times:
            continue

        def flush(seg_times: list[float]) -> None:
            if len(seg_times) < 2:
                return  # single-frame orphan: no clip
            start = seg_times[0]
            end = seg_times[-1]
            if end - start < min_segment_sec:
                end = start + min_segment_sec
            segments.append(
                Segment(
                    person_id,
                    start,
                    end,
                    preview_start_sec=start,
                    preview_end_sec=end,
                    face_time_sec=_median_time(seg_times, start, end),
                )
            )

        run: list[float] = [times[0]]
        for t in times[1:]:
            if t - run[-1] > gap_sec:
                flush(run)
                run = [t]
            else:
                run.append(t)
        flush(run)

    segments.sort(key=lambda s: (s.person_id, s.start_sec))
    return coalesce_segments(segments, gap_sec=gap_sec)


def refine_person_stats(
    people: list[PersonCluster],
    segments: list[Segment],
) -> list[PersonCluster]:
    for person in people:
        segs = [s for s in segments if s.person_id == person.person_id]
        person.clip_estimate = len(segs)
        person.screen_time_sec = sum(max(0.0, s.end_sec - s.start_sec) for s in segs)
        if segs:
            person.first_sec = segs[0].start_sec
            person.last_sec = segs[-1].end_sec
    return people
