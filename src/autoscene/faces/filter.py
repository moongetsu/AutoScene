from __future__ import annotations

from typing import Literal

from autoscene.faces.base import FaceDetection

FaceSelectMode = Literal["all", "largest", "relative", "top_n"]


def _bbox_area(face: FaceDetection) -> int:
    x1, y1, x2, y2 = face.bbox
    return max(0, x2 - x1) * max(0, y2 - y1)


def filter_faces_per_frame(
    faces: list[FaceDetection],
    *,
    mode: str = "all",
    relative_pct: float = 40.0,
    top_n: int = 2,
) -> list[FaceDetection]:
    """
    Per-frame geometric selection. Does not infer story role or depth;
    keeps the largest / most prominent faces by bounding-box area.
    """
    if not faces or mode == "all":
        return faces

    ranked = sorted(faces, key=_bbox_area, reverse=True)
    largest_area = _bbox_area(ranked[0])
    if largest_area <= 0:
        return faces

    if mode == "largest":
        return [ranked[0]]

    if mode == "top_n":
        n = max(1, int(top_n))
        return ranked[:n]

    if mode == "relative":
        pct = max(1.0, min(100.0, float(relative_pct))) / 100.0
        floor = largest_area * pct
        return [f for f in ranked if _bbox_area(f) >= floor]

    return faces
