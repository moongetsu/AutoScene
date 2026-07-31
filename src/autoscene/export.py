from __future__ import annotations

import logging
import os
import shutil
import subprocess
import tempfile
import threading
from collections.abc import Callable
from concurrent.futures import ThreadPoolExecutor, as_completed
from pathlib import Path

from autoscene.cluster import Segment
from autoscene.ffmpeg_encode import (
    ExportMode,
    SourceEncodeProfile,
    VideoCodecPref,
    audio_encode_args,
    bit_depth,
    container_of,
    hwaccel_input_args,
    lossless_video_encode_candidates,
    part_suffix_for_container,
    prefer_output_path,
    prefer_lossless_output_path,
    probe_encode_profile,
    quality_video_encode_candidates,
    video_filter_args,
)

log = logging.getLogger(__name__)

_SAFE_VIDEO = frozenset({"h264", "vp8", "vp9", "av1"})
_SAFE_AUDIO = frozenset({"aac", "mp3", "opus", "vorbis", "flac"})

ProgressCb = Callable[[float, str], None]
CancelCb = Callable[[], bool]

# Concurrent NVENC jobs — leave headroom for scene-frame ffmpeg during export
_QUALITY_WORKERS = 2

# Active export ffmpeg PIDs (for cancel)
_active_ffmpeg: set[int] = set()
_active_ffmpeg_lock = threading.Lock()


class ExportCancelled(Exception):
    """Raised when user cancels the current export job."""


def kill_active_export_ffmpeg() -> int:
    """Terminate ffmpeg processes started by export helpers."""
    import os
    import signal

    with _active_ffmpeg_lock:
        pids = list(_active_ffmpeg)
    killed = 0
    for pid in pids:
        try:
            if os.name == "nt":
                subprocess.run(
                    ["taskkill", "/PID", str(pid), "/F", "/T"],
                    check=False,
                    capture_output=True,
                )
            else:
                os.kill(pid, signal.SIGTERM)
            killed += 1
        except OSError:
            pass
    with _active_ffmpeg_lock:
        _active_ffmpeg.clear()
    return killed


def ensure_ffmpeg() -> str:
    from autoscene.ffmpeg_encode import ensure_ffmpeg as _ensure

    return _ensure()


def source_needs_reencode(video_path: Path) -> bool:
    """True when stream-copy cuts would stutter (HEVC, E-AC3, …)."""
    from autoscene.video_proxy import probe_codecs

    try:
        vcodec, acodec = probe_codecs(video_path)
    except Exception:  # noqa: BLE001
        return True
    if vcodec not in _SAFE_VIDEO:
        return True
    if acodec is not None and acodec not in _SAFE_AUDIO:
        return True
    return False


def resolve_export_mode(
    video_path: Path,
    *,
    export_mode: str | None = None,
    reencode: bool = False,
) -> ExportMode:
    """
    Map API inputs to a concrete mode.
    Default when re-encode is required: quality (GPU), not bit-exact lossless.
    """
    raw = (export_mode or "").strip().lower()
    if raw in {"copy", "quality", "lossless"}:
        mode: ExportMode = raw  # type: ignore[assignment]
    elif reencode:
        mode = "quality"
    else:
        mode = "copy"

    if mode == "copy" and source_needs_reencode(video_path):
        return "quality"
    return mode


def _clip_weight(seg: Segment) -> float:
    return max(0.25, float(seg.end_sec) - float(seg.start_sec))


def export_person_segments(
    video_path: Path,
    segments: list[Segment],
    out_dir: Path,
    reencode: bool = False,
    *,
    export_mode: str | None = None,
    video_codec: VideoCodecPref = "h264_nvenc",
) -> list[Path]:
    ffmpeg = ensure_ffmpeg()
    out_dir.mkdir(parents=True, exist_ok=True)
    mode = resolve_export_mode(video_path, export_mode=export_mode, reencode=reencode)
    profile = probe_encode_profile(video_path) if mode != "copy" else None
    out = prefer_output_path(
        out_dir / "clip",
        mode=mode,
        codec=video_codec,
    )
    container = container_of(out)
    suffix = part_suffix_for_container(container)
    written: list[Path] = []
    for i, seg in enumerate(segments, start=1):
        part = out_dir / f"clip_{i:03d}{suffix}"
        _extract_segment_clip(
            ffmpeg,
            video_path,
            seg,
            part,
            mode=mode,
            profile=profile,
            container=container,
            video_codec=video_codec,
        )
        written.append(part)
    return written


def _encode_candidates(
    profile: SourceEncodeProfile,
    *,
    mode: ExportMode,
    container: str,
    video_codec: VideoCodecPref,
) -> list[tuple[list[str], str]]:
    if mode == "lossless":
        return lossless_video_encode_candidates(
            profile, container=container, codec=video_codec
        )
    return quality_video_encode_candidates(
        profile, container=container, codec=video_codec
    )


def _extract_segment_clip(
    ffmpeg: str,
    video_path: Path,
    seg: Segment,
    out: Path,
    *,
    mode: ExportMode,
    profile: SourceEncodeProfile | None = None,
    container: str = "mp4",
    video_codec: VideoCodecPref = "h264_nvenc",
    should_cancel: CancelCb | None = None,
) -> None:
    # Exact segment bounds — do NOT pad. ±0.15s (~3–4 frames @24fps) made
    # every clip look both early and late even when cut detection was correct.
    start = max(0.0, float(seg.start_sec))
    end = max(start + 0.04, float(seg.end_sec))
    duration = max(0.04, end - start)

    def run(cmd: list[str], *, label: str) -> None:
        _run_ffmpeg(cmd, label=label, should_cancel=should_cancel)

    if mode == "copy":
        cmd = [
            ffmpeg,
            "-y",
            "-ss",
            f"{start:.3f}",
            "-i",
            str(video_path),
            "-t",
            f"{duration:.3f}",
            "-c",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
            str(out),
        ]
        run(cmd, label="segment-copy")
        return

    if profile is None:
        profile = probe_encode_profile(video_path)
    container = container_of(out) or container
    a_args = audio_encode_args(profile, container=container, mode=mode)
    vf = video_filter_args(profile)
    use_input_seek = mode in {"quality", "lossless"}
    # Zero-copy CUDA→NVENC cannot convert 10-bit→8-bit. Stripping -pix_fmt on
    # Main10 HEVC makes h264_nvenc hang (0% GPU) instead of failing cleanly.
    depth = bit_depth(profile)
    zero_copy = mode in {"quality", "lossless"} and not vf and depth <= 8
    hw = hwaccel_input_args(zero_copy=zero_copy)
    if use_input_seek:
        base = [
            ffmpeg,
            "-y",
            *hw,
            "-ss",
            f"{start:.3f}",
            "-i",
            str(video_path),
            "-t",
            f"{duration:.3f}",
            *vf,
        ]
    else:
        base = [
            ffmpeg,
            "-y",
            *hw,
            "-i",
            str(video_path),
            "-ss",
            f"{start:.3f}",
            "-t",
            f"{duration:.3f}",
            *vf,
        ]
    tail = [*a_args, "-movflags", "+faststart", str(out)]
    if container == "mkv":
        tail = [*a_args, str(out)]

    errors: list[str] = []
    candidates = _encode_candidates(
        profile, mode=mode, container=container, video_codec=video_codec
    )
    # Never silently slip to CPU when NVENC candidates exist.
    gpu_only = any("nvenc" in enc for _, enc in candidates)
    if gpu_only:
        candidates = [(a, e) for a, e in candidates if "nvenc" in e]
    if not candidates:
        raise RuntimeError(
            "No GPU (NVENC) encoder available for export. Check ffmpeg NVIDIA build."
        )

    def _try_candidates(base_cmd: list[str], *, tag: str) -> bool:
        for v_args, enc in candidates:
            enc_args = list(v_args)
            if zero_copy and tag == "zc" and "nvenc" in enc:
                stripped: list[str] = []
                skip = False
                for tok in enc_args:
                    if skip:
                        skip = False
                        continue
                    if tok == "-pix_fmt":
                        skip = True
                        continue
                    stripped.append(tok)
                enc_args = stripped
            try:
                run([*base_cmd, *enc_args, *tail], label=f"segment-{enc}-{tag}")
                log.info(
                    "Segment ok mode=%s encoder=%s tag=%s out=%s",
                    mode,
                    enc,
                    tag,
                    out.name,
                )
                return True
            except ExportCancelled:
                raise
            except subprocess.CalledProcessError as exc:
                err_tail = (exc.stderr or "")[-240:]
                errors.append(f"{enc}/{tag}: {err_tail}")
                log.warning("Encoder %s (%s) failed; trying next", enc, tag)
                try:
                    out.unlink(missing_ok=True)
                except OSError:
                    pass
        return False

    if _try_candidates(base, tag="zc" if zero_copy else "hw"):
        return

    if zero_copy:
        log.warning("Zero-copy NVENC failed; retrying CUDA decode -> sysmem (still GPU)")
        base_fb = [
            ffmpeg,
            "-y",
            *hwaccel_input_args(zero_copy=False),
            "-ss",
            f"{start:.3f}",
            "-i",
            str(video_path),
            "-t",
            f"{duration:.3f}",
            *vf,
        ]
        if _try_candidates(base_fb, tag="sys"):
            return

    raise RuntimeError(
        f"GPU encode failed for segment (mode={mode}). "
        "CPU fallback disabled. " + " | ".join(errors)[-1500:]
    )


def _concat_list_path(path: Path) -> str:
    normalized = path.resolve().as_posix().replace("'", "'\\''")
    return f"file '{normalized}'"


def export_merged_video(
    video_path: Path,
    segments: list[Segment],
    output_path: Path,
    reencode: bool = False,
    on_progress: ProgressCb | None = None,
    *,
    export_mode: str | None = None,
    video_codec: VideoCodecPref = "h264_nvenc",
    should_cancel: CancelCb | None = None,
) -> Path:
    if not segments:
        raise ValueError("No segments to export")

    def cancelled() -> bool:
        return bool(should_cancel and should_cancel())

    if cancelled():
        raise ExportCancelled()

    ffmpeg = ensure_ffmpeg()
    output_path = output_path.resolve()
    mode = resolve_export_mode(
        video_path, export_mode=export_mode, reencode=reencode
    )
    output_path = prefer_output_path(
        output_path, mode=mode, codec=video_codec
    )
    output_path.parent.mkdir(parents=True, exist_ok=True)

    profile = probe_encode_profile(video_path) if mode != "copy" else None
    container = container_of(output_path)
    suffix = part_suffix_for_container(container)

    weights = [_clip_weight(s) for s in segments]
    clip_total = sum(weights)
    concat_w = max(0.5, 0.02 * clip_total)
    grand = clip_total + concat_w
    done = 0.0
    n = len(segments)
    verb = {
        "lossless": "Archive GPU encode",
        "quality": "GPU encode",
        "copy": "Cutting",
    }.get(mode, "Encoding")

    def report(stage: str) -> None:
        frac = 0.0 if grand <= 0 else min(1.0, max(0.0, done / grand))
        if on_progress:
            on_progress(frac, stage)

    report(f"{verb} clip 1/{n}")

    temp_dir = Path(tempfile.mkdtemp(prefix="autoscene_merge_"))
    clip_paths: list[Path] = [temp_dir / f"part_{i:03d}{suffix}" for i in range(1, n + 1)]
    try:
        workers = _QUALITY_WORKERS if mode == "quality" else 1

        def _job(idx: int, seg: Segment, out: Path) -> int:
            if cancelled():
                raise ExportCancelled()
            _extract_segment_clip(
                ffmpeg,
                video_path,
                seg,
                out,
                mode=mode,
                profile=profile,
                container=container,
                video_codec=video_codec,
                should_cancel=should_cancel,
            )
            return idx

        if workers <= 1 or n == 1:
            for i, (seg, out) in enumerate(zip(segments, clip_paths), start=1):
                if cancelled():
                    raise ExportCancelled()
                report(f"{verb} clip {i}/{n}")
                _job(i, seg, out)
                done += weights[i - 1]
                if i < n:
                    report(f"{verb} clip {i + 1}/{n}")
                else:
                    report("Concatenating scenes")
        else:
            finished = 0
            report(f"{verb} clips 0/{n} (×{workers})")
            with ThreadPoolExecutor(max_workers=workers) as pool:
                futures = {
                    pool.submit(_job, i, seg, out): i
                    for i, (seg, out) in enumerate(
                        zip(segments, clip_paths), start=1
                    )
                }
                for fut in as_completed(futures):
                    if cancelled():
                        kill_active_export_ffmpeg()
                        raise ExportCancelled()
                    idx = fut.result()
                    finished += 1
                    done += weights[idx - 1]
                    if finished < n:
                        report(f"{verb} clips {finished}/{n}")
                    else:
                        report("Concatenating scenes")

        if cancelled():
            raise ExportCancelled()

        list_file = temp_dir / "concat.txt"
        list_file.write_text(
            "\n".join(_concat_list_path(p) for p in clip_paths),
            encoding="utf-8",
        )

        cmd = [
            ffmpeg,
            "-y",
            "-f",
            "concat",
            "-safe",
            "0",
            "-i",
            str(list_file),
            "-c",
            "copy",
        ]
        if container in {"mp4", "mov"}:
            cmd.extend(["-movflags", "+faststart"])
        cmd.append(str(output_path))
        _run_ffmpeg(cmd, label="concat-copy", should_cancel=should_cancel)
        done += concat_w
        report("Export complete")
    finally:
        shutil.rmtree(temp_dir, ignore_errors=True)

    return output_path


def _run_ffmpeg(
    cmd: list[str],
    *,
    label: str,
    should_cancel: CancelCb | None = None,
) -> None:
    """
    Run ffmpeg without deadlocking on full stderr pipes.

    Waiting on proc.wait() while PIPE buffers fill makes ffmpeg block on write
    (CPU=0 hang). Drain stdout/stderr on side threads while polling cancel.
    """
    import time

    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        text=True,
        encoding="utf-8",
        errors="replace",
    )
    with _active_ffmpeg_lock:
        _active_ffmpeg.add(proc.pid)

    chunks: dict[str, list[str]] = {"out": [], "err": []}

    def _drain(pipe, key: str) -> None:
        if pipe is None:
            return
        try:
            while True:
                line = pipe.readline()
                if line == "":
                    break
                bucket = chunks[key]
                bucket.append(line)
                # Cap memory on ultra-chatty progress spam
                if len(bucket) > 4000:
                    del bucket[:2000]
        except Exception:  # noqa: BLE001
            pass
        finally:
            try:
                pipe.close()
            except Exception:  # noqa: BLE001
                pass

    t_out = threading.Thread(target=_drain, args=(proc.stdout, "out"), daemon=True)
    t_err = threading.Thread(target=_drain, args=(proc.stderr, "err"), daemon=True)
    t_out.start()
    t_err.start()
    try:
        while proc.poll() is None:
            if should_cancel and should_cancel():
                proc.kill()
                try:
                    proc.wait(timeout=5)
                except subprocess.TimeoutExpired:
                    pass
                raise ExportCancelled()
            time.sleep(0.2)
        t_out.join(timeout=5)
        t_err.join(timeout=5)
    finally:
        with _active_ffmpeg_lock:
            _active_ffmpeg.discard(proc.pid)

    stdout = "".join(chunks["out"])
    stderr = "".join(chunks["err"])
    if proc.returncode == 0:
        return
    if should_cancel and should_cancel():
        raise ExportCancelled()
    tail = (stderr or "")[-800:].strip()
    log.error("ffmpeg %s failed code=%s: %s", label, proc.returncode, tail)
    raise subprocess.CalledProcessError(
        proc.returncode, cmd, output=stdout, stderr=stderr
    )


__all__ = [
    "ensure_ffmpeg",
    "source_needs_reencode",
    "resolve_export_mode",
    "export_person_segments",
    "export_merged_video",
    "prefer_lossless_output_path",
    "prefer_output_path",
    "ExportCancelled",
    "kill_active_export_ffmpeg",
]
