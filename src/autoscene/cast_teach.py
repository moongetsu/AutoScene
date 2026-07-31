"""Teach cast from reference stills (same face backend as the project scan)."""

from __future__ import annotations

import logging
import re
from dataclasses import dataclass
from pathlib import Path
from typing import Any

import cv2
import numpy as np

from autoscene.config import settings
from autoscene.faces import create_backend
from autoscene.faces.base import FaceBackend
from autoscene.project import PersonRecord, ProjectState, ProjectStore

log = logging.getLogger(__name__)


@dataclass
class TeachMatch:
    label: str
    person_id: int | None
    similarity: float
    status: str  # linked | unmatched | pinned
    filename: str | None = None


def _unit(v: np.ndarray) -> np.ndarray:
    v = np.asarray(v, dtype=np.float64).reshape(-1)
    n = float(np.linalg.norm(v))
    if n <= 0:
        return v
    return v / n


def _label_from_filename(name: str) -> str:
    stem = Path(name).stem
    stem = re.sub(r"[_\-]+", " ", stem).strip()
    stem = re.sub(r"\s+", " ", stem)
    return stem or "Character"


def person_centroid_array(person: PersonRecord) -> np.ndarray | None:
    if not person.centroid:
        return None
    try:
        return _unit(np.asarray(person.centroid, dtype=np.float64))
    except Exception:  # noqa: BLE001
        return None


def decode_image_bytes(data: bytes) -> np.ndarray:
    arr = np.frombuffer(data, dtype=np.uint8)
    bgr = cv2.imdecode(arr, cv2.IMREAD_COLOR)
    if bgr is None:
        raise ValueError("Could not decode image")
    return bgr


def embed_faces(backend: FaceBackend, frame_bgr: np.ndarray) -> list[np.ndarray]:
    faces = backend.detect(frame_bgr, timestamp_sec=0.0)
    if not faces:
        return []
    # Prefer largest face in stills (cast reference photos)
    faces = sorted(
        faces,
        key=lambda f: max(0, f.bbox[2] - f.bbox[0]) * max(0, f.bbox[3] - f.bbox[1]),
        reverse=True,
    )
    return [_unit(f.embedding) for f in faces[:3]]


def mean_embedding(embs: list[np.ndarray]) -> np.ndarray | None:
    if not embs:
        return None
    m = np.mean(np.stack(embs, axis=0), axis=0)
    return _unit(m)


def thumb_embedding(
    backend: FaceBackend,
    store: ProjectStore,
    project: ProjectState,
    person: PersonRecord,
) -> np.ndarray | None:
    if not person.thumbnail_path:
        return None
    path = store._dir(project.id) / person.thumbnail_path
    if not path.is_file():
        return None
    bgr = cv2.imread(str(path), cv2.IMREAD_COLOR)
    if bgr is None:
        return None
    # Thumbs are already face crops — detect may fail; encode via detect on padded canvas
    h, w = bgr.shape[:2]
    pad = max(8, min(h, w) // 4)
    canvas = np.zeros((h + pad * 2, w + pad * 2, 3), dtype=np.uint8)
    canvas[pad : pad + h, pad : pad + w] = bgr
    embs = embed_faces(backend, canvas)
    if embs:
        return embs[0]
    # Fallback: treat whole thumb as face by forcing a synthetic detection path
    # AnimeCV/InsightFace need detector; if empty, skip
    return None


def resolve_person_embedding(
    backend: FaceBackend,
    store: ProjectStore,
    project: ProjectState,
    person: PersonRecord,
) -> np.ndarray | None:
    cent = person_centroid_array(person)
    if cent is not None and cent.size:
        return cent
    return thumb_embedding(backend, store, project, person)


def best_person_match(
    emb: np.ndarray,
    people_embs: list[tuple[int, np.ndarray]],
    min_sim: float,
    *,
    margin: float = 0.05,
) -> tuple[int | None, float]:
    """Best match above floor AND clearly ahead of the runner-up.
    A reference photo that two cast members both 'kind of' match is
    unmatched — linking a name to the wrong cluster poisons everything."""
    best_id: int | None = None
    best_sim = -1.0
    second_sim = -1.0
    for pid, cent in people_embs:
        sim = float(np.dot(emb, cent))
        if sim > best_sim:
            second_sim = best_sim
            best_sim = sim
            best_id = pid
        elif sim > second_sim:
            second_sim = sim
    if best_id is None or best_sim < float(min_sim):
        return None, max(0.0, best_sim)
    if second_sim > 0 and (best_sim - second_sim) < margin:
        return None, best_sim
    return best_id, best_sim


def _blend_centroid(
    existing: list[float] | None,
    new_emb: np.ndarray,
    *,
    existing_weight: float = 0.65,
) -> list[float]:
    neu = _unit(new_emb)
    if not existing:
        return neu.tolist()
    old = _unit(np.asarray(existing, dtype=np.float64))
    if old.size != neu.size:
        return neu.tolist()
    w = max(0.0, min(1.0, existing_weight))
    blended = _unit(old * w + neu * (1.0 - w))
    return blended.tolist()


def _save_ref_files(
    store: ProjectStore,
    project_id: str,
    person_id: int,
    items: list[tuple[str, bytes]],
) -> None:
    root = store._dir(project_id) / "refs" / f"person_{person_id}"
    root.mkdir(parents=True, exist_ok=True)
    for name, data in items:
        safe = re.sub(r"[^\w.\-]+", "_", Path(name).name)[:80] or "ref.jpg"
        (root / safe).write_bytes(data)


def teach_cast(
    store: ProjectStore,
    project: ProjectState,
    *,
    images: list[tuple[str, bytes]],
    mode: str = "match",
    label: str | None = None,
    person_id: int | None = None,
) -> tuple[ProjectState, list[TeachMatch]]:
    """
    mode:
      - pin: attach refs to person_id; set label if provided
      - match: average refs under label → best person
      - pack: each file → label from filename → best person
    """
    if not images:
        raise ValueError("No reference images")
    if project.status not in ("ready", "scanning", "paused"):
        raise ValueError("Cast teach needs a scanned (or scanning) project")
    if not project.people:
        raise ValueError("No cast yet — wait for characters to appear")

    backend = create_backend(
        backend=project.backend,
        model=project.model,
        det_size=settings.det_size,
        min_face_size=settings.min_face_size,
    )
    backend.prepare()

    from autoscene.identity_gates import gate_config_for

    cfg = gate_config_for(settings.content_preset, project.backend)
    # Reference stills must clear the same assign floor as scan (no soft teach).
    min_sim = float(cfg.assign_min_sim)
    match_margin = float(cfg.assign_margin)
    people_embs: list[tuple[int, np.ndarray]] = []
    for person in project.people:
        emb = resolve_person_embedding(backend, store, project, person)
        if emb is not None:
            people_embs.append((person.person_id, emb))

    if not people_embs:
        raise ValueError(
            "Could not read cast embeddings. Re-scan once so centroids are saved, "
            "or ensure character thumbnails exist."
        )

    by_id = {p.person_id: p for p in project.people}
    matches: list[TeachMatch] = []
    mode = (mode or "match").strip().lower()

    def apply_to_person(
        pid: int,
        new_label: str | None,
        ref_emb: np.ndarray,
        ref_files: list[tuple[str, bytes]],
        status: str,
        filename: str | None,
        sim: float,
    ) -> None:
        person = by_id[pid]
        if new_label:
            person.label = new_label.strip() or person.label
        person.centroid = _blend_centroid(person.centroid, ref_emb)
        _save_ref_files(store, project.id, pid, ref_files)
        matches.append(
            TeachMatch(
                label=person.label,
                person_id=pid,
                similarity=sim,
                status=status,
                filename=filename,
            )
        )

    if mode == "pin":
        if person_id is None or person_id not in by_id:
            raise ValueError("pin mode requires a valid person_id")
        embs: list[np.ndarray] = []
        kept_files: list[tuple[str, bytes]] = []
        for fname, data in images:
            bgr = decode_image_bytes(data)
            found = embed_faces(backend, bgr)
            if not found:
                matches.append(
                    TeachMatch(
                        label=label or by_id[person_id].label,
                        person_id=None,
                        similarity=0.0,
                        status="unmatched",
                        filename=fname,
                    )
                )
                continue
            embs.append(found[0])
            kept_files.append((fname, data))
        mean = mean_embedding(embs)
        if mean is None:
            raise ValueError("No face found in reference image(s)")
        existing = resolve_person_embedding(backend, store, project, by_id[person_id])
        sim = float(np.dot(mean, existing)) if existing is not None else 1.0
        apply_to_person(
            person_id,
            label,
            mean,
            kept_files,
            "pinned",
            kept_files[0][0] if kept_files else None,
            sim,
        )

    elif mode == "pack":
        for fname, data in images:
            bgr = decode_image_bytes(data)
            found = embed_faces(backend, bgr)
            lab = _label_from_filename(fname)
            if not found:
                matches.append(
                    TeachMatch(
                        label=lab,
                        person_id=None,
                        similarity=0.0,
                        status="unmatched",
                        filename=fname,
                    )
                )
                continue
            pid, sim = best_person_match(
                found[0], people_embs, min_sim, margin=match_margin
            )
            if pid is None:
                matches.append(
                    TeachMatch(
                        label=lab,
                        person_id=None,
                        similarity=sim,
                        status="unmatched",
                        filename=fname,
                    )
                )
                continue
            apply_to_person(pid, lab, found[0], [(fname, data)], "linked", fname, sim)
            # Refresh that person's emb for subsequent files
            people_embs = [
                (p, e if p != pid else _unit(np.asarray(by_id[pid].centroid)))
                for p, e in people_embs
            ]

    else:  # match
        lab = (label or "").strip()
        if not lab:
            raise ValueError("match mode requires a label")
        embs = []
        kept_files = []
        for fname, data in images:
            bgr = decode_image_bytes(data)
            found = embed_faces(backend, bgr)
            if not found:
                matches.append(
                    TeachMatch(
                        label=lab,
                        person_id=None,
                        similarity=0.0,
                        status="unmatched",
                        filename=fname,
                    )
                )
                continue
            embs.append(found[0])
            kept_files.append((fname, data))
        mean = mean_embedding(embs)
        if mean is None:
            raise ValueError("No face found in reference image(s)")
        pid, sim = best_person_match(
            mean, people_embs, min_sim, margin=match_margin
        )
        if pid is None:
            matches.append(
                TeachMatch(
                    label=lab,
                    person_id=None,
                    similarity=sim,
                    status="unmatched",
                    filename=kept_files[0][0] if kept_files else None,
                )
            )
        else:
            apply_to_person(
                pid,
                lab,
                mean,
                kept_files,
                "linked",
                kept_files[0][0] if kept_files else None,
                sim,
            )

    project.people.sort(key=lambda p: (-p.detection_count, p.first_sec, p.person_id))
    _all_in_retrain(store, project, matches)
    return project, matches


def _all_in_retrain(
    store: ProjectStore,
    project: ProjectState,
    matches: list[TeachMatch],
) -> None:
    """Train identity adapter + rebuild segments from detection cache (all-in)."""
    from autoscene.cluster import build_segments
    from autoscene.identity_adapter import (
        IdentityAdapter,
        adapter_path,
        load_detection_cache,
        reassign_labels_with_adapter,
        save_detection_cache,
    )
    from autoscene.pipeline import _shots_for_project, clear_shot_cache
    from autoscene.project import SegmentRecord

    linked = [m for m in matches if m.person_id is not None and m.status in ("linked", "pinned")]
    if not linked:
        return

    cache = load_detection_cache(store._dir(project.id))
    if cache is None:
        log.info("teach: no detection cache — centroid update only (re-scan for full rebuild)")
        return

    timestamps = cache["timestamps"]
    embeddings = cache["embeddings"]
    old_labels = cache["labels"]
    cache_bboxes = cache.get("bboxes")
    cache_scores = cache.get("det_scores")
    if embeddings.ndim != 2 or embeddings.shape[0] == 0:
        return

    from autoscene.identity_gates import gate_config_for

    cfg = gate_config_for(settings.content_preset, project.backend)

    dim = int(embeddings.shape[1])
    positives: dict[int, list[np.ndarray]] = {}
    for person in project.people:
        cent = person_centroid_array(person)
        if cent is not None and cent.size == dim:
            positives.setdefault(person.person_id, []).append(cent)
        # Seed from current label assignments in cache
        idxs = np.where(old_labels == person.person_id)[0]
        if idxs.size:
            # Cap samples per person for speed
            pick = idxs if idxs.size <= 64 else idxs[:: max(1, idxs.size // 64)]
            for i in pick:
                positives.setdefault(person.person_id, []).append(
                    np.asarray(embeddings[int(i)], dtype=np.float64)
                )

    adapter = IdentityAdapter(dim)
    result = adapter.train(positives, steps=100, lr=0.08)
    log.info("identity adapter train: %s", result)
    use_adapter = False
    if result.get("trained"):
        collapsed, max_sim = adapter.training_collapsed_distinct_people(
            positives, cfg.assign_min_sim
        )
        if collapsed:
            log.warning(
                "identity adapter discarded: teach training collapsed distinct "
                "people (max pairwise sim=%.3f, floor=%.3f)",
                max_sim,
                cfg.assign_min_sim,
            )
        else:
            adapter.save(adapter_path(store._dir(project.id)))
            use_adapter = True

    centroids: dict[int, np.ndarray] = {}
    for person in project.people:
        cent = person_centroid_array(person)
        if cent is not None and cent.size == dim:
            centroids[person.person_id] = cent

    clear_shot_cache(project.id)
    shots = _shots_for_project(project, allow_detect=False)
    if shots is None:
        # Teach without prior cuts — refresh once
        clear_shot_cache(project.id)
        from autoscene.pipeline import run_scene_cuts_phase

        run_scene_cuts_phase(store, project, force=True)
        shots = _shots_for_project(project, allow_detect=False)

    # Full identity gates on the rebuild — the old path reassigned the whole
    # cache at merge_sim-0.08 (≈0.42) with no margin, undoing scan purity.
    new_labels = reassign_labels_with_adapter(
        embeddings,
        centroids,
        adapter if use_adapter else None,
        cfg=cfg,
        timestamps=timestamps,
        bboxes=cache_bboxes,
        det_scores=cache_scores,
        shots=shots,
    )

    # FaceDetection shells for build_segments; keep real bboxes/scores so
    # face_time thumbs and any geometry-aware path stay honest.
    from autoscene.faces.base import FaceDetection

    dets: list[FaceDetection] = []
    for i in range(len(timestamps)):
        if cache_bboxes is not None and i < len(cache_bboxes):
            bb = cache_bboxes[i]
            bbox = (int(bb[0]), int(bb[1]), int(bb[2]), int(bb[3]))
        else:
            bbox = (0, 0, 1, 1)
        score = (
            float(cache_scores[i])
            if cache_scores is not None and i < len(cache_scores)
            else 1.0
        )
        dets.append(
            FaceDetection(
                timestamp_sec=float(timestamps[i]),
                bbox=bbox,
                embedding=np.asarray(embeddings[i], dtype=np.float64),
                det_score=score,
            )
        )

    min_hits = 2
    segments = build_segments(
        dets,
        new_labels,
        gap_sec=settings.segment_gap_sec,
        min_segment_sec=settings.min_segment_sec,
        shots=shots,
        min_hits_per_shot=min_hits,
    )

    # Update detection counts / screen times lightly
    by_pid: dict[int, int] = {}
    for lb in new_labels.tolist():
        if lb >= 0:
            by_pid[int(lb)] = by_pid.get(int(lb), 0) + 1
    for person in project.people:
        person.detection_count = int(by_pid.get(person.person_id, 0))
        segs = [s for s in segments if s.person_id == person.person_id]
        person.clip_count = len(segs)
        if segs:
            person.first_sec = float(min(s.start_sec for s in segs))
            person.last_sec = float(max(s.end_sec for s in segs))
            person.screen_time_sec = float(
                sum(max(0.0, s.end_sec - s.start_sec) for s in segs)
            )

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
    # Drop exclusions that no longer match any segment key
    keep_keys = {
        (s.person_id, round(s.start_sec, 4), round(s.end_sec, 4))
        for s in project.segments
    }
    project.excluded_segments = [
        ex
        for ex in project.excluded_segments
        if (ex.person_id, round(ex.start_sec, 4), round(ex.end_sec, 4)) in keep_keys
    ]

    try:
        save_detection_cache(
            store._dir(project.id),
            timestamps=timestamps,
            embeddings=embeddings.astype(np.float32),
            labels=new_labels.astype(np.int32),
            bboxes=None if cache_bboxes is None else cache_bboxes.astype(np.float32),
            det_scores=(
                None if cache_scores is None else cache_scores.astype(np.float32)
            ),
        )
    except Exception as exc:  # noqa: BLE001
        log.warning("detection cache rewrite failed: %s", exc)


def matches_to_dict(matches: list[TeachMatch]) -> list[dict[str, Any]]:
    return [
        {
            "label": m.label,
            "person_id": m.person_id,
            "similarity": round(float(m.similarity), 4),
            "status": m.status,
            "filename": m.filename,
        }
        for m in matches
    ]
