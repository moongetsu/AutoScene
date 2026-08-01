"""Group consecutive shots into dramatic scenes.

A shot is one cut-to-cut plate. A *scene* is the run of shots that belong
together: the establishing wide, the conversation that follows, the reaction
at the end. ScenePack creators keep that whole run, which is why packs need a
unit above the shot.

Two signals decide where one scene ends and the next begins, both already paid
for by the scan:

  visual — cosine distance between per-shot fingerprints (shot_fingerprint.py),
           which is what carries a faceless establishing shot into the scene it
           introduces
  cast   — whether neighbouring shots share cast, including the A/B/A/B
           alternation of a reverse-shot exchange, where adjacent shots share
           nobody yet plainly belong to one conversation

A boundary is declared only when BOTH break. Either one alone is too eager:
cut to a close-up and visual similarity drops inside a single scene; cut to a
new location with the same character and cast continuity persists across a real
scene change.
"""

from __future__ import annotations

import logging
from dataclasses import dataclass

import numpy as np

from autoscene.shot_fingerprint import fingerprint_similarity

log = logging.getLogger(__name__)

# Below this fingerprint cosine, two shots look like different places.
VISUAL_BREAK_SIM = 0.82
# How far to look for cast continuity across a candidate boundary. 2 covers the
# A/B/A/B reverse-shot pattern; wider starts gluing adjacent scenes that happen
# to share a lead character.
CAST_LOOKAROUND = 2
# Never emit a scene longer than this without a boundary — a hard ceiling so a
# visually flat sequence cannot swallow an entire act.
MAX_SCENE_SEC = 300.0


@dataclass(frozen=True)
class SceneGroup:
    scene_index: int
    start_sec: float
    end_sec: float
    shot_indices: list[int]


def cast_by_shot(shots: list, segments: list) -> list[set[int]]:
    """
    Which cast members appear in each shot.

    Segments produced by the shots branch of cluster.build_segments are already
    aligned to shot plates, so matching on the start time is exact rather than
    an interval search.
    """
    starts = np.asarray([float(s.start_sec) for s in shots], dtype=np.float64)
    out: list[set[int]] = [set() for _ in shots]
    if not len(starts):
        return out
    for seg in segments:
        # Nearest shot start; segments and shots share the same boundaries.
        idx = int(np.argmin(np.abs(starts - float(seg.start_sec))))
        if abs(starts[idx] - float(seg.start_sec)) <= 0.05:
            out[idx].add(int(seg.person_id))
    return out


def _cast_continuous(cast: list[set[int]], i: int, look: int) -> bool:
    """
    True when cast bridges the gap between shot i and i+1.

    Checks the shots either side as a block, not just the touching pair, so a
    reverse-shot exchange (A | B | A | B) still reads as one conversation even
    though no two adjacent shots share a face.
    """
    lo = max(0, i - look + 1)
    hi = min(len(cast), i + look + 1)
    before: set[int] = set().union(*cast[lo : i + 1]) if i >= lo else set()
    after: set[int] = set().union(*cast[i + 1 : hi]) if hi > i + 1 else set()
    if not before or not after:
        # A faceless shot carries no cast evidence either way; let the visual
        # signal decide alone rather than forcing a boundary here.
        return False
    return bool(before & after)


def build_scene_groups(
    shots: list,
    segments: list,
    *,
    fingerprints: np.ndarray | None = None,
    visual_break_sim: float = VISUAL_BREAK_SIM,
    look: int = CAST_LOOKAROUND,
    max_scene_sec: float = MAX_SCENE_SEC,
) -> list:
    """
    Partition shots into scenes. Returns project.SceneGroupRecord rows.

    Without fingerprints (older projects, or a scan that could not write them)
    this falls back to cast continuity alone — weaker on establishing shots,
    but still better than treating every shot as its own scene.
    """
    from autoscene.project import SceneGroupRecord

    n = len(shots)
    if n == 0:
        return []

    cast = cast_by_shot(shots, segments)
    has_fp = (
        fingerprints is not None
        and getattr(fingerprints, "ndim", 0) == 2
        and fingerprints.shape[0] >= n
    )

    groups: list[SceneGroupRecord] = []
    current: list[int] = [0]

    def flush() -> None:
        first, last = current[0], current[-1]
        groups.append(
            SceneGroupRecord(
                scene_index=len(groups),
                start_sec=float(shots[first].start_sec),
                end_sec=float(shots[last].end_sec),
                shot_indices=list(current),
            )
        )

    for i in range(n - 1):
        if has_fp:
            visual_sim = fingerprint_similarity(fingerprints[i], fingerprints[i + 1])
            visual_breaks = visual_sim < visual_break_sim
        else:
            # No visual evidence: let cast alone decide.
            visual_breaks = True

        boundary = visual_breaks and not _cast_continuous(cast, i, look)

        span = float(shots[i].end_sec) - float(shots[current[0]].start_sec)
        if span >= max_scene_sec:
            boundary = True

        if boundary:
            flush()
            current = [i + 1]
        else:
            current.append(i + 1)

    flush()
    log.info(
        "scene groups shots=%s groups=%s visual=%s",
        n,
        len(groups),
        "yes" if has_fp else "cast-only",
    )
    return groups


def scene_index_for_shot(groups: list, shot_index: int) -> int:
    """Which scene a shot belongs to; -1 when ungrouped."""
    for g in groups:
        if shot_index in g.shot_indices:
            return int(g.scene_index)
    return -1
