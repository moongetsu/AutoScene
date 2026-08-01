from __future__ import annotations

import logging
import math
import subprocess
from dataclasses import dataclass
from pathlib import Path

import cv2
import numpy as np

log = logging.getLogger(__name__)


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


# Per-shot sample budget. A cut-to-cut shot shows the same faces from the same
# angle, so a dense uniform sweep spends most of its detector calls re-embedding
# a face it already has.
#
# Samples are laid down in BURSTS, not spread evenly. identity_gates gate 3
# links detections into tracklets only while consecutive hits are within
# cfg.max_track_gap_sec (0.40-0.45s) of each other, and drops any track shorter
# than cfg.min_track_len (2-3). Thinning a shot to evenly-spaced samples would
# push every pair past that gap, orphan the lot, and silently lose cast. So each
# burst is a short run of frames tight enough to link, and bursts are spread
# across the shot to capture different angles / expressions.
SHOT_BURST_FRAMES = 4
# Start a new burst roughly every N seconds of shot time.
SHOT_SECONDS_PER_BURST = 4.0
SHOT_MAX_BURSTS = 6
# Sample inside this fraction of the shot, never on the boundary frames —
# cut-adjacent frames catch dissolves, motion blur and residual fades.
_SHOT_INSET = 0.15


@dataclass(frozen=True)
class SamplePlan:
    """Which timestamps the detector should actually see.

    ``timestamps`` is sorted and monotonic; ``shot_index[i]`` is the shot that
    produced ``timestamps[i]`` (-1 when planning without cuts).
    """

    timestamps: list[float]
    shot_index: list[int]

    def __len__(self) -> int:
        return len(self.timestamps)


def plan_shot_samples(
    shots: list,
    sample_fps: float,
    *,
    max_track_gap_sec: float,
    min_track_len: int,
    burst_frames: int = SHOT_BURST_FRAMES,
    seconds_per_burst: float = SHOT_SECONDS_PER_BURST,
    max_bursts: int = SHOT_MAX_BURSTS,
) -> SamplePlan:
    """
    Lay bursts of tightly-spaced samples across each shot.

    Burst length is at least ``min_track_len + 1`` so every burst survives gate
    3, and intra-burst spacing is capped below ``max_track_gap_sec`` so its hits
    actually link into a tracklet. A short shot gets one burst; a long one gets
    a burst roughly every ``seconds_per_burst`` up to ``max_bursts``.

    Both bounds come from the caller's GateConfig — sampling and the identity
    gates have to agree or the scan quietly throws its own detections away.
    """
    timestamps: list[float] = []
    shot_index: list[int] = []
    rate = max(0.1, float(sample_fps))
    # Stay clear of the gate's cutoff; landing exactly on it is a coin flip.
    spacing = min(1.0 / rate, 0.8 * float(max_track_gap_sec))
    per_burst = max(int(burst_frames), int(min_track_len) + 1)

    for idx, shot in enumerate(shots):
        start = float(shot.start_sec)
        end = float(shot.end_sec)
        span = end - start
        if span <= 0:
            continue

        inset = span * _SHOT_INSET
        a = start + inset
        b = end - inset
        if b <= a:
            a = b = start + span / 2.0
        usable = b - a

        # A burst needs (per_burst - 1) * spacing of room; shrink it for shots
        # too short to hold a full one rather than spilling across the cut.
        n_frames = per_burst
        if usable < (per_burst - 1) * spacing:
            n_frames = max(1, int(usable / spacing) + 1)
        burst_span = (n_frames - 1) * spacing

        n_bursts = round(span / max(0.1, float(seconds_per_burst)))
        n_bursts = max(1, min(max_bursts, n_bursts))
        # Bursts must not overlap each other.
        if burst_span > 0:
            n_bursts = max(1, min(n_bursts, int(usable / burst_span) or 1))

        head_room = max(0.0, usable - burst_span)
        gap = head_room / (n_bursts - 1) if n_bursts > 1 else 0.0
        for bi in range(n_bursts):
            base = a + gap * bi
            for k in range(n_frames):
                t = base + spacing * k
                if t > end:
                    break
                timestamps.append(t)
                shot_index.append(idx)

    return SamplePlan(timestamps=timestamps, shot_index=shot_index)


def plan_uniform_samples(duration_sec: float, sample_fps: float) -> SamplePlan:
    """Fallback plan when there are no shot cuts: even ticks across the file."""
    rate = max(0.1, float(sample_fps))
    n = max(1, math.floor(max(0.0, float(duration_sec)) * rate))
    step = 1.0 / rate
    return SamplePlan(
        timestamps=[i * step for i in range(n)],
        shot_index=[-1] * n,
    )


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


def iter_sampled_frames_ffmpeg(
    path: Path,
    sample_fps: float = 12.0,
    *,
    width: int,
    height: int,
):
    """
    Yield (timestamp_sec, frame_bgr) through an ffmpeg pipe with hardware decode.

    OpenCV's VideoCapture demuxes and decodes every frame in software on the
    calling thread. ffmpeg gets `-hwaccel cuda` on Windows and `videotoolbox` on
    macOS (see ffmpeg_encode.hwaccel_input_args), and its `fps` filter drops the
    frames we would have thrown away anyway before they are ever converted.

    Frames come out at native resolution on purpose. `min_face_size` is defined
    in source pixels and re-checked against them in pipeline._map_faces_to_source,
    so pre-scaling here would quietly tighten that threshold on UHD sources.
    Scaling stays in downscale_for_detect where the coord_scale bookkeeping lives.
    """
    from autoscene.export import ensure_ffmpeg
    from autoscene.ffmpeg_encode import hwaccel_input_args

    ffmpeg = ensure_ffmpeg()
    rate = max(0.1, float(sample_fps))
    frame_bytes = int(width) * int(height) * 3
    if frame_bytes <= 0:
        raise RuntimeError(f"Bad frame geometry {width}x{height}")

    def build(hw: list[str]) -> list[str]:
        return [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            *hw,
            "-i",
            str(path),
            "-vf",
            f"fps={rate:.6f}",
            "-f",
            "rawvideo",
            "-pix_fmt",
            "bgr24",
            "pipe:1",
        ]

    hw = hwaccel_input_args(zero_copy=False)
    proc = subprocess.Popen(
        build(hw), stdout=subprocess.PIPE, stderr=subprocess.PIPE, bufsize=0
    )
    idx = 0
    try:
        assert proc.stdout is not None
        buf = bytearray(frame_bytes)
        view = memoryview(buf)
        while True:
            filled = 0
            while filled < frame_bytes:
                got = proc.stdout.readinto(view[filled:])
                if not got:
                    break
                filled += got
            if filled < frame_bytes:
                if idx == 0:
                    # Nothing decoded at all: hwaccel is unusable on this file.
                    err = (proc.stderr.read() or b"").decode("utf-8", "replace")
                    raise RuntimeError(f"ffmpeg decode produced no frames: {err[-400:]}")
                break
            frame = np.frombuffer(bytes(buf), np.uint8).reshape(height, width, 3)
            yield idx / rate, frame
            idx += 1
    finally:
        if proc.poll() is None:
            proc.kill()
        for pipe in (proc.stdout, proc.stderr):
            try:
                if pipe is not None:
                    pipe.close()
            except OSError:
                pass
        proc.wait(timeout=5)


def _iter_source_frames(
    path: Path,
    sample_fps: float,
    info: VideoInfo | None,
):
    """
    Decode the sampled frames, honouring settings.decode_backend.

    OpenCV is the default because it measured faster than an ffmpeg rawvideo
    pipe on every case tried on Apple Silicon (720p 6.7x, 1080p 1.9x, 4K 1.2x),
    with `-hwaccel videotoolbox` no better than software decode. OpenCV's own
    FFmpeg backend decodes in-process, so the pipe only adds serialization of
    every frame — 4.5GB of raw pixels for a 30s 4K clip.

    The pipe stays available because `-hwaccel cuda` on a Windows NVIDIA box is
    a genuinely different path that was NOT measured here; leave the default
    alone unless a benchmark on that hardware says otherwise.
    """
    from autoscene.config import settings as _settings

    backend = str(getattr(_settings, "decode_backend", "opencv") or "opencv").lower()
    if backend == "ffmpeg" and info is not None and info.width > 0 and info.height > 0:
        try:
            gen = iter_sampled_frames_ffmpeg(
                path, sample_fps=sample_fps, width=info.width, height=info.height
            )
            first = next(gen, None)
            if first is not None:
                yield first
                yield from gen
                return
            log.warning("ffmpeg decode yielded nothing — falling back to OpenCV")
        except Exception as exc:  # noqa: BLE001
            # A broken/odd ffmpeg build must degrade, not dead-end the scan.
            log.warning("ffmpeg decode unavailable (%s) — falling back to OpenCV", exc)
    yield from iter_sampled_frames(path, sample_fps=sample_fps)


def iter_planned_frames(
    path: Path,
    plan: SamplePlan,
    *,
    sample_fps: float,
    info: VideoInfo | None = None,
):
    """
    Yield only the frames the sample plan asks for, as (timestamp, frame, shot).

    Decode stays sequential — seeking a thousand times costs more than letting
    hardware decode stream through the file. What the plan saves is detector
    work, which runs 10-40x the cost of decoding one frame.
    """
    if not plan.timestamps:
        return
    targets = plan.timestamps
    shots = plan.shot_index
    n = len(targets)
    cursor = 0
    tolerance = 0.5 / max(0.1, float(sample_fps))

    for ts, frame in _iter_source_frames(path, sample_fps, info):
        if cursor >= n:
            break
        # Skip past any targets this frame already overshot (short shots can
        # request two samples that land inside one decoded frame interval).
        while cursor < n and targets[cursor] < ts - tolerance:
            cursor += 1
        if cursor >= n:
            break
        if targets[cursor] <= ts + tolerance:
            shot = shots[cursor]
            cursor += 1
            # Collapse any further targets this same frame satisfies.
            while cursor < n and targets[cursor] <= ts + tolerance:
                cursor += 1
            yield ts, frame, shot


def iter_sampled_frames_prefetched(
    path: Path,
    sample_fps: float = 12.0,
    queue_size: int = 8,
    *,
    plan: SamplePlan | None = None,
    info: VideoInfo | None = None,
):
    """
    Same as iter_sampled_frames, but decode runs on a background thread so the
    GPU detect loop is not stuck waiting on reads.

    With a plan, yields (timestamp, frame, shot_index); without one, the legacy
    (timestamp, frame) pairs.
    """
    from queue import Empty, Queue
    import threading

    depth = max(1, int(queue_size))
    q: Queue = Queue(maxsize=depth)
    sentinel = object()
    errors: list[BaseException] = []

    def worker() -> None:
        try:
            if plan is not None:
                source = iter_planned_frames(
                    path, plan, sample_fps=sample_fps, info=info
                )
            else:
                source = _iter_source_frames(path, sample_fps, info)
            for item in source:
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
