"""Manual cast edits (merge identities, etc.)."""

from __future__ import annotations

import numpy as np

from autoscene.project import PersonRecord, ProjectState, SegmentExclusion, SegmentRecord


def _merge_centroids(
    keep: PersonRecord,
    others: list[PersonRecord],
) -> list[float] | None:
    vecs: list[np.ndarray] = []
    weights: list[float] = []
    for person in [keep, *others]:
        if not person.centroid:
            continue
        v = np.asarray(person.centroid, dtype=np.float64)
        n = float(np.linalg.norm(v))
        if n <= 0:
            continue
        vecs.append(v / n)
        weights.append(max(1.0, float(person.detection_count)))
    if not vecs:
        return keep.centroid
    w = np.asarray(weights, dtype=np.float64)
    w = w / w.sum()
    blend = sum(vi * wi for vi, wi in zip(vecs, w))
    n = float(np.linalg.norm(blend))
    if n <= 0:
        return keep.centroid
    return (blend / n).tolist()


def _coalesce_segments(segs: list[SegmentRecord]) -> list[SegmentRecord]:
    """
    Merge truly overlapping segments for one person (timeline order).
    Only real overlap merges — no gap bridging, which used to glue windows
    across scene cuts into clips with faceless middles. Face windows and
    thumbnail anchors survive the merge.
    """
    if not segs:
        return []
    ordered = sorted(segs, key=lambda s: (s.start_sec, s.end_sec))

    def _copy(s: SegmentRecord) -> SegmentRecord:
        return SegmentRecord(
            person_id=s.person_id,
            start_sec=s.start_sec,
            end_sec=s.end_sec,
            preview_start_sec=s.preview_start_sec,
            preview_end_sec=s.preview_end_sec,
            face_time_sec=s.face_time_sec,
        )

    out: list[SegmentRecord] = []
    cur = _copy(ordered[0])
    for s in ordered[1:]:
        if s.start_sec <= cur.end_sec + 1e-3:
            cur.end_sec = max(cur.end_sec, s.end_sec)
            if s.preview_start_sec is not None and cur.preview_start_sec is not None:
                cur.preview_start_sec = min(cur.preview_start_sec, s.preview_start_sec)
            if s.preview_end_sec is not None and cur.preview_end_sec is not None:
                cur.preview_end_sec = max(cur.preview_end_sec, s.preview_end_sec)
            if cur.face_time_sec is None:
                cur.face_time_sec = s.face_time_sec
        else:
            out.append(cur)
            cur = _copy(s)
    out.append(cur)
    return out


def merge_people(
    project: ProjectState,
    keep_person_id: int,
    merge_person_ids: list[int],
) -> ProjectState:
    """
    Absorb merge_person_ids into keep_person_id.
    Segments remapped + coalesced in chronological order. Exclusions remapped.
    """
    absorb = sorted({int(i) for i in merge_person_ids if int(i) != int(keep_person_id)})
    if not absorb:
        raise ValueError("No other characters to merge")

    by_id = {p.person_id: p for p in project.people}
    if keep_person_id not in by_id:
        raise KeyError(f"Keep person {keep_person_id} not found")
    missing = [i for i in absorb if i not in by_id]
    if missing:
        raise KeyError(f"Merge person(s) not found: {missing}")

    keep = by_id[keep_person_id]
    others = [by_id[i] for i in absorb]

    # Remap segments (keep face windows + thumbnail anchors)
    remapped: list[SegmentRecord] = []
    for s in project.segments:
        if s.person_id == keep_person_id or s.person_id in absorb:
            remapped.append(
                SegmentRecord(
                    person_id=keep_person_id,
                    start_sec=s.start_sec,
                    end_sec=s.end_sec,
                    preview_start_sec=s.preview_start_sec,
                    preview_end_sec=s.preview_end_sec,
                    face_time_sec=s.face_time_sec,
                )
            )
        else:
            remapped.append(s)

    keep_segs = _coalesce_segments(
        [s for s in remapped if s.person_id == keep_person_id]
    )
    other_segs = [s for s in remapped if s.person_id != keep_person_id]
    project.segments = sorted(
        other_segs + keep_segs,
        key=lambda s: (s.person_id, s.start_sec, s.end_sec),
    )

    # Remap exclusions (drop duplicates after remap)
    seen_ex: set[tuple[int, float, float]] = set()
    new_ex: list[SegmentExclusion] = []
    absorb_set = set(absorb) | {keep_person_id}
    for ex in project.excluded_segments:
        pid = keep_person_id if ex.person_id in absorb_set else ex.person_id
        key = (pid, round(ex.start_sec, 4), round(ex.end_sec, 4))
        if key in seen_ex:
            continue
        seen_ex.add(key)
        new_ex.append(
            SegmentExclusion(person_id=pid, start_sec=ex.start_sec, end_sec=ex.end_sec)
        )
    project.excluded_segments = new_ex

    # Survivor stats
    det = keep.detection_count + sum(p.detection_count for p in others)
    first = min([keep.first_sec, *[p.first_sec for p in others]]) if keep_segs else keep.first_sec
    last = max([keep.last_sec, *[p.last_sec for p in others]]) if keep_segs else keep.last_sec
    if keep_segs:
        first = keep_segs[0].start_sec
        last = keep_segs[-1].end_sec
    screen = sum(max(0.0, s.end_sec - s.start_sec) for s in keep_segs)

    # Prefer sharper / better avatar among merge candidates
    import cv2

    from autoscene.config import settings
    from autoscene.faces.quality import avatar_score_from_bgr

    root = (settings.projects_dir or settings.data_dir / "projects") / project.id
    best_path = keep.thumbnail_path
    best_score = -1.0
    anime_soft = (project.backend or "").lower() == "animecv"
    for person in [keep, *others]:
        if not person.thumbnail_path:
            continue
        path = root / person.thumbnail_path
        img = cv2.imread(str(path)) if path.is_file() else None
        score = avatar_score_from_bgr(img, anime_soft=anime_soft)
        if score > best_score:
            best_score = score
            best_path = person.thumbnail_path
    thumb = best_path or keep.thumbnail_path

    project.people = [
        PersonRecord(
            person_id=keep_person_id,
            label=keep.label,
            detection_count=det,
            clip_count=len(keep_segs),
            screen_time_sec=screen,
            first_sec=first,
            last_sec=last,
            thumbnail_path=thumb,
            centroid=_merge_centroids(keep, others),
        ),
        *[p for p in project.people if p.person_id not in absorb and p.person_id != keep_person_id],
    ]
    project.people.sort(key=lambda p: (-p.detection_count, p.first_sec, p.person_id))

    if project.selected_person_id in absorb:
        project.selected_person_id = keep_person_id

    return project
