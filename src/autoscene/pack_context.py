"""Widen detected shot plates into context-preserving pack clips.

A detected segment marks one shot where a character's face was found. That is
not how ScenePacks are cut by hand: creators keep the whole exchange, including
the reverse shots of whoever the character is talking to, and the establishing
shots that lead up to an entrance.

This runs at READ time, not during the scan. pipeline._publish_live_cast
rewrites project.segments wholesale on every publish, so expansion baked in
there would be destroyed by a rescan — and changing context mode would require
one. Keeping project.segments as the raw detection record means switching modes
is instant, and the co-cast graph in ProjectState._derived keeps deriving from
real face windows rather than widened plates.
"""

from __future__ import annotations

import logging

from autoscene.cluster import Segment, coalesce_segments

log = logging.getLogger(__name__)

# Cap runaway growth in conversation mode: a two-hander that never cuts away
# could otherwise walk an entire act.
DEFAULT_MAX_CLIP_SEC = 180.0


def _shot_bounds(shots: list, index: int) -> tuple[float, float] | None:
    if 0 <= index < len(shots):
        sh = shots[index]
        return float(sh.start_sec), float(sh.end_sec)
    return None


def _shot_of_segment(shot_starts: list[float], seg) -> int:
    """Index of the shot a segment plate sits on; -1 when it matches none."""
    target = float(seg.start_sec)
    best, best_d = -1, 0.06
    for i, s in enumerate(shot_starts):
        d = abs(s - target)
        if d < best_d:
            best, best_d = i, d
    return best


def _cast_by_shot(shots: list, segments: list) -> list[set[int]]:
    from autoscene.scene_groups import cast_by_shot

    return cast_by_shot(shots, segments)


def _scene_of_shot(scene_groups: list) -> dict[int, int]:
    out: dict[int, int] = {}
    for g in scene_groups:
        for si in g.shot_indices:
            out[int(si)] = int(g.scene_index)
    return out


def _partners_of(person_id: int, cocast_edges: list) -> set[int]:
    """Cast this character actually shares frames with."""
    partners: set[int] = set()
    for edge in cocast_edges or []:
        a, b = int(edge["a"]), int(edge["b"])
        if a == person_id:
            partners.add(b)
        elif b == person_id:
            partners.add(a)
    return partners


def _grow_conversation(
    seed: int,
    *,
    cast: list[set[int]],
    person_id: int,
    partners: set[int],
    scene_of: dict[int, int],
) -> tuple[int, int]:
    """
    Walk out from a seed shot across the exchange the character is part of.

    Keeps going while the next shot holds the character or one of the people
    they share frames with — which is what carries the reverse shots of a
    conversation into the clip. Stops at a scene boundary so an exchange cannot
    bleed into the next location.
    """
    n = len(cast)
    scene = scene_of.get(seed, -1)
    keep = {person_id} | partners

    lo = seed
    while lo - 1 >= 0:
        prev = lo - 1
        if scene_of.get(prev, -1) != scene:
            break
        if not (cast[prev] & keep):
            break
        lo = prev

    hi = seed
    while hi + 1 < n:
        nxt = hi + 1
        if scene_of.get(nxt, -1) != scene:
            break
        if not (cast[nxt] & keep):
            break
        hi = nxt

    return lo, hi


def subtract_shots(
    clips: list[Segment],
    shots: list,
    excluded_shot_indices: set[int],
) -> list[Segment]:
    """
    Carve excluded shots out of clips, splitting where they fall in the middle.

    A widened clip can span a dozen shots, so dropping the whole clip because
    the user switched one of them off would throw away everything around it.
    Excluding the opening shot of a 0-18s scene clip leaves 6-18s, not nothing.
    """
    if not excluded_shot_indices or not clips:
        return clips

    cuts = sorted(
        (float(shots[i].start_sec), float(shots[i].end_sec))
        for i in excluded_shot_indices
        if 0 <= i < len(shots)
    )
    if not cuts:
        return clips

    out: list[Segment] = []
    for clip in clips:
        pieces = [(float(clip.start_sec), float(clip.end_sec))]
        for cut_a, cut_b in cuts:
            nxt: list[tuple[float, float]] = []
            for a, b in pieces:
                if cut_b <= a or cut_a >= b:
                    nxt.append((a, b))
                    continue
                if a < cut_a:
                    nxt.append((a, cut_a))
                if cut_b < b:
                    nxt.append((cut_b, b))
            pieces = nxt
        for a, b in pieces:
            if b - a < 0.15:
                # Slivers left between two excluded shots are not a clip.
                continue
            ft = clip.face_time_sec
            out.append(
                Segment(
                    clip.person_id,
                    a,
                    b,
                    preview_start_sec=a,
                    preview_end_sec=b,
                    face_time_sec=(ft if ft is not None and a <= ft <= b else None),
                )
            )
    return out


def expand_segments(
    segments: list,
    *,
    person_id: int,
    shots: list,
    scene_groups: list,
    cocast_edges: list,
    all_segments: list,
    mode: str = "shot",
    lead_shots: int = 0,
    tail_shots: int = 0,
    max_clip_sec: float | None = None,
    extra_shot_indices: set[int] | None = None,
    excluded_shot_indices: set[int] | None = None,
) -> list[Segment]:
    """
    Turn this character's detected plates into pack clips at the chosen context.

    `segments` are the character's own plates; `all_segments` is every
    character's, needed to know who else is in each shot.
    """
    if not segments and not extra_shot_indices:
        return []

    if mode == "face" and not extra_shot_indices:
        out = [
            Segment(
                person_id,
                float(s.preview_start_sec if s.preview_start_sec is not None else s.start_sec),
                float(s.preview_end_sec if s.preview_end_sec is not None else s.end_sec),
                preview_start_sec=s.preview_start_sec,
                preview_end_sec=s.preview_end_sec,
                face_time_sec=getattr(s, "face_time_sec", None),
            )
            for s in segments
        ]
        return coalesce_segments(out, gap_sec=0.0, same_plate_only=True)

    if not shots:
        # Nothing to widen against; hand back the plates untouched.
        return coalesce_segments(
            [
                Segment(
                    person_id,
                    float(s.start_sec),
                    float(s.end_sec),
                    preview_start_sec=s.preview_start_sec,
                    preview_end_sec=s.preview_end_sec,
                    face_time_sec=getattr(s, "face_time_sec", None),
                )
                for s in segments
            ],
            gap_sec=0.0,
            same_plate_only=True,
        )

    shot_starts = [float(sh.start_sec) for sh in shots]
    dropped = {int(i) for i in (excluded_shot_indices or set())}
    seeds = {i for i in (_shot_of_segment(shot_starts, s) for s in segments) if i >= 0}
    seeds |= {i for i in (extra_shot_indices or set()) if 0 <= i < len(shots)}
    # An excluded shot must not anchor a clip; it can still be swept up as
    # context for a neighbour, which subtract_shots then carves back out.
    seeds -= dropped
    if not seeds:
        return []

    cast = _cast_by_shot(shots, all_segments)
    scene_of = _scene_of_shot(scene_groups)
    partners = _partners_of(person_id, cocast_edges)
    cap = float(max_clip_sec) if max_clip_sec else DEFAULT_MAX_CLIP_SEC

    # face_time anchors the still shown on a clip tile; keep the real detection
    # time rather than letting it drift to the middle of a widened range.
    face_time: dict[int, float] = {}
    for s in segments:
        idx = _shot_of_segment(shot_starts, s)
        ft = getattr(s, "face_time_sec", None)
        if idx >= 0 and ft is not None and idx not in face_time:
            face_time[idx] = float(ft)

    ranges: list[tuple[int, int, int]] = []  # (lo, hi, seed)
    for seed in sorted(seeds):
        if mode == "conversation":
            lo, hi = _grow_conversation(
                seed,
                cast=cast,
                person_id=person_id,
                partners=partners,
                scene_of=scene_of,
            )
        elif mode == "scene":
            gi = scene_of.get(seed, -1)
            group = next(
                (g for g in scene_groups if int(g.scene_index) == gi), None
            )
            if group and group.shot_indices:
                lo, hi = min(group.shot_indices), max(group.shot_indices)
            else:
                lo = hi = seed
        else:  # "shot"
            lo = hi = seed

        lo = max(0, lo - max(0, int(lead_shots)))
        hi = min(len(shots) - 1, hi + max(0, int(tail_shots)))

        # Trim to the cap, keeping the seed shot centred in what survives.
        while hi > lo:
            a = _shot_bounds(shots, lo)
            b = _shot_bounds(shots, hi)
            if a is None or b is None or (b[1] - a[0]) <= cap:
                break
            if (seed - lo) >= (hi - seed):
                lo += 1
            else:
                hi -= 1

        ranges.append((lo, hi, seed))

    out: list[Segment] = []
    for lo, hi, seed in ranges:
        a = _shot_bounds(shots, lo)
        b = _shot_bounds(shots, hi)
        if a is None or b is None:
            continue
        start, end = a[0], b[1]
        if end - start < 0.04:
            continue
        ft = face_time.get(seed)
        out.append(
            Segment(
                person_id,
                start,
                end,
                preview_start_sec=start,
                preview_end_sec=end,
                face_time_sec=(
                    min(max(ft, start), end) if ft is not None else None
                ),
            )
        )

    # Widened ranges routinely overlap; fold them so a pack never ships the
    # same footage twice. gap_sec=0 keeps genuinely separate scenes separate.
    out = coalesce_segments(out, gap_sec=0.0)
    return subtract_shots(out, shots, dropped)
