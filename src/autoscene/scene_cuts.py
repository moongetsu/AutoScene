"""Shot / transition boundary detection for character scene packs.

GPU-only: TransNetV2 on CUDA, then hard-cut photometric snap (±12 frames)
so instant cuts land on the true frame peak (TransNet alone is often early/late).
App path: api._scan_job → run_scene_cuts_phase → detect_shot_spans → detect_cuts_transnet.
"""

from __future__ import annotations

import logging
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


def require_torch_cuda() -> str:
    from autoscene.gpu import ensure_cuda_dll_path
    from autoscene.models_vault import ensure_torch_dll_path

    ensure_cuda_dll_path()
    ensure_torch_dll_path()
    try:
        import torch
    except Exception as exc:  # noqa: BLE001
        raise RuntimeError(
            "Scene tools are not installed. Open Settings → Packs, install required packs, "
            "then restart AutoScene."
        ) from exc
    if not torch.cuda.is_available():
        raise RuntimeError(
            "Graphics hardware is not ready for scene detection. "
            "Open Settings → Packs, install the required packs, then restart AutoScene."
        )
    log.info("scene cuts GPU torch=%s device=%s", torch.__version__, torch.cuda.get_device_name(0))
    return "cuda"


def transnet_available() -> bool:
    try:
        import transnetv2_pytorch  # noqa: F401

        return True
    except Exception:  # noqa: BLE001
        return False


def adaptive_available() -> bool:
    return transnet_available()


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


def _assert_model_on_cuda(model: Any) -> None:
    import torch

    if not torch.cuda.is_available():
        raise RuntimeError("Graphics not ready for scene detection.")
    p = next(model.parameters())
    if p.device.type != "cuda":
        raise RuntimeError("Scene pack is not on graphics hardware.")
    if str(getattr(model, "device", "")).startswith("cpu"):
        raise RuntimeError("Scene pack is not on graphics hardware.")


def _extract_frames_rgb_48x27(
    video_path: Path,
    *,
    on_progress: CutProgressCb | None = None,
) -> tuple[Any, float]:
    """Decode to TransNet 48x27 RGB. Prefer ffmpeg CUDA hwaccel."""
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
                "device": "cuda",
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
                "device": "cuda",
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
        log.warning("CUDA hwaccel extract failed; retry software decode")
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
                "device": "cuda",
            }
        )
    return frames, float(fps)


def detect_cuts_transnet(
    video_path: Path,
    *,
    threshold: float = 0.5,
    on_progress: CutProgressCb | None = None,
) -> list[float]:
    """TransNetV2 CUDA-only inference used by the app cut phase."""
    device = require_torch_cuda()
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
                "device": "cuda",
            }
        )

    model = TransNetV2(device=device)
    model.to(torch.device("cuda"))
    model.device = torch.device("cuda")
    model.eval()
    _assert_model_on_cuda(model)

    mem0 = torch.cuda.memory_allocated(0)
    log.info(
        "APP TransNetV2 device=%s cuda=%s threshold=%.3f mem0=%s video=%s",
        model.device,
        torch.cuda.get_device_name(0),
        threshold,
        mem0,
        video_path.name,
    )

    frames_np, fps = _extract_frames_rgb_48x27(video_path, on_progress=on_progress)
    n_frames = int(frames_np.shape[0])
    if n_frames < 2:
        return []

    # Keep CPU copy for hard-cut photometric snap after inference.
    frames_cpu = np.array(frames_np, copy=True)
    frames = torch.from_numpy(frames_cpu).to(
        device=torch.device("cuda"), non_blocking=True
    )
    del frames_np
    if frames.device.type != "cuda":
        raise RuntimeError(f"Frame tensor on {frames.device}, not cuda")
    torch.cuda.synchronize()
    mem1 = torch.cuda.memory_allocated(0)
    log.info("frames on cuda count=%s mem_alloc=%s (+%s)", n_frames, mem1, mem1 - mem0)
    if mem1 <= mem0 and n_frames > 100:
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
    if padded.device.type != "cuda":
        raise RuntimeError(f"Padded tensor on {padded.device}, not cuda")

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
        if batch.device.type != "cuda":
            raise RuntimeError(f"Infer batch on {batch.device}, not cuda")
        with torch.no_grad():
            inp = batch.unsqueeze(0)
            if inp.device.type != "cuda":
                raise RuntimeError(f"Infer input on {inp.device}, not cuda")
            single_frame_pred, _all_pred = model.predict_raw(inp)
            if single_frame_pred.device.type != "cuda":
                raise RuntimeError(
                    f"Model output on {single_frame_pred.device}, not cuda"
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
                    "device": "cuda",
                    "cuda_memory_alloc": int(torch.cuda.memory_allocated(0)),
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
                "device": "cuda",
            }
        )

    log.info(
        "APP transnet GPU cuts=%s snapped_moves=%s frames=%s device=%s mem=%s video=%s",
        len(cuts),
        moved,
        n_frames,
        model.device,
        torch.cuda.memory_allocated(0),
        video_path.name,
    )
    return cuts


def detect_shot_spans(
    video_path: Path,
    duration_sec: float,
    *,
    mode: SceneCutMode = "auto",
    adaptive_threshold: float = 3.0,
    transnet_threshold: float = 0.5,
    on_progress: CutProgressCb | None = None,
) -> tuple[list[ShotSpan], str]:
    """CUDA TransNet propose + hard-cut snap. `adaptive` = softer threshold."""
    del adaptive_threshold
    path = Path(video_path)
    if not path.is_file():
        return [ShotSpan(0.0, max(0.01, float(duration_sec)))], "none"

    if mode not in ("adaptive", "transnet", "auto"):
        mode = "auto"

    thr = _transnet_threshold_for_mode(mode, transnet_threshold)
    try:
        cuts = detect_cuts_transnet(path, threshold=thr, on_progress=on_progress)
    except Exception as exc:  # noqa: BLE001
        log.error("GPU scene cut failed: %s", exc)
        raise

    # Always snap-refined; label so project.json / UI show the real pipeline.
    used = "transnet-snap-soft" if mode == "adaptive" else "transnet-snap"
    return _shots_from_cuts(cuts, duration_sec), used


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
