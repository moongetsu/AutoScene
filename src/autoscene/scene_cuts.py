"""Shot / transition boundary detection for character scene packs.

Default: PySceneDetect AdaptiveDetector (HSV frame-delta vs rolling average).
Optional: TransNetV2 on GPU + photometric snap for dissolve-heavy footage.
App path: api._scan_job → run_scene_cuts_phase → detect_shot_spans.
"""

from __future__ import annotations

import logging
import os
import subprocess
import time
from collections.abc import Callable
from dataclasses import dataclass
from pathlib import Path
from typing import Any, Literal

log = logging.getLogger(__name__)

SceneCutMode = Literal["adaptive", "transnet", "auto"]
CutProgressCb = Callable[[dict[str, Any]], None]


@dataclass(frozen=True)
class ShotSpan:
    start_sec: float
    end_sec: float


def _shots_from_cuts(cuts: list[float], duration_sec: float) -> list[ShotSpan]:
    duration = max(0.0, float(duration_sec))
    bounds = [0.0]
    for c in sorted(float(x) for x in cuts):
        if 0.12 < c < duration - 0.05:
            if not bounds or c - bounds[-1] >= 0.12:
                bounds.append(c)
    if not bounds or bounds[-1] < duration - 0.02:
        bounds.append(duration)
    if len(bounds) < 2:
        return [ShotSpan(0.0, duration or 0.01)]
    out: list[ShotSpan] = []
    for i in range(len(bounds) - 1):
        a, b = bounds[i], bounds[i + 1]
        if b - a >= 0.08:
            out.append(ShotSpan(a, b))
    return out or [ShotSpan(0.0, duration or 0.01)]


def require_torch_device() -> str:
    """
    Resolve the torch device for TransNetV2: cuda, else MPS on Apple Silicon.

    MPS does not implement every op TransNetV2 uses; PYTORCH_ENABLE_MPS_FALLBACK
    lets the unsupported ones run on CPU instead of aborting the whole pass.
    """
    from autoscene.gpu import ensure_cuda_dll_path
    from autoscene.models_vault import ensure_torch_dll_path

    ensure_cuda_dll_path()
    ensure_torch_dll_path()
    try:
        import torch
    except Exception as exc:  # noqa: BLE001
        raise RuntimeError(
            f"Scene detection requires PyTorch ({exc}). Open Settings → Packs, "
            "install the 'PyTorch (CUDA)' pack, then restart AutoScene."
        ) from exc

    if torch.cuda.is_available():
        log.info(
            "scene cuts GPU torch=%s device=%s",
            torch.__version__,
            torch.cuda.get_device_name(0),
        )
        return "cuda"

    mps = getattr(torch.backends, "mps", None)
    if mps is not None and mps.is_available():
        os.environ.setdefault("PYTORCH_ENABLE_MPS_FALLBACK", "1")
        log.info("scene cuts torch=%s device=mps", torch.__version__)
        return "mps"

    raise RuntimeError(
        "Graphics hardware is not ready for scene detection. "
        "Open Settings → Packs, install the required packs, then restart AutoScene."
    )


# Back-compat alias
def require_torch_cuda() -> str:
    return require_torch_device()


def transnet_available() -> bool:
    try:
        import transnetv2_pytorch  # noqa: F401

        return True
    except Exception:  # noqa: BLE001
        return False


def adaptive_available() -> bool:
    # scenedetect is a core dependency, so the adaptive path always has a detector.
    return True


def _transnet_threshold_for_mode(mode: SceneCutMode, base: float) -> float:
    t = float(base)
    if mode == "adaptive":
        return max(0.1, min(0.95, t * 0.7))
    return max(0.1, min(0.95, t))


def _count_provisional_cuts(probs: list[float], threshold: float) -> int:
    if not probs:
        return 0
    n = 0
    prev = 0.0
    thr = float(threshold)
    for p in probs:
        v = float(p)
        if v >= thr and prev < thr:
            n += 1
        prev = v
    return n


def _frame_pair_delta(a, b) -> float:
    """Mean abs RGB delta between two HxWx3 uint8 frames (float64)."""
    import numpy as np

    return float(np.mean(np.abs(a.astype(np.float32) - b.astype(np.float32))))


def snap_cut_indices(
    frames,
    cut_indices: list[int],
    *,
    window: int = 12,
    min_delta: float = 8.0,
) -> list[int]:
    """
    Snap TransNet cut frame indices to the strongest hard-cut peak nearby.

    TransNet fires on a soft rising edge inside a temporal window, so hard
    cuts land a few frames early/late. For each proposed index, search
    [i-window, i+window] for the max consecutive-frame delta and move there
    when the peak is sharp enough (real hard cut). Soft/dissolve proposals
    with a weak peak keep the original TransNet index.

    frames: array-like (N, H, W, 3)
    """
    import numpy as np

    if frames is None:
        return list(cut_indices)
    arr = np.asarray(frames)
    n = int(arr.shape[0])
    if n < 2 or not cut_indices:
        return list(cut_indices)

    win = max(1, int(window))
    snapped: list[int] = []
    for raw in cut_indices:
        i0 = int(raw)
        lo = max(1, i0 - win)
        hi = min(n - 1, i0 + win)
        best_i = i0
        best_d = -1.0
        second_d = -1.0
        for i in range(lo, hi + 1):
            d = _frame_pair_delta(arr[i - 1], arr[i])
            if d > best_d:
                second_d = best_d
                best_d = d
                best_i = i
            elif d > second_d:
                second_d = d
        # Hard cut: clear photometric spike. Otherwise keep TransNet time
        # (dissolves / whip pans have flatter deltas).
        sharp = best_d >= float(min_delta) and (
            second_d < 0 or best_d >= second_d * 1.25 or best_d - second_d >= 4.0
        )
        snapped.append(best_i if sharp else i0)

    # Dedupe + sort (two proposals can snap to the same peak).
    out = sorted(set(snapped))
    # Drop near-duplicates within 2 frames (keep earliest).
    cleaned: list[int] = []
    for idx in out:
        if cleaned and idx - cleaned[-1] < 2:
            continue
        cleaned.append(idx)
    return cleaned


def rising_edge_cut_indices(probs: list[float], threshold: float) -> list[int]:
    """Frame indices where TransNet probability crosses threshold upward."""
    thr = float(threshold)
    prev = 0.0
    cuts: list[int] = []
    for i, p in enumerate(probs):
        v = float(p)
        if v >= thr and prev < thr and i > 0:
            cuts.append(i)
        prev = v
    return cuts


def transition_boundary_indices(probs: list[float], threshold: float) -> list[int]:
    """
    New-shot frame = first frame AFTER an above-threshold TransNet run.

    Empirically (Main10 animation @24fps): rising-edge is systematically
    1 frame early vs the true hard-cut photometric peak; the 1→0 edge
    matches full-res peak (mean abs error 0 on sampled cuts). This also
    matches TransNet's own predictions_to_scenes next-scene start.
    """
    thr = float(threshold)
    prev_on = False
    cuts: list[int] = []
    for i, p in enumerate(probs):
        on = float(p) >= thr
        if prev_on and not on and i > 0:
            cuts.append(i)
        prev_on = on
    return cuts


def merge_cut_windows(cut_times: list[float], pad_sec: float) -> list[tuple[float, float]]:
    """Sorted, merged [t-pad, t+pad] windows around candidate cuts."""
    windows: list[list[float]] = []
    for t in sorted(float(x) for x in cut_times):
        a, b = t - pad_sec, t + pad_sec
        if windows and a <= windows[-1][1]:
            windows[-1][1] = max(windows[-1][1], b)
        else:
            windows.append([max(0.0, a), b])
    return [(a, b) for a, b in windows]


def parse_scene_score_lines(text: str) -> list[tuple[float, float]]:
    """
    (pts_time, change score) rows from `metadata=mode=print` output.

    Two layouts are supported:
    - ffmpeg < 8: ``frame:123 pts:2952 pts_time:1.23 ... lavfi.scene_score=0.8342``
      (single line).
    - ffmpeg >= 8: ``frame:0 pts:57856 pts_time:4.52`` followed by
      ``lavfi.scd.score=0.142`` on its own line.
    """
    import re

    out: list[tuple[float, float]] = []
    pts_pat = re.compile(r"pts_time:([-+0-9.eE]+)")
    score_pat = re.compile(r"lavfi\.(?:scene_score|scd\.score)=([-+0-9.eE]+)")
    cur_pts: float | None = None
    for line in text.splitlines():
        mpts = pts_pat.search(line)
        if mpts:
            try:
                cur_pts = float(mpts.group(1))
            except ValueError:
                cur_pts = None
        ms = score_pat.search(line)
        if ms and cur_pts is not None:
            try:
                out.append((cur_pts, float(ms.group(1))))
            except ValueError:
                pass
    return out


def pick_cut_peaks(
    cut_times: list[float],
    scores: list[tuple[float, float]],
    *,
    pad_sec: float,
    min_score: float = 0.35,
) -> list[float]:
    """
    Move each candidate cut onto the strongest frame-delta peak near it.

    scores: (pts_time, scene_score) over the whole file (already restricted to
    windows around the candidates). A score at frame k measures the k vs k-1
    delta, so the peak frame is exactly the first frame of the new shot —
    the same convention detect_cuts_transnet emits. Candidates with no clear
    peak nearby (dissolves, whip pans) keep their TransNet time.
    """
    if not scores:
        return list(cut_times)
    pts = [p for p, _s in scores]
    vals = [s for _p, s in scores]
    import bisect

    out: list[float] = []
    for t in cut_times:
        t = float(t)
        lo = bisect.bisect_left(pts, t - pad_sec)
        hi = bisect.bisect_right(pts, t + pad_sec)
        if hi - lo < 2:
            out.append(t)
            continue
        best_i = lo
        second = -1.0
        for i in range(lo, hi):
            if vals[i] > vals[best_i]:
                second = max(second, vals[best_i])
                best_i = i
            elif vals[i] > second:
                second = vals[i]
        best = vals[best_i]
        sharp = best >= float(min_score) and (
            second < 0 or best >= second * 1.3 or best - second >= 0.15
        )
        out.append(pts[best_i] if sharp else t)
    return out


def refine_cuts_at_source(
    video_path: Path,
    cut_times: list[float],
    *,
    pad_sec: float = 0.45,
    min_score: float = 0.35,
    width: int = 320,
) -> list[float]:
    """
    Frame-perfect cut refine on the SOURCE timeline.

    TransNet runs on a 48x27 @24fps resample, so its cut seconds are only as
    good as that grid. This pass decodes only the small windows around each
    candidate (input seek via -ss/-t with -copyts so pts_time stays on the
    source timeline), computes ffmpeg's per-frame change score via the
    `scdet` filter (ffmpeg >= 8; `scene` on older builds), and snaps the cut
    onto the exact source frame whose score peaks.

    Degrades cleanly: any ffmpeg/parse trouble returns the input cuts.
    """
    cuts = [float(t) for t in cut_times]
    if not cuts:
        return cuts

    from autoscene.export import ensure_ffmpeg

    ffmpeg = ensure_ffmpeg()
    windows = merge_cut_windows(cuts, pad_sec)

    all_scores: list[tuple[float, float]] = []
    # Chunk so giant cut lists never brush the process command-line limit.
    chunk = 350
    for start in range(0, len(windows), chunk):
        part = windows[start : start + chunk]
        first = part[0][0]
        last_end = part[-1][1]
        dur = max(0.1, last_end - first) + 0.3
        cmd = [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            # Input seek: decode ONLY this chunk's range, not the whole file.
            # -copyts keeps original pts_time so scores stay on the source
            # timeline (input seeking alone resets timestamps to zero).
            "-ss",
            f"{first:.6f}",
            "-t",
            f"{dur:.6f}",
            "-copyts",
            "-i",
            str(video_path),
            "-vf",
            # scdet = ffmpeg 8's scene-change filter (the old `scene` filter
            # was removed there). threshold=0 so every frame gets a score.
            f"scale={int(width)}:-2,scdet=threshold=0,metadata=mode=print:file=-",
            "-an",
            "-f",
            "null",
            "-",
        ]
        proc = subprocess.run(cmd, capture_output=True, text=True, check=False)
        if proc.returncode != 0:
            err = (proc.stderr or "")[-300:]
            log.warning("source cut refine chunk failed: %s", err)
            return cuts
        all_scores.extend(parse_scene_score_lines(proc.stdout or ""))

    if not all_scores:
        return cuts

    all_scores.sort(key=lambda row: row[0])
    refined = pick_cut_peaks(cuts, all_scores, pad_sec=pad_sec, min_score=min_score)
    moved = sum(1 for a, b in zip(cuts, refined) if abs(a - b) > 1e-3)
    log.info(
        "source-res cut refine cuts=%s moved=%s pad=%.2fs min_score=%.2f video=%s",
        len(cuts),
        moved,
        pad_sec,
        min_score,
        Path(video_path).name,
    )
    return refined





def _assert_model_on_device(model: Any, device: str) -> None:
    """
    Sanity-check that the model's parameters landed on the requested device.

    After ``model.to(dev)`` a silent no-op (e.g. CUDA init failed and torch
    fell back to CPU) would run the whole pass on CPU without anyone noticing.
    Log so the trace tells the real story; do not raise — the scan should still
    complete on CPU rather than dead-end the import.
    """
    import torch

    want = str(torch.device(device))
    try:
        got = str(next(model.parameters()).device)
    except StopIteration:
        return
    if got != want:
        log.warning(
            "TransNetV2 device mismatch: requested=%s got=%s — running on %s",
            want,
            got,
            got,
        )


# Back-compat alias
def _assert_model_on_cuda(model: Any) -> None:
    _assert_model_on_device(model, "cuda")


def _extract_frames_rgb_48x27(
    video_path: Path,
    *,
    on_progress: CutProgressCb | None = None,
    device: str = "cuda",
) -> tuple[Any, float]:
    """Decode to TransNet 48x27 RGB. Prefer ffmpeg hardware decode."""
    import numpy as np

    from autoscene.export import ensure_ffmpeg
    from autoscene.ffmpeg_encode import hwaccel_input_args

    fps = 24.0
    try:
        probe = subprocess.run(
            [
                "ffprobe",
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=r_frame_rate",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
                str(video_path),
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        raw = (probe.stdout or "").strip().splitlines()
        if raw and "/" in raw[0]:
            a, b = raw[0].split("/", 1)
            fps = float(a) / max(float(b), 1e-6)
        elif raw:
            fps = float(raw[0])
    except Exception as exc:  # noqa: BLE001
        log.debug("ffprobe fps fallback: %s", exc)

    if on_progress:
        on_progress(
            {
                "phase": "extract",
                "progress": 0.02,
                "stage": "Preparing frames…",
                "frames_done": 0,
                "frames_total": 0,
                "cuts_found": 0,
                "device": device,
            }
        )

    ffmpeg = ensure_ffmpeg()
    hw = hwaccel_input_args(zero_copy=False)
    if on_progress:
        on_progress(
            {
                "phase": "extract",
                "progress": 0.08,
                "stage": "Reading frames…" if hw else "Reading frames…",
                "frames_done": 0,
                "frames_total": 0,
                "cuts_found": 0,
                "device": device,
            }
        )

    def _run(with_hw: bool) -> subprocess.CompletedProcess[bytes]:
        cmd = [
            ffmpeg,
            "-hide_banner",
            "-loglevel",
            "error",
            *(hw if with_hw else []),
            "-i",
            str(video_path),
            "-f",
            "rawvideo",
            "-pix_fmt",
            "rgb24",
            "-s",
            "48x27",
            "pipe:1",
        ]
        return subprocess.run(cmd, capture_output=True, check=False)

    proc = _run(bool(hw))
    if proc.returncode != 0 and hw:
        log.warning("Hardware decode extract failed; retry software decode")
        proc = _run(False)
    if proc.returncode != 0:
        err = (proc.stderr or b"").decode("utf-8", errors="replace")[-500:]
        raise RuntimeError(f"ffmpeg frame extract failed: {err}")

    video = np.frombuffer(proc.stdout, np.uint8)
    if video.size % (27 * 48 * 3) != 0:
        usable = (video.size // (27 * 48 * 3)) * (27 * 48 * 3)
        video = video[:usable]
    frames = video.reshape([-1, 27, 48, 3])
    n = int(frames.shape[0])
    if on_progress:
        on_progress(
            {
                "phase": "extract",
                "progress": 0.22,
                "stage": f"Ready · {n} frames",
                "frames_done": 0,
                "frames_total": n,
                "cuts_found": 0,
                "device": device,
            }
        )
    return frames, float(fps)


def detect_cuts_transnet(
    video_path: Path,
    *,
    threshold: float = 0.5,
    on_progress: CutProgressCb | None = None,
) -> list[float]:
    """TransNetV2 GPU inference (CUDA or MPS) used by the app cut phase."""
    device = require_torch_device()
    if not transnet_available():
        raise RuntimeError(
            "Scene pack is missing. Open Settings → Packs, install required packs, then retry."
        )

    from transnetv2_pytorch import TransNetV2  # type: ignore
    import numpy as np
    import torch

    # Pre-infer prep: UI hides scene count until phase=infer.
    if on_progress:
        on_progress(
            {
                "phase": "extract",
                "progress": 0.01,
                "stage": "Preparing scene pass…",
                "frames_done": 0,
                "frames_total": 0,
                "cuts_found": 0,
                "device": device,
            }
        )

    is_cuda = device == "cuda"
    dev = torch.device(device)

    def _mem() -> int:
        """Allocated bytes — only CUDA reports this reliably."""
        return int(torch.cuda.memory_allocated(0)) if is_cuda else 0

    model = TransNetV2(device=device)
    model.to(dev)
    model.device = dev
    model.eval()
    _assert_model_on_device(model, device)

    mem0 = _mem()
    log.info(
        "APP TransNetV2 device=%s gpu=%s threshold=%.3f mem0=%s video=%s",
        model.device,
        torch.cuda.get_device_name(0) if is_cuda else device,
        threshold,
        mem0,
        video_path.name,
    )

    frames_np, fps = _extract_frames_rgb_48x27(
        video_path, on_progress=on_progress, device=device
    )
    n_frames = int(frames_np.shape[0])
    if n_frames < 2:
        return []

    # Keep CPU copy for hard-cut photometric snap after inference.
    frames_cpu = np.array(frames_np, copy=True)
    frames = torch.from_numpy(frames_cpu).to(device=dev, non_blocking=True)
    del frames_np
    if frames.device.type != device:
        raise RuntimeError(f"Frame tensor on {frames.device}, not {device}")
    if is_cuda:
        torch.cuda.synchronize()
    mem1 = _mem()
    log.info(
        "frames on %s count=%s mem_alloc=%s (+%s)", device, n_frames, mem1, mem1 - mem0
    )
    # Memory accounting is CUDA-only; on MPS this check would false-positive.
    if is_cuda and mem1 <= mem0 and n_frames > 100:
        raise RuntimeError(
            "CUDA memory did not rise after frame upload — refusing silent CPU path"
        )

    window_size = 100
    step_size = 50
    no_padded_frames_start = 25
    no_padded_frames_end = (
        25
        + step_size
        - (n_frames % step_size if n_frames % step_size != 0 else step_size)
    )
    padded = torch.cat(
        [torch.unsqueeze(frames[0], 0)] * no_padded_frames_start
        + [frames]
        + [torch.unsqueeze(frames[-1], 0)] * no_padded_frames_end,
        0,
    )
    if padded.device.type != device:
        raise RuntimeError(f"Padded tensor on {padded.device}, not {device}")

    predictions: list[torch.Tensor] = []
    ptr = 0
    thr = float(threshold)
    last_emit = 0.0
    total_windows = max(1, (len(padded) - window_size) // step_size + 1)
    win_i = 0

    while ptr + window_size <= len(padded):
        batch = padded[ptr : ptr + window_size]
        ptr += step_size
        win_i += 1
        if batch.device.type != device:
            raise RuntimeError(f"Infer batch on {batch.device}, not {device}")
        with torch.no_grad():
            inp = batch.unsqueeze(0)
            if inp.device.type != device:
                raise RuntimeError(f"Infer input on {inp.device}, not {device}")
            single_frame_pred, _all_pred = model.predict_raw(inp)
            if single_frame_pred.device.type != device:
                raise RuntimeError(
                    f"Model output on {single_frame_pred.device}, not {device}"
                )
            chunk = single_frame_pred[0, 25:75, 0].detach().float().cpu()
            predictions.append(chunk.clone())

        done_frames = min(len(predictions) * step_size, n_frames)
        cuts_found = (
            _count_provisional_cuts(
                torch.cat(predictions, 0)[:done_frames].tolist(), thr
            )
            if predictions
            else 0
        )
        now = time.monotonic()
        if on_progress and (now - last_emit >= 0.35 or win_i >= total_windows):
            last_emit = now
            frac = done_frames / max(n_frames, 1)
            on_progress(
                {
                    "phase": "infer",
                    "progress": 0.22 + 0.70 * frac,
                    # UI shows cuts_found only; keep stage short for side panels.
                    "stage": "Scanning scenes…",
                    "frames_done": done_frames,
                    "frames_total": n_frames,
                    "cuts_found": cuts_found,
                    "device": device,
                    "cuda_memory_alloc": _mem(),
                }
            )

    probs = torch.cat(predictions, 0)[:n_frames].numpy().tolist()
    # Prefer transition-end (not rising-edge): rising is ~1f early on hard cuts.
    raw_idx = transition_boundary_indices(probs, thr)
    if not raw_idx:
        # Fallback if threshold run never drops (degenerate all-1 predictions).
        raw_idx = rising_edge_cut_indices(probs, thr)
    snapped_idx = snap_cut_indices(frames_cpu, raw_idx, window=8, min_delta=6.0)
    raw_set = set(raw_idx)
    moved = sum(1 for s in snapped_idx if s not in raw_set)

    cuts = [i / max(fps, 1e-6) for i in snapped_idx]
    del frames_cpu

    # Frame-perfect pass: snap every candidate onto the true source frame via
    # ffmpeg scene-delta peaks at native pts (48x27 grid is only an estimate).
    try:
        cuts = refine_cuts_at_source(video_path, cuts)
    except Exception as exc:  # noqa: BLE001
        log.warning("source-res cut refine failed (%s) — keeping snapped cuts", exc)

    if on_progress:
        on_progress(
            {
                "phase": "finalize",
                "progress": 0.96,
                "stage": (
                    f"Finishing · {len(cuts)} scenes"
                    + (f" · snapped {moved}" if moved else "")
                ),
                "frames_done": n_frames,
                "frames_total": n_frames,
                "cuts_found": len(cuts),
                "device": device,
            }
        )

    log.info(
        "APP transnet GPU cuts=%s snapped_moves=%s frames=%s device=%s mem=%s video=%s",
        len(cuts),
        moved,
        n_frames,
        model.device,
        _mem(),
        video_path.name,
    )
    return cuts


def _probe_video_meta(video_path: Path) -> tuple[float, int, int, float]:
    """Return (fps, width, height, duration_sec) via ffprobe. Best-effort."""
    fps = 24.0
    width, height = 1920, 1080
    duration = 0.0
    try:
        probe = subprocess.run(
            [
                "ffprobe",
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=width,height,r_frame_rate,duration:format=duration",
                "-of",
                "json",
                str(video_path),
            ],
            capture_output=True,
            text=True,
            check=False,
            timeout=60,
        )
        if probe.returncode == 0 and probe.stdout:
            import json

            data = json.loads(probe.stdout)
            streams = data.get("streams") or []
            fmt = data.get("format") or {}
            if streams:
                st = streams[0]
                width = int(st.get("width") or width)
                height = int(st.get("height") or height)
                rate = str(st.get("r_frame_rate") or "").strip()
                if "/" in rate:
                    a, b = rate.split("/", 1)
                    fps = float(a) / max(float(b), 1e-6)
                elif rate:
                    fps = float(rate)
                try:
                    duration = float(st.get("duration") or 0.0)
                except (TypeError, ValueError):
                    duration = 0.0
            if duration <= 0:
                try:
                    duration = float(fmt.get("duration") or 0.0)
                except (TypeError, ValueError):
                    duration = 0.0
    except Exception as exc:  # noqa: BLE001
        log.debug("ffprobe meta fallback: %s", exc)
    return float(fps), max(2, width), max(2, height), max(0.0, duration)


def _even_downscale(width: int, height: int, max_w: int = 480) -> tuple[int, int]:
    """Even WxH for ffmpeg scale; keep aspect, cap width."""
    w = max(2, int(width))
    h = max(2, int(height))
    if w > max_w:
        h = max(2, int(round(h * (max_w / w))))
        w = max_w
    w -= w % 2
    h -= h % 2
    return max(2, w), max(2, h)


def _make_scenedetect_detector(
    *,
    use_adaptive: bool,
    threshold: float,
    adaptive_threshold: float,
) -> Any:
    from scenedetect.detectors import AdaptiveDetector, ContentDetector

    if use_adaptive:
        return AdaptiveDetector(adaptive_threshold=float(adaptive_threshold))
    return ContentDetector(threshold=float(threshold))


def _detect_cuts_ffmpeg_pipe(
    video_path: Path,
    *,
    threshold: float = 27.0,
    use_adaptive: bool = False,
    adaptive_threshold: float = 3.0,
    on_progress: CutProgressCb | None = None,
    use_hwaccel: bool = True,
) -> list[float]:
    """
    Stream downscaled BGR frames from ffmpeg into AdaptiveDetector/ContentDetector.

    Hardware decode (CUDA / VideoToolbox) when available — detector itself stays
    on CPU (HSV). Avoids loading 4K into OpenCV for the adaptive path.
    """
    import numpy as np
    from scenedetect.common import FrameTimecode

    from autoscene.export import ensure_ffmpeg
    from autoscene.ffmpeg_encode import hwaccel_input_args

    fps, src_w, src_h, duration = _probe_video_meta(video_path)
    out_w, out_h = _even_downscale(src_w, src_h, max_w=480)
    frame_bytes = out_w * out_h * 3
    frames_total = int(duration * fps) if duration > 0 and fps > 0 else 0
    hw = hwaccel_input_args(zero_copy=False) if use_hwaccel else []
    if use_hwaccel and not hw:
        raise RuntimeError("no ffmpeg hwaccel available")
    hw_joined = " ".join(hw).lower()
    if "cuda" in hw_joined:
        device = "cuda"
        decode_label = "CUDA decode"
    elif hw:
        device = "cpu"
        decode_label = "HW decode"
    else:
        device = "cpu"
        decode_label = "CPU"

    if on_progress:
        on_progress(
            {
                "phase": "extract",
                "progress": 0.05,
                "stage": f"Scanning scenes ({decode_label})…",
                "frames_done": 0,
                "frames_total": frames_total,
                "cuts_found": 0,
                "device": device,
            }
        )

    ffmpeg = ensure_ffmpeg()
    cmd = [
        ffmpeg,
        "-hide_banner",
        "-loglevel",
        "error",
        *hw,
        "-i",
        str(video_path),
        "-an",
        "-sn",
        "-vf",
        f"scale={out_w}:{out_h}",
        "-f",
        "rawvideo",
        "-pix_fmt",
        "bgr24",
        "pipe:1",
    ]
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        bufsize=frame_bytes * 8,
    )
    assert proc.stdout is not None

    detector = _make_scenedetect_detector(
        use_adaptive=use_adaptive,
        threshold=threshold,
        adaptive_threshold=adaptive_threshold,
    )
    cut_secs: list[float] = []
    frame_i = 0
    last_emit = 0.0
    last_tc: FrameTimecode | None = None

    def _emit(*, force: bool = False, phase: str = "infer") -> None:
        nonlocal last_emit
        if on_progress is None:
            return
        now = time.monotonic()
        if not force and now - last_emit < 0.35:
            return
        last_emit = now
        total = frames_total
        done = frame_i
        if total > 0:
            frac = 0.05 + 0.90 * min(1.0, done / total)
        else:
            frac = 0.08 + min(0.85, done / 50_000.0)
        on_progress(
            {
                "phase": phase,
                "progress": float(max(0.05, min(0.95, frac))),
                "stage": (
                    f"Scanning scenes · {decode_label} · frame {done}/{total}"
                    if total > 0
                    else f"Scanning scenes · {decode_label} · frame {done}"
                ),
                "frames_done": int(done),
                "frames_total": int(total),
                "cuts_found": len(cut_secs) + 1,
                "device": device,
            }
        )

    try:
        while True:
            buf = proc.stdout.read(frame_bytes)
            if not buf or len(buf) < frame_bytes:
                break
            frame = np.frombuffer(buf, dtype=np.uint8).reshape((out_h, out_w, 3))
            tc = FrameTimecode(timecode=frame_i, fps=fps)
            last_tc = tc
            for hit in detector.process_frame(tc, frame):
                cut_secs.append(float(hit.get_seconds()))
            frame_i += 1
            _emit()
    finally:
        try:
            proc.stdout.close()
        except Exception:  # noqa: BLE001
            pass
        try:
            err_b = proc.stderr.read() if proc.stderr else b""
        except Exception:  # noqa: BLE001
            err_b = b""
        rc = proc.wait(timeout=120)
        if frame_i == 0:
            err = (err_b or b"").decode("utf-8", errors="replace")[-500:]
            raise RuntimeError(
                f"ffmpeg scene-scan pipe produced no frames (rc={rc}): {err}"
            )
        if rc != 0 and frame_i < max(1, frames_total // 4 if frames_total else 1):
            err = (err_b or b"").decode("utf-8", errors="replace")[-500:]
            raise RuntimeError(f"ffmpeg scene-scan pipe failed (rc={rc}): {err}")

    if last_tc is not None:
        for hit in detector.post_process(last_tc):
            cut_secs.append(float(hit.get_seconds()))

    # Unique sorted cuts (detector can re-emit near boundaries).
    cuts = sorted({round(c, 4) for c in cut_secs if c > 0.05})
    scene_count = len(cuts) + 1
    if on_progress:
        on_progress(
            {
                "phase": "finalize",
                "progress": 0.96,
                "stage": f"Finishing · {scene_count} scenes",
                "frames_done": frame_i,
                "frames_total": frames_total or frame_i,
                "cuts_found": scene_count,
                "device": device,
            }
        )
    log.info(
        "scenedetect-pipe cuts=%s scenes=%s frames=%s %sx%s hw=%s adaptive=%s video=%s",
        len(cuts),
        scene_count,
        frame_i,
        out_w,
        out_h,
        bool(hw),
        use_adaptive,
        video_path.name,
    )
    return cuts


def detect_cuts_scenedetect(
    video_path: Path,
    *,
    threshold: float = 27.0,
    use_adaptive: bool = False,
    adaptive_threshold: float = 3.0,
    on_progress: CutProgressCb | None = None,
) -> list[float]:
    """
    PySceneDetect shot boundaries (no torch).

    Prefer ffmpeg hardware decode → downscale → AdaptiveDetector/ContentDetector
    on CPU. Fall back to OpenCV ``open_video`` if the pipe fails.

    use_adaptive=True runs AdaptiveDetector (survives fast camera motion that
    fires ContentDetector false cuts). Sensitivity maps to
    ``AdaptiveDetector(adaptive_threshold=…)``; ContentDetector uses ``threshold``.

    Emits live ``frames_done`` / ``frames_total`` / ``cuts_found`` via on_progress.
    """
    from autoscene.ffmpeg_encode import hwaccel_input_args

    hw = hwaccel_input_args(zero_copy=False)
    if hw:
        try:
            return _detect_cuts_ffmpeg_pipe(
                video_path,
                threshold=threshold,
                use_adaptive=use_adaptive,
                adaptive_threshold=adaptive_threshold,
                on_progress=on_progress,
                use_hwaccel=True,
            )
        except Exception as exc:  # noqa: BLE001
            log.warning(
                "Hardware-decode scene scan failed (%s); falling back to CPU open_video",
                exc,
            )

    import scenedetect.scene_manager as sd_sm
    from scenedetect import SceneManager, open_video

    class _ProgressBridge:
        """tqdm-compatible sink → AutoScene cut progress events."""

        def __init__(self, total: int = 0, desc: str = "", **_kwargs: Any) -> None:
            self.total = max(0, int(total or 0))
            self.n = 0
            self.desc = desc or ""
            self._cuts = 0
            self._last_emit = 0.0
            self._emit(force=True)

        def update(self, n: int = 1) -> None:
            self.n += int(n)
            if self.total > 0:
                self.n = min(self.n, self.total)
            self._emit()

        def set_description(self, desc: str | None = None, refresh: bool = True) -> None:
            del refresh
            if desc:
                self.desc = str(desc)
                # Description looks like "Detected: 12" (cut count) from PySceneDetect.
                digits = "".join(ch for ch in self.desc if ch.isdigit())
                if digits:
                    try:
                        # UI "scenes" ≈ cuts + 1 (opening shot before first cut).
                        self._cuts = int(digits) + 1
                    except ValueError:
                        pass
            self._emit()

        def close(self) -> None:
            self._emit(force=True)

        def __enter__(self) -> "_ProgressBridge":
            return self

        def __exit__(self, *_exc: object) -> None:
            self.close()

        def _emit(self, *, force: bool = False) -> None:
            if on_progress is None:
                return
            now = time.monotonic()
            if not force and now - self._last_emit < 0.35:
                return
            self._last_emit = now
            total = self.total
            done = self.n
            if total > 0:
                frac = 0.05 + 0.90 * (done / total)
            else:
                frac = 0.08
            on_progress(
                {
                    "phase": "infer",
                    "progress": float(max(0.05, min(0.95, frac))),
                    "stage": (
                        f"Scanning scenes · frame {done}/{total}"
                        if total > 0
                        else "Scanning scenes (CPU)…"
                    ),
                    "frames_done": int(done),
                    "frames_total": int(total),
                    "cuts_found": int(self._cuts),
                    "device": "cpu",
                }
            )

    if on_progress:
        on_progress(
            {
                "phase": "extract",
                "progress": 0.05,
                "stage": "Scanning scenes (CPU)…",
                "frames_done": 0,
                "frames_total": 0,
                "cuts_found": 0,
                "device": "cpu",
            }
        )

    video = open_video(str(video_path))
    manager = SceneManager()
    manager.add_detector(
        _make_scenedetect_detector(
            use_adaptive=use_adaptive,
            threshold=threshold,
            adaptive_threshold=adaptive_threshold,
        )
    )

    prev_tqdm = sd_sm.tqdm
    try:
        if on_progress is not None:
            sd_sm.tqdm = _ProgressBridge  # type: ignore[misc, assignment]
        manager.detect_scenes(video, show_progress=on_progress is not None)
    finally:
        sd_sm.tqdm = prev_tqdm

    scenes = manager.get_scene_list()

    # Scene starts after the first are the cut points.
    cuts = [float(start.get_seconds()) for start, _end in scenes[1:]]
    scene_count = len(scenes)

    if on_progress:
        # frames_total from video duration when available
        try:
            frames_total = int(video.duration.frame_num) if video.duration else 0
        except Exception:  # noqa: BLE001
            frames_total = 0
        on_progress(
            {
                "phase": "finalize",
                "progress": 0.96,
                "stage": f"Finishing · {scene_count} scenes",
                "frames_done": frames_total or 0,
                "frames_total": frames_total or 0,
                "cuts_found": scene_count,
                "device": "cpu",
            }
        )
    log.info(
        "scenedetect cuts=%s scenes=%s threshold=%.1f adaptive=%s video=%s",
        len(cuts),
        scene_count,
        threshold,
        use_adaptive,
        video_path.name,
    )
    return cuts


def detect_shot_spans(
    video_path: Path,
    duration_sec: float,
    *,
    mode: SceneCutMode = "adaptive",
    adaptive_threshold: float = 3.0,
    transnet_threshold: float = 0.5,
    on_progress: CutProgressCb | None = None,
) -> tuple[list[ShotSpan], str]:
    """
    Shot boundaries for pack context.

    - ``adaptive`` (default): PySceneDetect AdaptiveDetector — HSV frame-delta
      vs rolling average. No Torch. Best simple path for hard cuts.
    - ``transnet``: TransNetV2 + photometric snap (GPU). Fail hard if missing.
    - ``auto``: try TransNet, else AdaptiveDetector.
    """
    path = Path(video_path)
    if not path.is_file():
        return [ShotSpan(0.0, max(0.01, float(duration_sec)))], "none"

    if mode not in ("adaptive", "transnet", "auto"):
        mode = "adaptive"

    # ContentDetector/Adaptive scale is unrelated to TransNet's 0-1 probability;
    # 27.0 is ContentDetector's documented default. Sensitivity knob (1..10,
    # default 3): higher → more cuts (lower threshold).
    sens = max(0.5, float(adaptive_threshold))
    sd_thr = float(max(6.0, min(81.0, 27.0 * 3.0 / sens)))

    def _scenedetect(*, use_adaptive: bool) -> tuple[list[ShotSpan], str]:
        cuts = detect_cuts_scenedetect(
            path,
            threshold=sd_thr,
            use_adaptive=use_adaptive,
            adaptive_threshold=sens,
            on_progress=on_progress,
        )
        used = "scenedetect-adaptive" if use_adaptive else "scenedetect"
        return _shots_from_cuts(cuts, duration_sec), used

    if mode == "adaptive":
        try:
            return _scenedetect(use_adaptive=True)
        except Exception as exc:  # noqa: BLE001
            log.error("PySceneDetect adaptive cut failed: %s", exc)
            raise RuntimeError(f"Scene cut detection failed: {exc}") from exc

    thr = _transnet_threshold_for_mode(mode, transnet_threshold)
    try:
        cuts = detect_cuts_transnet(path, threshold=thr, on_progress=on_progress)
        return _shots_from_cuts(cuts, duration_sec), "transnet-snap"
    except Exception as exc:  # noqa: BLE001
        if mode == "transnet":
            log.error("GPU scene cut failed: %s", exc)
            raise
        log.warning("TransNetV2 unavailable (%s) — falling back to AdaptiveDetector", exc)

    try:
        return _scenedetect(use_adaptive=True)
    except Exception as exc:  # noqa: BLE001
        log.error("PySceneDetect scene cut failed: %s", exc)
        raise RuntimeError(f"Scene cut detection failed: {exc}") from exc


def shot_index_for_time(shots: list[ShotSpan], t: float) -> int:
    if not shots:
        return 0
    for i, sh in enumerate(shots):
        if sh.start_sec <= t < sh.end_sec or (
            i == len(shots) - 1 and abs(t - sh.end_sec) < 1e-3
        ):
            return i
        if t < sh.start_sec:
            return max(0, i - 1)
    return len(shots) - 1
