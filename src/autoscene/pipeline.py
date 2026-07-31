from __future__ import annotations

import logging
import time
from collections.abc import Callable
from pathlib import Path

import cv2
import numpy as np

from autoscene.cluster import (
    PersonCluster,
    build_segments,
    cluster_faces,
    merge_person_clusters,
    refine_person_stats,
)
from autoscene.config import settings
from autoscene.identity_gates import (
    GateConfig,
    build_tracklets,
    gate_config_for,
    gated_assign,
    identity_tracks,
    laplacian_var,
    quality_mask,
    shot_ids_for,
    unit_rows,
)
from autoscene.decode import (
    downscale_for_detect,
    expected_sample_count,
    iter_sampled_frames_prefetched,
    probe_video,
)
from autoscene.faces import create_backend
from autoscene.faces.base import FaceDetection
from autoscene.faces.filter import filter_faces_per_frame
from autoscene.faces.quality import pick_best_avatar
from autoscene.faces.thumb import make_face_thumbnail, render_hq_avatar
from autoscene.project import (
    PersonRecord,
    ProjectState,
    ProjectStore,
    SegmentRecord,
    ShotRecord,
)
from autoscene.scan_debug import ScanDebugger

log = logging.getLogger(__name__)

ProgressCb = Callable[[float, str], None]
CancelCb = Callable[[], bool]


def _avatar_anime_soft(project: ProjectState) -> bool:
    """Cel / anime backends: softer sharpness weight for avatar pick."""
    backend = (project.backend or "").lower()
    if backend == "animecv":
        return True
    preset = (getattr(settings, "content_preset", None) or "").lower()
    return preset in {"anime", "animation"}

# Phase budgets must sum to 1.0
PHASE_PROBE = 0.02
PHASE_MODEL = 0.04
PHASE_DETECT = 0.75
PHASE_CLUSTER = 0.08
PHASE_THUMBS = 0.09
PHASE_SEAL = 0.02

# Live gallery: re-cluster during detect so cast appears while scanning
# Live Gallery cast while scanning. Keep sparse — full HDBSCAN on 7k+ hits
# is CPU-heavy; a bug once stamped the interval at cluster *start* so a 45s
# cluster immediately re-triggered every frame.
LIVE_CLUSTER_MIN_HITS = 40
LIVE_CLUSTER_EVERY_HITS = 700
LIVE_CLUSTER_EVERY_SEC = 75.0
# Interim UI only: cluster on a time-spread subset, then assign all hits to
# nearest centroid. Final pass still uses every detection.
LIVE_CLUSTER_INTERIM_CAP = 1800
# Sliding window for ETA (avoid cumulative avg poisoned by slow start)
ETA_WINDOW_SEC = 20.0
_SHOT_CACHE: dict[str, list] = {}


def clear_shot_cache(project_id: str | None = None) -> None:
    if project_id is None:
        _SHOT_CACHE.clear()
    else:
        _SHOT_CACHE.pop(project_id, None)


def _spans_from_project(project: ProjectState) -> list | None:
    from autoscene.scene_cuts import ShotSpan

    if not project.shots:
        return None
    spans = [
        ShotSpan(float(s.start_sec), float(s.end_sec))
        for s in project.shots
        if float(s.end_sec) > float(s.start_sec)
    ]
    return spans or None


def _shots_for_project(project: ProjectState, *, allow_detect: bool = False) -> list | None:
    """
    Prefer persisted / in-memory shots. Detect only when allow_detect=True
    (pre-scan cut phase or teach refresh).
    """
    cached = _SHOT_CACHE.get(project.id)
    if cached is not None:
        return cached
    persisted = _spans_from_project(project)
    if persisted is not None:
        _SHOT_CACHE[project.id] = persisted
        return persisted
    if not allow_detect:
        return None

    from autoscene.scene_cuts import detect_shot_spans

    path = Path(project.video_path) if project.video_path else None
    if path is None or not path.is_file():
        return None
    mode = str(getattr(settings, "scene_cut_detector", "auto") or "auto")
    if mode not in ("adaptive", "transnet", "auto"):
        mode = "auto"
    try:
        spans, used = detect_shot_spans(
            path,
            float(project.duration_sec or 0.0),
            mode=mode,  # type: ignore[arg-type]
            adaptive_threshold=float(
                getattr(settings, "scene_cut_adaptive_threshold", 3.0)
            ),
            transnet_threshold=float(
                getattr(settings, "scene_cut_transnet_threshold", 0.5)
            ),
        )
    except Exception as exc:  # noqa: BLE001
        raise RuntimeError(str(exc)) from exc
    if used == "none":
        raise RuntimeError(
            "Scene cut detection failed — install TransNetV2 + CUDA torch "
            "from Settings → Model vault, then retry."
        )
    log.info(
        "shot spans project=%s detector=%s shots=%s",
        project.id,
        used,
        len(spans),
    )
    project.shots = [
        ShotRecord(start_sec=s.start_sec, end_sec=s.end_sec) for s in spans
    ]
    project.scene_cut_detector_used = used
    _SHOT_CACHE[project.id] = spans
    return spans


def run_scene_cuts_phase(
    store: ProjectStore,
    project: ProjectState,
    *,
    should_cancel: CancelCb | None = None,
    force: bool = True,
) -> ProjectState:
    """
    Pre-scan shot detection. Status=cutting. F3 fallback inside detect_shot_spans.
    Must complete before face scan starts.
    """
    if should_cancel and should_cancel():
        raise ScanCancelled(project.id)
    if store.is_deleted(project.id):
        raise ScanCancelled(project.id)

    clear_shot_cache(project.id)
    if not force and project.shots and project.scene_cut_detector_used not in ("", "none"):
        spans = _spans_from_project(project)
        if spans:
            _SHOT_CACHE[project.id] = spans
            return project

    video = Path(project.video_path)
    if not video.is_file():
        project.status = "error"
        project.error = f"Video missing: {video}"
        project.stage = "Error"
        store.save(project)
        raise FileNotFoundError(project.error)

    project.status = "cutting"
    project.progress = 0.05
    project.stage = "Preparing scene pass…"
    project.progress_phase = "cuts"
    project.error = None
    project.eta_sec = None
    project.cuts_found = 0
    project.frames_done = 0
    project.frames_total = 0
    store.save(project)

    info = probe_video(video)
    project.duration_sec = info.duration_sec
    if should_cancel and should_cancel():
        raise ScanCancelled(project.id)

    mode = str(getattr(settings, "scene_cut_detector", "auto") or "auto")
    project.progress = 0.12
    project.stage = "Finding scene breaks…"
    project.progress_phase = "cuts"
    store.save(project)

    from autoscene.scene_cuts import detect_shot_spans

    if mode not in ("adaptive", "transnet", "auto"):
        mode = "auto"

    last_save = 0.0

    def on_cut_progress(ev: dict) -> None:
        nonlocal last_save
        if should_cancel and should_cancel():
            raise ScanCancelled(project.id)
        if store.is_deleted(project.id):
            raise ScanCancelled(project.id)
        project.status = "cutting"
        raw_phase = str(ev.get("phase") or "")
        if raw_phase == "extract":
            project.progress_phase = "cuts_extract"
        elif raw_phase == "infer":
            project.progress_phase = "cuts_infer"
        elif raw_phase == "finalize":
            project.progress_phase = "cuts_finalize"
        else:
            project.progress_phase = "cuts"
        project.progress = float(max(0.0, min(1.0, ev.get("progress", project.progress))))
        project.stage = str(ev.get("stage") or project.stage)
        if "frames_done" in ev:
            project.frames_done = int(ev["frames_done"])
        if "frames_total" in ev:
            project.frames_total = int(ev["frames_total"])
        if "cuts_found" in ev:
            project.cuts_found = int(ev["cuts_found"])
        store.remember(project)
        now = time.monotonic()
        if now - last_save >= 0.45 or project.progress >= 0.95:
            last_save = now
            store.save(project)

    try:
        spans, used = detect_shot_spans(
            video,
            float(project.duration_sec or 0.0),
            mode=mode,  # type: ignore[arg-type]
            adaptive_threshold=float(
                getattr(settings, "scene_cut_adaptive_threshold", 3.0)
            ),
            transnet_threshold=float(
                getattr(settings, "scene_cut_transnet_threshold", 0.5)
            ),
            on_progress=on_cut_progress,
        )
    except Exception as exc:  # noqa: BLE001
        if isinstance(exc, ScanCancelled):
            raise
        project.status = "error"
        project.error = "Could not find scene breaks. Try again, or install packs in Settings."
        project.stage = "Error"
        project.progress_phase = "cuts"
        store.save(project)
        raise RuntimeError(project.error) from exc
    if should_cancel and should_cancel():
        raise ScanCancelled(project.id)

    if used == "none":
        project.status = "error"
        project.error = (
            "Scene detection is not ready. Open Settings → Packs, install required packs, "
            "then retry."
        )
        project.stage = "Error"
        project.progress_phase = "cuts"
        store.save(project)
        raise RuntimeError(project.error)

    project.shots = [
        ShotRecord(start_sec=float(s.start_sec), end_sec=float(s.end_sec))
        for s in spans
    ]
    project.scene_cut_detector_used = used
    project.cuts_found = len(spans)
    project.progress = 1.0
    project.stage = f"Scenes ready · {len(spans)} · snap-locked"
    project.progress_phase = "cuts"
    store.save(project)
    _SHOT_CACHE[project.id] = spans
    log.info(
        "pre-scan cuts done project=%s detector=%s shots=%s (hard-cut snap on)",
        project.id,
        used,
        len(spans),
    )
    return project


class ScanCancelled(Exception):
    """Raised when a scan is aborted by user request."""


def _map_faces_to_source(
    faces: list[FaceDetection],
    source_bgr: np.ndarray,
    coord_scale: float,
    min_face_size: int,
) -> list[FaceDetection]:
    """Map detect-frame bboxes/thumbs into native source pixels."""
    if coord_scale == 1.0:
        return faces
    sh, sw = source_bgr.shape[:2]
    out: list[FaceDetection] = []
    for face in faces:
        x1, y1, x2, y2 = face.bbox
        ix1 = max(0, int(round(x1 * coord_scale)))
        iy1 = max(0, int(round(y1 * coord_scale)))
        ix2 = min(sw - 1, int(round(x2 * coord_scale)))
        iy2 = min(sh - 1, int(round(y2 * coord_scale)))
        bw, bh = ix2 - ix1, iy2 - iy1
        if bw < min_face_size or bh < min_face_size:
            continue
        thumb = make_face_thumbnail(source_bgr, (ix1, iy1, ix2, iy2))
        out.append(
            FaceDetection(
                timestamp_sec=face.timestamp_sec,
                bbox=(ix1, iy1, ix2, iy2),
                embedding=face.embedding,
                det_score=face.det_score,
                thumbnail_bgr=thumb,
            )
        )
    return out


def _save_live_preview(
    store: ProjectStore,
    project_id: str,
    face: FaceDetection,
    frame_bgr,
) -> None:
    """Write a 1:1 face crop from the source frame at native resolution (no downscale)."""
    x1, y1, x2, y2 = face.bbox
    h, w = frame_bgr.shape[:2]
    cx = (x1 + x2) / 2.0
    cy = (y1 + y2) / 2.0
    bw, bh = max(1, x2 - x1), max(1, y2 - y1)
    side = int(max(bw, bh) * 1.35)
    side = max(side, 64)
    half = side / 2.0
    xa = int(round(cx - half))
    ya = int(round(cy - half))
    xb = xa + side
    yb = ya + side
    # Shift square into frame bounds
    if xa < 0:
        xb -= xa
        xa = 0
    if ya < 0:
        yb -= ya
        ya = 0
    if xb > w:
        xa -= xb - w
        xb = w
    if yb > h:
        ya -= yb - h
        yb = h
    xa, ya = max(0, xa), max(0, ya)
    crop = frame_bgr[ya:yb, xa:xb]
    if crop.size == 0:
        if face.thumbnail_bgr is None or face.thumbnail_bgr.size == 0:
            return
        crop = face.thumbnail_bgr
    # Exact square from source pixels — never resize down for preview
    ch, cw = crop.shape[:2]
    if ch != cw:
        m = min(ch, cw)
        y0 = (ch - m) // 2
        x0 = (cw - m) // 2
        crop = crop[y0 : y0 + m, x0 : x0 + m]
    path = store.preview_latest_path(project_id)
    tmp = path.with_name(path.stem + ".tmp" + path.suffix)
    cv2.imwrite(str(tmp), crop, [int(cv2.IMWRITE_JPEG_QUALITY), 95])
    try:
        tmp.replace(path)
    except OSError:
        # Windows may briefly lock destination; fall back to direct write
        cv2.imwrite(str(path), crop, [int(cv2.IMWRITE_JPEG_QUALITY), 95])
        try:
            tmp.unlink(missing_ok=True)
        except OSError:
            pass


def _stabilize_live_people(
    people: list[PersonCluster],
    prev: list[tuple[np.ndarray, PersonRecord]],
) -> list[PersonCluster]:
    """
    Match new clusters to prior live cast by centroid so IDs/labels stick.
    Requires a clear winner (floor + margin vs second-best prior); when in
    doubt a new id is minted — a fresh "Person N" is recoverable, a stolen
    user-taught label on the wrong face is not.
    """
    if not people:
        return []
    ordered = sorted(people, key=lambda p: (-p.detection_count, p.first_sec))
    used_prev: set[int] = set()
    used_ids: set[int] = set()
    next_id = 0
    if prev:
        next_id = max(rec.person_id for _, rec in prev) + 1

    from autoscene.identity_gates import gate_config_for

    cfg = gate_config_for(settings.content_preset)
    match_floor = max(float(cfg.assign_min_sim), float(settings.cluster_merge_sim))
    match_margin = float(cfg.assign_margin)
    for person in ordered:
        best_i = -1
        best_sim = match_floor
        second_sim = -1.0
        for i, (cent, _rec) in enumerate(prev):
            if i in used_prev:
                continue
            sim = float(np.dot(person.centroid, cent))
            if sim > best_sim:
                second_sim = best_sim if best_i >= 0 else second_sim
                best_sim = sim
                best_i = i
            elif sim > second_sim:
                second_sim = sim
        ambiguous = best_i >= 0 and second_sim > 0 and (
            best_sim - second_sim
        ) < match_margin
        if best_i >= 0 and not ambiguous:
            _cent, rec = prev[best_i]
            used_prev.add(best_i)
            person.person_id = rec.person_id
            used_ids.add(rec.person_id)
            if rec.label and not rec.label.startswith("Person "):
                person.label = rec.label
            else:
                person.label = f"Person {rec.person_id + 1}"
        else:
            while next_id in used_ids:
                next_id += 1
            person.person_id = next_id
            used_ids.add(next_id)
            person.label = f"Person {next_id + 1}"
            next_id += 1
    return ordered


def _time_spread_indices(n: int, cap: int) -> np.ndarray:
    """Evenly spaced indices across [0, n) for interim clustering."""
    if n <= cap:
        return np.arange(n, dtype=int)
    return np.unique(np.linspace(0, n - 1, num=cap, dtype=int))


def _detection_arrays(
    detections: list[FaceDetection],
) -> tuple[np.ndarray, np.ndarray, np.ndarray, np.ndarray]:
    """(timestamps, bboxes, det_scores, embeddings) as dense arrays."""
    n = len(detections)
    ts = np.asarray([float(d.timestamp_sec) for d in detections], dtype=np.float64)
    bbs = np.asarray([d.bbox for d in detections], dtype=np.float64).reshape(n, 4)
    scores = np.asarray([float(d.det_score) for d in detections], dtype=np.float64)
    X = np.stack(
        [np.asarray(d.embedding, dtype=np.float64).reshape(-1) for d in detections],
        axis=0,
    )
    return ts, bbs, scores, X


def _labels_from_people(
    people: list[PersonCluster],
    n_detections: int,
) -> np.ndarray:
    labels = np.full(n_detections, -1, dtype=int)
    for person in people:
        for idx in person.detection_indices:
            if 0 <= idx < n_detections:
                labels[idx] = person.person_id
    return labels


def _assign_detections_to_people(
    detections: list[FaceDetection],
    people: list[PersonCluster],
    *,
    tracks: list[list[int]],
    cfg: GateConfig,
    anime_soft: bool = False,
) -> list[PersonCluster]:
    """
    Re-index people against the full detection list through the identity
    gates (floor + margin + tracklet vote + member purity + median confirm).
    Only tracklet members can be labeled; everything ambiguous stays noise.
    """
    if not people or not detections:
        return people
    _ts, _bbs, scores, X = _detection_arrays(detections)
    C = np.stack([np.asarray(p.centroid, dtype=np.float64) for p in people], axis=0)
    labels = gated_assign(X, C, cfg, tracks=tracks, quality=scores)
    Xn = unit_rows(X)

    rebuilt: list[PersonCluster] = []
    for row, person in enumerate(people):
        keep = np.where(labels == row)[0].tolist()
        if len(keep) < 2:
            continue
        subset = [detections[i] for i in keep]
        emb = Xn[keep]
        centroid = emb.mean(axis=0)
        cn = float(np.linalg.norm(centroid))
        if cn > 0:
            centroid = centroid / cn
        times = sorted(d.timestamp_sec for d in subset)
        best_face, av_score = pick_best_avatar(subset, anime_soft=anime_soft)
        rebuilt.append(
            PersonCluster(
                person_id=person.person_id,
                label=person.label,
                detection_indices=keep,
                centroid=centroid,
                detection_count=len(keep),
                clip_estimate=person.clip_estimate,
                screen_time_sec=person.screen_time_sec,
                first_sec=times[0],
                last_sec=times[-1],
                thumbnail_bgr=best_face.thumbnail_bgr if best_face else None,
                avatar_score=float(av_score) if av_score >= 0 else 0.0,
            )
        )
    return rebuilt


def _publish_live_cast(
    store: ProjectStore,
    project: ProjectState,
    detections: list[FaceDetection],
    *,
    prev_live: list[tuple[np.ndarray, PersonRecord]],
    interim: bool,
) -> list[tuple[np.ndarray, PersonRecord]]:
    """
    Cluster current detections and write people/thumbs/segments for Gallery.
    Keeps status=scanning when interim=True.
    Always re-picks best avatar each publish (no sticky weak thumb).
    """
    if len(detections) < 8:
        return prev_live

    anime_soft = _avatar_anime_soft(project)
    cfg = gate_config_for(settings.content_preset, project.backend)
    shots = _shots_for_project(project, allow_detect=False)

    # Gates 1-3: quality mask + tracklets. Orphans and weak detections can
    # never enter clustering, assignment, or segments.
    ts, bboxes, det_scores, X = _detection_arrays(detections)
    lap_vars = np.asarray(
        [
            laplacian_var(d.thumbnail_bgr)
            if d.thumbnail_bgr is not None
            else np.nan
            for d in detections
        ],
        dtype=np.float64,
    )
    q_ok = quality_mask(
        det_scores, X, cfg, bboxes=bboxes, laplacian_vars=lap_vars
    )
    shot_ids = shot_ids_for(ts, shots)
    tracks = identity_tracks(
        build_tracklets(ts, bboxes, shot_ids, cfg, eligible=q_ok), cfg
    )
    eligible = sorted({i for t in tracks for i in t})
    log.info(
        "identity gates hits=%s quality_ok=%s rejected=%s tracked=%s tracks=%s preset=%s",
        len(detections),
        int(q_ok.sum()),
        int((~q_ok).sum()),
        len(eligible),
        len(tracks),
        settings.content_preset or "custom",
    )
    if len(eligible) < 8:
        return prev_live

    min_size = settings.cluster_min_size
    if interim:
        # Appear earlier in the scan; final pass uses full min size.
        min_size = max(3, min(settings.cluster_min_size, max(3, settings.cluster_min_size // 2)))

    pool_idx = np.asarray(eligible, dtype=int)
    used_subset = False
    if interim and pool_idx.size > LIVE_CLUSTER_INTERIM_CAP:
        pick = _time_spread_indices(pool_idx.size, LIVE_CLUSTER_INTERIM_CAP)
        pool_idx = pool_idx[pick]
        used_subset = True
    cluster_dets = [detections[int(i)] for i in pool_idx]

    _cluster_labels, people = cluster_faces(
        cluster_dets,
        min_cluster_size=min_size,
        merge_sim=settings.cluster_merge_sim,
        anime_soft=anime_soft,
        cooccur_veto=cfg.merge_cooccur_veto,
        adjacency_gap_sec=cfg.merge_adjacency_gap_sec,
        adjacency_min_switches=cfg.merge_adjacency_min_switches,
    )
    if not people:
        return prev_live

    # Gates 4-5: re-anchor every person on the full detection list.
    people = _assign_detections_to_people(
        detections, people, tracks=tracks, cfg=cfg, anime_soft=anime_soft
    )
    if not people:
        return prev_live

    # Stabilize IDs with gate floor+margin (never soft-match a taught name).
    people = _stabilize_live_people(people, prev_live)
    # Gate 6: same-frame + reverse-shot pairs must never merge.
    from autoscene.identity_gates import merge_forbid_set

    forbid = merge_forbid_set(
        _labels_from_people(people, len(detections)),
        ts,
        cfg,
    )
    people = merge_person_clusters(
        people,
        min_sim=settings.cluster_merge_sim,
        anime_soft=anime_soft,
        forbid=forbid,
    )
    # Purity re-pass after merges (centroids moved).
    people = _assign_detections_to_people(
        detections, people, tracks=tracks, cfg=cfg, anime_soft=anime_soft
    )
    if not people:
        return prev_live

    stable_labels = _labels_from_people(people, len(detections))

    # Animation needs more face evidence per shot before a clip is born.
    # Animation no longer needs 3 hits/shot — balanced gates already filter junk.
    min_hits = 2
    segments = build_segments(
        detections,
        stable_labels,
        gap_sec=settings.segment_gap_sec,
        min_segment_sec=settings.min_segment_sec,
        shots=shots,
        min_hits_per_shot=min_hits,
    )
    people = refine_person_stats(people, segments)

    if not interim and detections:
        try:
            from autoscene.identity_adapter import save_detection_cache

            save_detection_cache(
                store._dir(project.id),
                timestamps=ts,
                embeddings=X.astype(np.float32),
                labels=stable_labels.astype(np.int32),
                bboxes=bboxes.astype(np.float32),
                det_scores=det_scores.astype(np.float32),
            )
        except Exception as exc:  # noqa: BLE001
            log.warning("detection cache save failed: %s", exc)

    thumbs = store.thumbs_dir(project.id)
    records: list[PersonRecord] = []
    next_prev: list[tuple[np.ndarray, PersonRecord]] = []
    video_path = Path(project.video_path) if project.video_path else None
    for person in people:
        thumb_path = None
        subset = [
            detections[i]
            for i in person.detection_indices
            if 0 <= i < len(detections)
        ]
        best_face, av_score = pick_best_avatar(subset, anime_soft=anime_soft)
        thumb_bgr = (
            best_face.thumbnail_bgr
            if best_face is not None
            else person.thumbnail_bgr
        )
        # Final publish: re-crop from source at 512px so cast tiles stay crisp
        if (
            not interim
            and best_face is not None
            and video_path is not None
            and video_path.is_file()
        ):
            hq = render_hq_avatar(
                video_path,
                best_face.timestamp_sec,
                best_face.bbox,
                size=512,
                pad_scale=1.25,
                fallback_bgr=thumb_bgr,
            )
            if hq is not None:
                thumb_bgr = hq
                x1, y1, x2, y2 = best_face.bbox
                log.info(
                    "HQ avatar person=%s score=%.3f bbox=%sx%s t=%.2f",
                    person.person_id,
                    av_score,
                    max(0, x2 - x1),
                    max(0, y2 - y1),
                    best_face.timestamp_sec,
                )
        if thumb_bgr is not None:
            rel = f"thumbs/person_{person.person_id}.jpg"
            abs_path = thumbs / f"person_{person.person_id}.jpg"
            cv2.imwrite(
                str(abs_path),
                thumb_bgr,
                [int(cv2.IMWRITE_JPEG_QUALITY), 92],
            )
            thumb_path = rel
            person.thumbnail_bgr = thumb_bgr
            person.avatar_score = float(av_score) if av_score >= 0 else 0.0
        # Keep user-taught names if stabilize already set them; also prefer
        # non-generic labels already on the project for this person_id.
        label = person.label
        for old in project.people:
            if old.person_id == person.person_id and old.label and not old.label.startswith(
                "Person "
            ):
                label = old.label
                break
        cent = person.centroid
        cent_list = None
        # Interim JSON is polled often — skip fat centroid arrays on disk.
        # In-memory next_prev still carries centroids for stabilize.
        if (not interim) and cent is not None and getattr(cent, "size", 0):
            n = float(np.linalg.norm(cent))
            if n > 0:
                cent_list = (cent / n).astype(np.float64).tolist()
        rec = PersonRecord(
            person_id=person.person_id,
            label=label,
            detection_count=person.detection_count,
            clip_count=person.clip_estimate,
            screen_time_sec=person.screen_time_sec,
            first_sec=person.first_sec,
            last_sec=person.last_sec,
            thumbnail_path=thumb_path,
            centroid=cent_list,
        )
        records.append(rec)
        next_prev.append((person.centroid.copy(), rec))

    project.people = records
    project.segments = [
        SegmentRecord(
            person_id=s.person_id,
            start_sec=s.start_sec,
            end_sec=s.end_sec,
            preview_start_sec=s.preview_start_sec,
            preview_end_sec=s.preview_end_sec,
            face_time_sec=s.face_time_sec,
        )
        for s in segments
    ]
    if interim:
        project.status = "scanning"
        project.stage = f"Finding faces · {len(records)} characters"
    store.save(project)
    log.info(
        "Live cast published project=%s people=%s hits=%s interim=%s subset=%s",
        project.id,
        len(records),
        len(detections),
        interim,
        used_subset,
    )
    return next_prev


def _wait_if_paused(
    store: ProjectStore,
    project: ProjectState,
    should_cancel: CancelCb | None,
    should_pause: CancelCb | None,
) -> None:
    """Block while pause is asserted; cancel still wins."""
    if not should_pause or not should_pause():
        return
    project.status = "paused"
    project.stage = "Paused"
    project.eta_sec = None
    store.save(project)
    while should_pause():
        if store.is_deleted(project.id) or (should_cancel and should_cancel()):
            raise ScanCancelled(project.id)
        time.sleep(0.25)
    project.status = "scanning"
    project.stage = "Resuming…"
    store.save(project)


def run_scan(
    store: ProjectStore,
    project: ProjectState,
    on_progress: ProgressCb | None = None,
    should_cancel: CancelCb | None = None,
    should_pause: CancelCb | None = None,
) -> ProjectState:
    dbg = ScanDebugger(project.id)
    # Keep persisted shots; only clear in-memory cache then reload from project.
    clear_shot_cache(project.id)
    if not project.shots:
        run_scene_cuts_phase(
            store,
            project,
            should_cancel=should_cancel,
            force=True,
        )
    else:
        spans = _spans_from_project(project)
        if spans:
            _SHOT_CACHE[project.id] = spans
    file_handler: logging.Handler | None = None
    try:
        file_handler = logging.FileHandler(dbg.path, encoding="utf-8")
        file_handler.setFormatter(
            logging.Formatter("%(asctime)s | %(name)s | %(levelname)s | %(message)s")
        )
        file_handler.setLevel(logging.DEBUG)
        logging.getLogger("autoscene").addHandler(file_handler)
        logging.getLogger("autoscene").setLevel(logging.DEBUG)
    except Exception:  # noqa: BLE001
        file_handler = None

    last_persist = 0.0
    last_pct_bucket = -1

    def persist(force: bool = False) -> None:
        nonlocal last_persist, last_pct_bucket
        # Always publish memory so UI polls never need to open project.json.
        store.remember(project)
        if on_progress:
            on_progress(project.progress, project.stage)
        now = time.monotonic()
        bucket = int(project.progress * 100)
        # Disk coalesce (Windows AV / sharing locks hate rapid replace).
        if (
            not force
            and bucket == last_pct_bucket
            and (now - last_persist) < 2.5
        ):
            return
        last_persist = now
        last_pct_bucket = bucket
        store.save(project)

    def set_progress(
        *,
        overall: float,
        stage: str,
        phase: str,
        frames_done: int | None = None,
        frames_total: int | None = None,
        frames_approx: bool | None = None,
        detect_hits: int | None = None,
        eta_sec: float | None = None,
        clear_eta: bool = False,
        force: bool = False,
    ) -> None:
        if should_cancel and should_cancel():
            raise ScanCancelled(project.id)
        if store.is_deleted(project.id):
            raise ScanCancelled(project.id)
        # Honor pause between progress ticks (API sets status=paused)
        _wait_if_paused(store, project, should_cancel, should_pause)
        project.progress = max(0.0, min(1.0, overall))
        project.stage = stage
        project.progress_phase = phase
        if project.status != "paused":
            project.status = "scanning"
        if frames_done is not None:
            project.frames_done = frames_done
        if frames_total is not None:
            project.frames_total = frames_total
        if frames_approx is not None:
            project.frames_approx = frames_approx
        if detect_hits is not None:
            project.detect_hits = detect_hits
        if clear_eta:
            project.eta_sec = None
        elif eta_sec is not None:
            project.eta_sec = max(0.0, eta_sec)
        persist(force=force)

    video = Path(project.video_path)
    dbg.dump_project(project)
    dbg.dump_settings(label="runtime_settings_at_scan_start")
    dbg.dump_module_versions()
    dbg.info("scan_log=%s", dbg.path)

    if not video.exists():
        project.status = "error"
        project.error = f"Video missing: {video}"
        store.save(project)
        dbg.info("FATAL video missing path=%s", video)
        dbg.close()
        raise FileNotFoundError(project.error)

    set_progress(
        overall=0.0,
        stage="Checking video…",
        phase="probe",
        frames_done=0,
        frames_total=0,
        detect_hits=0,
        clear_eta=True,
        force=True,
    )
    info = probe_video(video)
    project.duration_sec = info.duration_sec
    samples_total, approx = expected_sample_count(
        info.frame_count, info.fps, settings.sample_fps
    )
    dbg.info(
        "probe w=%s h=%s fps=%.3f frames=%s duration=%.2fs sample_fps=%.2f "
        "samples_total=%s approx=%s step~%s",
        info.width,
        info.height,
        info.fps,
        info.frame_count,
        info.duration_sec,
        settings.sample_fps,
        samples_total,
        approx,
        max(1, int(round(max(0.1, info.fps) / max(0.1, settings.sample_fps)))),
    )
    set_progress(
        overall=PHASE_PROBE,
        stage="Video ready",
        phase="probe",
        frames_done=0,
        frames_total=samples_total,
        frames_approx=approx,
        force=True,
    )

    set_progress(
        overall=PHASE_PROBE,
        stage="Preparing face pack…",
        phase="model",
        force=True,
    )
    detect_max_edge = int(getattr(settings, "detect_max_edge", 1920) or 0)
    # Estimate detect-frame scale from probe so min_face_size stays native-correct
    probe_long = max(info.width, info.height)
    coord_scale_est = (
        probe_long / float(detect_max_edge)
        if detect_max_edge > 0 and probe_long > detect_max_edge
        else 1.0
    )
    detect_min_face = max(
        8, int(round(settings.min_face_size / max(coord_scale_est, 1e-6)))
    )
    dbg.info(
        "detect_geometry detect_max_edge=%s probe_long=%s coord_scale_est=%.4f "
        "native_min_face=%s detect_min_face=%s det_size=%s prefetch=%s "
        "face_select=%s relative_pct=%s top_n=%s",
        detect_max_edge,
        probe_long,
        coord_scale_est,
        settings.min_face_size,
        detect_min_face,
        settings.det_size,
        settings.prefetch_depth,
        settings.face_select_mode,
        settings.face_relative_pct,
        settings.face_top_n,
    )
    dbg.info(
        "create_backend request backend=%r model=%r",
        project.backend,
        project.model,
    )
    t_model0 = time.monotonic()
    try:
        backend = create_backend(
            backend=project.backend,
            model=project.model,
            det_size=settings.det_size,
            min_face_size=detect_min_face,
        )
        dbg.dump_backend(backend)
        # Sanity: requested backend must match constructed backend name
        want = (project.backend or "").strip().lower()
        got = (getattr(backend, "name", "") or "").strip().lower()
        if want in ("animecv", "anime") and got != "animecv":
            raise RuntimeError(
                f"Backend mismatch: project asked {project.backend!r} but got "
                f"{type(backend).__name__} name={got!r}. Restart AutoScene API "
                f"so AnimeCV code is loaded. Debug log: {dbg.path}"
            )
        if want in ("insightface", "") and got not in ("insightface", ""):
            dbg.info("NOTE backend name=%s for insightface request", got)
        set_progress(
            overall=PHASE_PROBE,
            stage="Preparing face pack…",
            phase="model",
            force=True,
        )
        dbg.info("backend.prepare() begin")
        backend.prepare()
        dbg.info(
            "backend.prepare() done in %.2fs class=%s model_id=%s",
            time.monotonic() - t_model0,
            type(backend).__name__,
            getattr(backend, "model_id", "?"),
        )
    except Exception as exc:
        dbg.exception("Model prepare failed: %s", exc)
        project.status = "error"
        project.error = "Could not prepare the face pack. Check Settings → Packs, then retry."
        project.stage = "Error"
        store.save(project)
        if file_handler:
            logging.getLogger("autoscene").removeHandler(file_handler)
            file_handler.close()
        dbg.close()
        raise
    set_progress(
        overall=PHASE_PROBE + PHASE_MODEL,
        stage="Face pack ready",
        phase="model",
        force=True,
    )

    detections: list[FaceDetection] = []
    seen = 0
    ema_fps: float | None = None
    # (monotonic_t, frames_done) samples for windowed rate
    rate_marks: list[tuple[float, int]] = []
    last_preview_at = 0.0
    last_live_cluster_at = 0.0
    last_live_cluster_hits = 0
    live_cluster_cooldown = LIVE_CLUSTER_EVERY_SEC
    last_heartbeat = 0.0
    prev_live: list[tuple[np.ndarray, PersonRecord]] = []
    detect_base = PHASE_PROBE + PHASE_MODEL
    detect_t0 = time.monotonic()
    # Clear previous live preview so UI does not show a stale face
    try:
        prev = store.preview_latest_path(project.id)
        if prev.exists():
            prev.unlink()
    except OSError:
        pass
    set_progress(
        overall=detect_base,
        stage="Finding faces…",
        phase="detect",
        frames_done=0,
        frames_total=samples_total,
        frames_approx=approx,
        detect_hits=0,
        force=True,
    )
    dbg.info(
        "detect_loop_start samples_total=%s backend=%s/%s detect_max_edge=%s",
        samples_total,
        getattr(backend, "name", "?"),
        getattr(backend, "model_id", "?"),
        detect_max_edge,
    )

    try:
      for ts, frame in iter_sampled_frames_prefetched(
          video,
          sample_fps=settings.sample_fps,
          queue_size=settings.prefetch_depth,
      ):
        if store.is_deleted(project.id) or (should_cancel and should_cancel()):
            if not store.is_deleted(project.id):
                project.status = "idle"
                project.stage = "Cancelled"
                project.error = None
                project.eta_sec = None
                store.save(project)
            raise ScanCancelled(project.id)

        _wait_if_paused(store, project, should_cancel, should_pause)

        t_frame0 = time.monotonic()
        detect_frame, coord_scale = downscale_for_detect(frame, detect_max_edge)
        detected = backend.detect(detect_frame, ts)
        n_raw = len(detected)
        mapped = _map_faces_to_source(
            detected,
            frame,
            coord_scale,
            min_face_size=settings.min_face_size,
        )
        faces = filter_faces_per_frame(
            mapped,
            mode=settings.face_select_mode,
            relative_pct=settings.face_relative_pct,
            top_n=settings.face_top_n,
        )
        detections.extend(faces)
        seen += 1
        frame_ms = (time.monotonic() - t_frame0) * 1000.0

        if faces:
            now_mono = time.monotonic()
            if now_mono - last_preview_at >= 0.45:
                best = max(faces, key=lambda f: f.det_score)
                try:
                    # Native crop from source frame (bboxes already in orig space)
                    _save_live_preview(store, project.id, best, frame)
                    last_preview_at = now_mono
                except Exception:  # noqa: BLE001
                    log.debug("Live preview write failed", exc_info=True)

        # Grow total if container under-reported frame_count
        if seen > samples_total:
            samples_total = seen
            approx = True

        frac = seen / max(1, samples_total)
        overall = detect_base + PHASE_DETECT * min(1.0, frac)

        # Windowed sample rate (recent ETA_WINDOW_SEC) + light EMA
        now_rate = time.monotonic()
        rate_marks.append((now_rate, seen))
        cutoff = now_rate - ETA_WINDOW_SEC
        while len(rate_marks) > 2 and rate_marks[0][0] < cutoff:
            rate_marks.pop(0)
        inst = 0.0
        if len(rate_marks) >= 2:
            t0, n0 = rate_marks[0]
            dt = now_rate - t0
            dn = seen - n0
            if dt >= 0.5 and dn > 0:
                inst = dn / dt
        if inst > 0:
            ema_fps = inst if ema_fps is None else (0.35 * inst + 0.65 * ema_fps)
        remaining = max(0, samples_total - seen)
        eta = (remaining / ema_fps) if ema_fps and ema_fps > 0 else None

        if (
            seen == 1
            or seen % 25 == 0
            or (now_rate - last_heartbeat) >= 10.0
        ):
            last_heartbeat = now_rate
            fh, fw = frame.shape[:2]
            dh, dw = detect_frame.shape[:2]
            dbg.info(
                "detect_tick seen=%s/%s hits_total=%s raw=%s mapped=%s kept=%s "
                "src=%sx%s det=%sx%s scale=%.3f frame_ms=%.1f "
                "ema_sps=%s eta_sec=%s ts=%.2f select=%s",
                seen,
                samples_total,
                len(detections),
                n_raw,
                len(mapped),
                len(faces),
                fw,
                fh,
                dw,
                dh,
                coord_scale,
                frame_ms,
                f"{ema_fps:.3f}" if ema_fps else "n/a",
                f"{eta:.0f}" if eta is not None else "n/a",
                ts,
                settings.face_select_mode,
            )

        set_progress(
            overall=overall,
            stage=f"Finding faces · {seen}/{samples_total}"
            + (" ~" if approx else ""),
            phase="detect",
            frames_done=seen,
            frames_total=samples_total,
            frames_approx=approx,
            detect_hits=len(detections),
            eta_sec=eta,
            clear_eta=eta is None,
        )

        hits = len(detections)
        now_live = time.monotonic()
        due_hits = hits - last_live_cluster_hits >= LIVE_CLUSTER_EVERY_HITS
        due_time = now_live - last_live_cluster_at >= live_cluster_cooldown
        if hits >= LIVE_CLUSTER_MIN_HITS and (due_hits or due_time):
            try:
                dbg.info(
                    "live_cluster begin hits=%s due_hits=%s due_time=%s cooldown=%.1f",
                    hits,
                    due_hits,
                    due_time,
                    live_cluster_cooldown,
                )
                t_cluster0 = time.monotonic()
                prev_live = _publish_live_cast(
                    store,
                    project,
                    detections,
                    prev_live=prev_live,
                    interim=True,
                )
                # Stamp AFTER cluster finishes — stamping the start time made a
                # 45s cluster look immediately due again on the next frame.
                last_live_cluster_at = time.monotonic()
                last_live_cluster_hits = hits
                cluster_sec = last_live_cluster_at - t_cluster0
                # Back off if clustering is expensive (large hit lists).
                live_cluster_cooldown = max(
                    LIVE_CLUSTER_EVERY_SEC,
                    min(180.0, cluster_sec * 2.5),
                )
                dbg.info(
                    "live_cluster done people=%s took=%.1fs next_cooldown=%.1f",
                    len(project.people),
                    cluster_sec,
                    live_cluster_cooldown,
                )
            except Exception:  # noqa: BLE001
                log.exception("Live cast publish failed project=%s", project.id)
                dbg.exception("live_cluster failed")
                last_live_cluster_at = time.monotonic()
                last_live_cluster_hits = hits
                live_cluster_cooldown = max(live_cluster_cooldown, 90.0)
    except ScanCancelled:
        dbg.info(
            "scan cancelled after seen=%s hits=%s elapsed=%.1fs",
            seen,
            len(detections),
            time.monotonic() - detect_t0,
        )
        if file_handler:
            logging.getLogger("autoscene").removeHandler(file_handler)
            file_handler.close()
        dbg.close()
        raise
    except Exception as exc:
        dbg.exception("detect loop failed: %s", exc)
        if file_handler:
            logging.getLogger("autoscene").removeHandler(file_handler)
            file_handler.close()
        dbg.close()
        raise

    dbg.info(
        "detect_loop_done seen=%s hits=%s elapsed=%.1fs avg_sps=%.3f",
        seen,
        len(detections),
        time.monotonic() - detect_t0,
        seen / max(1e-3, time.monotonic() - detect_t0),
    )

    def _dbg_cleanup() -> None:
        if file_handler:
            try:
                logging.getLogger("autoscene").removeHandler(file_handler)
                file_handler.close()
            except Exception:  # noqa: BLE001
                pass
        dbg.close()

    if store.is_deleted(project.id) or (should_cancel and should_cancel()):
        if not store.is_deleted(project.id):
            project.status = "idle"
            project.stage = "Cancelled"
            project.error = None
            project.eta_sec = None
            store.save(project)
        dbg.info("cancelled after detect loop")
        _dbg_cleanup()
        raise ScanCancelled(project.id)

    # Snap detect phase complete with exact count
    samples_total = max(samples_total, seen)
    set_progress(
        overall=detect_base + PHASE_DETECT,
        stage=f"Faces found · {seen} checked · {len(detections)} faces",
        phase="detect",
        frames_done=seen,
        frames_total=samples_total,
        frames_approx=False,
        detect_hits=len(detections),
        clear_eta=True,
        force=True,
    )

    set_progress(
        overall=detect_base + PHASE_DETECT,
        stage="Grouping characters…",
        phase="cluster",
        clear_eta=True,
        force=True,
    )
    dbg.info(
        "cluster begin hits=%s merge_sim=%.3f min_size=%s",
        len(detections),
        settings.cluster_merge_sim,
        settings.cluster_min_size,
    )
    if store.is_deleted(project.id) or (should_cancel and should_cancel()):
        _dbg_cleanup()
        raise ScanCancelled(project.id)

    set_progress(
        overall=detect_base + PHASE_DETECT + PHASE_CLUSTER * 0.4,
        stage="Saving portraits…",
        phase="thumbs",
        force=True,
    )
    try:
        prev_live = _publish_live_cast(
            store,
            project,
            detections,
            prev_live=prev_live,
            interim=False,
        )
    except Exception as exc:
        dbg.exception("final cluster failed: %s", exc)
        _dbg_cleanup()
        raise
    dbg.info(
        "cluster done people=%s segments=%s",
        len(project.people),
        len(project.segments),
    )
    set_progress(
        overall=detect_base + PHASE_DETECT + PHASE_CLUSTER + PHASE_THUMBS,
        stage=f"Ready · {len(project.people)} characters",
        phase="thumbs",
        force=True,
    )

    project.excluded_segments = []
    project.status = "ready"
    project.progress = 1.0
    project.stage = "Ready"
    project.progress_phase = "done"
    project.frames_done = seen
    project.frames_total = samples_total
    project.frames_approx = False
    project.detect_hits = len(detections)
    project.eta_sec = None
    project.error = None
    from autoscene.identity_gates import IDENTITY_PIPELINE_VERSION

    project.identity_pipeline_version = IDENTITY_PIPELINE_VERSION
    project.needs_rescan = False
    store.save(project)
    log.info(
        "Scan done project=%s people=%s segments=%s detections=%s frames=%s",
        project.id,
        len(project.people),
        len(project.segments),
        len(detections),
        seen,
    )
    dbg.info(
        "SCAN_OK people=%s segments=%s detections=%s frames=%s log=%s",
        len(project.people),
        len(project.segments),
        len(detections),
        seen,
        dbg.path,
    )
    _dbg_cleanup()
    return project
