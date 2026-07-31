from __future__ import annotations

import math
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np


@dataclass
class VideoInfo:
    path: Path
    width: int
    height: int
    fps: float
    frame_count: int
    duration_sec: float


def probe_video(path: Path) -> VideoInfo:
    cap = cv2.VideoCapture(str(path))
    if not cap.isOpened():
        raise RuntimeError(f"Cannot open video: {path}")
    fps = float(cap.get(cv2.CAP_PROP_FPS) or 25.0)
    frame_count = int(cap.get(cv2.CAP_PROP_FRAME_COUNT) or 0)
    width = int(cap.get(cv2.CAP_PROP_FRAME_WIDTH) or 0)
    height = int(cap.get(cv2.CAP_PROP_FRAME_HEIGHT) or 0)
    duration = frame_count / fps if fps > 0 and frame_count > 0 else 0.0
    cap.release()
    return VideoInfo(path, width, height, fps, frame_count, duration)


def sample_step(src_fps: float, sample_fps: float) -> int:
    return max(1, int(round(max(0.1, src_fps) / max(0.1, sample_fps))))


def expected_sample_count(frame_count: int, src_fps: float, sample_fps: float) -> tuple[int, bool]:
    """
    Exact number of frames iter_sampled_frames will yield, when frame_count is trusted.
    Returns (count, approximate).
    """
    step = sample_step(src_fps, sample_fps)
    if frame_count > 0:
        return math.ceil(frame_count / step), False
    # Fallback when container lies about frame_count
    duration = 0.0
    if src_fps > 0 and frame_count > 0:
        duration = frame_count / src_fps
    return max(1, int(round(max(duration, 1.0) * sample_fps))), True


def downscale_for_detect(
    frame_bgr: np.ndarray,
    max_edge: int,
) -> tuple[np.ndarray, float]:
    """
    Shrink frame so longest side <= max_edge. Returns (frame, scale) where
    scale maps detect coords -> original (orig = detect * scale).
    max_edge <= 0 means no resize (scale 1.0).
    """
    if max_edge <= 0:
        return frame_bgr, 1.0
    h, w = frame_bgr.shape[:2]
    long_edge = max(h, w)
    if long_edge <= max_edge:
        return frame_bgr, 1.0
    scale = long_edge / float(max_edge)  # orig = detect * scale
    nw = max(1, int(round(w / scale)))
    nh = max(1, int(round(h / scale)))
    small = cv2.resize(frame_bgr, (nw, nh), interpolation=cv2.INTER_AREA)
    return small, scale


def iter_sampled_frames(path: Path, sample_fps: float = 12.0):
    """Yield (timestamp_sec, frame_bgr) at approximately sample_fps.

    Non-kept frames use grab() so demux/decode can skip full BGR materialize
    when the backend supports it (FFMPEG).
    """
    cap = cv2.VideoCapture(str(path))
    if not cap.isOpened():
        raise RuntimeError(f"Cannot open video: {path}")
    src_fps = float(cap.get(cv2.CAP_PROP_FPS) or 25.0)
    step = sample_step(src_fps, sample_fps)
    idx = 0
    try:
        while True:
            if step > 1 and (idx % step) != 0:
                if not cap.grab():
                    break
                idx += 1
                continue
            ok, frame = cap.read()
            if not ok:
                break
            ts = idx / src_fps if src_fps > 0 else 0.0
            yield ts, frame
            idx += 1
    finally:
        cap.release()


def iter_sampled_frames_prefetched(
    path: Path,
    sample_fps: float = 12.0,
    queue_size: int = 8,
):
    """
    Same as iter_sampled_frames, but decode runs on a background thread so the
    GPU detect loop is not stuck waiting on OpenCV reads.
    """
    from queue import Empty, Queue
    import threading

    depth = max(1, int(queue_size))
    q: Queue = Queue(maxsize=depth)
    sentinel = object()
    errors: list[BaseException] = []

    def worker() -> None:
        try:
            for item in iter_sampled_frames(path, sample_fps=sample_fps):
                q.put(item)
        except BaseException as exc:  # noqa: BLE001 — surface to consumer
            errors.append(exc)
        finally:
            q.put(sentinel)

    thread = threading.Thread(
        target=worker,
        name="autoscene-decode-prefetch",
        daemon=True,
    )
    thread.start()
    try:
        while True:
            try:
                item = q.get(timeout=120.0)
            except Empty as exc:
                raise RuntimeError("Decode prefetch stalled (no frames for 120s)") from exc
            if item is sentinel:
                break
            yield item
    finally:
        thread.join(timeout=5.0)
    if errors:
        raise errors[0]
