"""FFmpeg encode helpers for AutoScene export.

Modes:
  quality  — GPU NVENC delivery encode (balanced CQ). Default when re-encode needed.
  lossless — archive / master: visually transparent NVENC matched to source bitrate
             (not giant bit-exact FFV1/x264). Keeps original look without ballooning size.
  copy     — stream copy (H.264-safe sources only).
"""

from __future__ import annotations

import json
import logging
import shutil
import subprocess
from dataclasses import dataclass
from functools import lru_cache
from pathlib import Path
from typing import Literal

log = logging.getLogger(__name__)

ExportMode = Literal["copy", "quality", "lossless"]
VideoCodecPref = Literal["h264_nvenc", "hevc_nvenc", "av1_nvenc", "auto"]

_SAFE_AUDIO_COPY = frozenset(
    {"aac", "mp3", "opus", "vorbis", "flac", "pcm_s16le", "pcm_s24le"}
)


def ensure_ffmpeg() -> str:
    """
    Resolve ffmpeg binary.

    Pin channel (first hit wins):
      1) AUTOSCENE_FFMPEG — absolute path to ffmpeg(.exe)
      2) data_dir/tools/ffmpeg/ffmpeg(.exe) — optional bundled build
      3) PATH — system ffmpeg

    If AUTOSCENE_FFMPEG_SHA256 is set, the chosen binary must match.
    Requires a usable build with libx264 (preview/proxy path).
    """
    import os

    from autoscene.config import settings

    candidates: list[Path] = []
    env = (os.environ.get("AUTOSCENE_FFMPEG") or "").strip()
    if env:
        candidates.append(Path(env))
    settings.ensure_dirs()
    tools = settings.data_dir / "tools" / "ffmpeg"
    candidates.append(tools / ("ffmpeg.exe" if os.name == "nt" else "ffmpeg"))
    which = shutil.which("ffmpeg")
    if which:
        candidates.append(Path(which))

    exe: Path | None = None
    for c in candidates:
        if c.is_file():
            exe = c.resolve()
            break
    if exe is None:
        raise RuntimeError(
            "ffmpeg not found. Install ffmpeg on PATH, or set AUTOSCENE_FFMPEG, "
            "or place a build at data_dir/tools/ffmpeg/."
        )

    expect_sha = (os.environ.get("AUTOSCENE_FFMPEG_SHA256") or "").strip().lower()
    if expect_sha:
        import hashlib

        h = hashlib.sha256()
        with exe.open("rb") as f:
            while True:
                chunk = f.read(1024 * 1024)
                if not chunk:
                    break
                h.update(chunk)
        got = h.hexdigest()
        if got != expect_sha:
            raise RuntimeError(
                f"ffmpeg SHA256 mismatch for {exe}: got {got}, expected {expect_sha}"
            )

    try:
        ver = subprocess.run(
            [str(exe), "-hide_banner", "-version"],
            check=False,
            capture_output=True,
            text=True,
            timeout=8,
        )
    except OSError as exc:
        raise RuntimeError(f"Cannot run ffmpeg at {exe}") from exc
    banner = ((ver.stdout or "") + (ver.stderr or "")).strip()
    head = banner.splitlines()[0] if banner else str(exe)
    log.info("ffmpeg pin: %s (%s)", exe, head)

    try:
        enc = subprocess.run(
            [str(exe), "-hide_banner", "-encoders"],
            check=False,
            capture_output=True,
            text=True,
            timeout=12,
        )
    except OSError as exc:
        raise RuntimeError(f"Cannot probe ffmpeg encoders at {exe}") from exc
    enc_text = (enc.stdout or "") + (enc.stderr or "")
    if "libx264" not in enc_text:
        raise RuntimeError(
            f"ffmpeg at {exe} is missing libx264 (needed for preview/proxy). "
            "Install a full build (e.g. gyan.dev / BtbN essentials+)."
        )
    return str(exe)


def _ffmpeg() -> str:
    return ensure_ffmpeg()


@dataclass(frozen=True)
class SourceEncodeProfile:
    width: int
    height: int
    video_bitrate: int | None
    audio_bitrate: int | None
    fps: float | None
    pix_fmt: str | None
    vcodec: str | None
    acodec: str | None
    bits_per_raw_sample: int | None
    color_range: str | None
    color_space: str | None
    color_transfer: str | None
    color_primaries: str | None


@lru_cache(maxsize=1)
def _encoder_list() -> str:
    ffmpeg = _ffmpeg()
    try:
        proc = subprocess.run(
            [ffmpeg, "-hide_banner", "-encoders"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return ""
    return (proc.stdout or "") + (proc.stderr or "")


@lru_cache(maxsize=1)
def cuda_hwaccel_available() -> bool:
    ffmpeg = _ffmpeg()
    try:
        proc = subprocess.run(
            [ffmpeg, "-hide_banner", "-hwaccels"],
            check=False,
            capture_output=True,
            text=True,
        )
    except OSError:
        return False
    text = (proc.stdout or "") + (proc.stderr or "")
    return "cuda" in text.lower()


def nvenc_h264_available() -> bool:
    return "h264_nvenc" in _encoder_list()


def nvenc_hevc_available() -> bool:
    return "hevc_nvenc" in _encoder_list()


def nvenc_av1_available() -> bool:
    return "av1_nvenc" in _encoder_list()


def ffv1_available() -> bool:
    return "ffv1" in _encoder_list()


def available_export_codecs() -> dict[str, bool]:
    return {
        "h264_nvenc": nvenc_h264_available(),
        "hevc_nvenc": nvenc_hevc_available(),
        "av1_nvenc": nvenc_av1_available(),
        "cuda_hwaccel": cuda_hwaccel_available(),
    }


def probe_encode_profile(video_path: Path) -> SourceEncodeProfile:
    from autoscene.video_proxy import _ensure_ffprobe

    ffprobe = _ensure_ffprobe()
    proc = subprocess.run(
        [
            ffprobe,
            "-v",
            "quiet",
            "-print_format",
            "json",
            "-show_format",
            "-show_streams",
            str(video_path),
        ],
        check=False,
        capture_output=True,
        text=True,
    )
    if proc.returncode != 0:
        raise RuntimeError((proc.stderr or "ffprobe failed")[-300:])
    data = json.loads(proc.stdout or "{}")

    width = 0
    height = 0
    v_bitrate: int | None = None
    a_bitrate: int | None = None
    fps: float | None = None
    pix_fmt: str | None = None
    vcodec: str | None = None
    acodec: str | None = None
    bits: int | None = None
    color_range: str | None = None
    color_space: str | None = None
    color_transfer: str | None = None
    color_primaries: str | None = None

    for stream in data.get("streams") or []:
        kind = stream.get("codec_type")
        if kind == "video" and vcodec is None:
            vcodec = stream.get("codec_name")
            width = int(stream.get("width") or 0)
            height = int(stream.get("height") or 0)
            pix_fmt = stream.get("pix_fmt")
            bits = _parse_int(stream.get("bits_per_raw_sample"))
            color_range = stream.get("color_range")
            color_space = stream.get("color_space")
            color_transfer = stream.get("color_transfer")
            color_primaries = stream.get("color_primaries")
            br = _parse_int(stream.get("bit_rate"))
            if br:
                v_bitrate = br
            fps = _parse_fps(stream.get("avg_frame_rate")) or _parse_fps(
                stream.get("r_frame_rate")
            )
        elif kind == "audio" and acodec is None:
            acodec = stream.get("codec_name")
            a_bitrate = _parse_int(stream.get("bit_rate"))

    fmt = data.get("format") or {}
    fmt_br = _parse_int(fmt.get("bit_rate"))
    if v_bitrate is None and fmt_br:
        v_bitrate = max(0, fmt_br - (a_bitrate or 0)) or fmt_br

    return SourceEncodeProfile(
        width=width,
        height=height,
        video_bitrate=v_bitrate,
        audio_bitrate=a_bitrate,
        fps=fps,
        pix_fmt=pix_fmt,
        vcodec=vcodec,
        acodec=acodec,
        bits_per_raw_sample=bits,
        color_range=color_range if color_range not in (None, "unknown") else None,
        color_space=color_space if color_space not in (None, "unknown") else None,
        color_transfer=color_transfer if color_transfer not in (None, "unknown") else None,
        color_primaries=color_primaries
        if color_primaries not in (None, "unknown")
        else None,
    )


def bit_depth(profile: SourceEncodeProfile) -> int:
    if profile.bits_per_raw_sample and profile.bits_per_raw_sample >= 8:
        return int(profile.bits_per_raw_sample)
    pix = profile.pix_fmt or ""
    if "12" in pix:
        return 12
    if "10" in pix:
        return 10
    if "9" in pix:
        return 9
    return 8


def preserve_pix_fmt(profile: SourceEncodeProfile) -> str:
    if profile.pix_fmt:
        return profile.pix_fmt
    return "yuv420p10le" if bit_depth(profile) >= 10 else "yuv420p"


def container_of(path: Path | str) -> str:
    return Path(path).suffix.lower().lstrip(".") or "mp4"


def part_suffix_for_container(container: str) -> str:
    c = container.lower().lstrip(".")
    if c == "mkv":
        return ".mkv"
    if c == "mov":
        return ".mov"
    return ".mp4"


def video_filter_args(profile: SourceEncodeProfile) -> list[str]:
    """Only force even size when needed; never downscale."""
    if profile.width > 0 and profile.height > 0:
        if profile.width % 2 == 0 and profile.height % 2 == 0:
            return []
    return ["-vf", "scale=trunc(iw/2)*2:trunc(ih/2)*2"]


def color_tag_args(profile: SourceEncodeProfile) -> list[str]:
    args: list[str] = []
    if profile.color_range:
        args.extend(["-color_range", profile.color_range])
    if profile.color_space:
        args.extend(["-colorspace", profile.color_space])
    if profile.color_transfer:
        args.extend(["-color_trc", profile.color_transfer])
    if profile.color_primaries:
        args.extend(["-color_primaries", profile.color_primaries])
    return args


def hwaccel_input_args(*, zero_copy: bool = False) -> list[str]:
    """
    CUDA decode when available.
    zero_copy=True keeps frames on GPU for NVENC (no software filters allowed).
    """
    if not cuda_hwaccel_available():
        return []
    if zero_copy:
        return ["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]
    return ["-hwaccel", "cuda"]


def prefer_output_path(
    output_path: Path,
    *,
    mode: ExportMode,
    codec: VideoCodecPref = "h264_nvenc",
) -> Path:
    """Fill missing/invalid suffix. Respect explicit .mp4/.mov/.mkv."""
    if output_path.suffix.lower() in {".mp4", ".mov", ".mkv"}:
        return output_path
    if mode in {"quality", "lossless"} and codec in {"hevc_nvenc", "av1_nvenc"}:
        return output_path.with_suffix(".mkv")
    return output_path.with_suffix(".mp4")


# Back-compat alias
def prefer_lossless_output_path(output_path: Path, *, reencode: bool) -> Path:
    return prefer_output_path(
        output_path,
        mode="lossless" if reencode else "copy",
        codec="h264_nvenc",
    )


def _any_nvenc_available() -> bool:
    return nvenc_h264_available() or nvenc_hevc_available() or nvenc_av1_available()


def _estimate_source_video_bitrate(profile: SourceEncodeProfile) -> int | None:
    """Bits/sec for archive bitrate match. Prefer probe; else resolution heuristic."""
    if profile.video_bitrate and profile.video_bitrate >= 400_000:
        return int(profile.video_bitrate)
    w, h = int(profile.width or 0), int(profile.height or 0)
    if w <= 0 or h <= 0:
        return None
    fps = float(profile.fps or 24.0)
    # ~0.07–0.10 bpp web/master heuristic; clamp to sane delivery range.
    est = int(w * h * max(12.0, fps) * 0.08)
    return max(4_000_000, min(est, 80_000_000))


def _nvenc_vbr_args(
    encoder: str,
    *,
    cq: int,
    pix: str,
    color: list[str],
    preset: str = "p5",
    bitrate: int | None = None,
    maxrate: int | None = None,
) -> list[str]:
    args = [
        "-c:v",
        encoder,
        "-preset",
        preset,
        "-tune",
        "hq",
        "-rc",
        "vbr",
        "-cq",
        str(cq),
        "-spatial_aq",
        "1",
        "-temporal_aq",
        "1",
        "-rc-lookahead",
        "32",
        "-pix_fmt",
        pix,
        *color,
    ]
    if bitrate and bitrate > 0:
        mr = int(maxrate or bitrate)
        args.extend(
            [
                "-b:v",
                str(bitrate),
                "-maxrate",
                str(mr),
                "-bufsize",
                str(mr * 2),
            ]
        )
    else:
        args.extend(["-b:v", "0"])
    return args


def _nvenc_quality_args(
    encoder: str,
    *,
    cq: int,
    pix: str,
    color: list[str],
) -> list[str]:
    return _nvenc_vbr_args(
        encoder, cq=cq, pix=pix, color=color, preset="p5", bitrate=None
    )


def _codec_order(pref: VideoCodecPref, profile: SourceEncodeProfile) -> list[str]:
    auto = (
        "hevc_nvenc"
        if (profile.vcodec or "").startswith("hevc")
        else "h264_nvenc"
    )
    key = auto if pref == "auto" else pref
    return {
        "h264_nvenc": ["h264_nvenc", "hevc_nvenc", "av1_nvenc"],
        "hevc_nvenc": ["hevc_nvenc", "h264_nvenc", "av1_nvenc"],
        "av1_nvenc": ["av1_nvenc", "hevc_nvenc", "h264_nvenc"],
    }.get(key, ["h264_nvenc", "hevc_nvenc", "av1_nvenc"])


def quality_video_encode_candidates(
    profile: SourceEncodeProfile,
    *,
    container: str,
    codec: VideoCodecPref = "h264_nvenc",
) -> list[tuple[list[str], str]]:
    """GPU NVENC delivery encodes. No CPU when NVENC exists."""
    c = container.lower().lstrip(".")
    depth = bit_depth(profile)
    color = color_tag_args(profile)
    candidates: list[tuple[list[str], str]] = []

    for enc in _codec_order(codec, profile):
        if enc == "h264_nvenc" and nvenc_h264_available():
            candidates.append(
                (
                    _nvenc_quality_args(
                        "h264_nvenc", cq=17, pix="yuv420p", color=color
                    ),
                    "h264_nvenc-quality",
                )
            )
        elif enc == "hevc_nvenc" and nvenc_hevc_available():
            if depth >= 10:
                candidates.append(
                    (
                        _nvenc_quality_args(
                            "hevc_nvenc", cq=18, pix="p010le", color=color
                        ),
                        "hevc_nvenc-quality-10",
                    )
                )
            candidates.append(
                (
                    _nvenc_quality_args(
                        "hevc_nvenc", cq=19, pix="yuv420p", color=color
                    ),
                    "hevc_nvenc-quality",
                )
            )
        elif enc == "av1_nvenc" and nvenc_av1_available():
            candidates.append(
                (
                    _nvenc_quality_args(
                        "av1_nvenc", cq=20, pix="yuv420p", color=color
                    ),
                    "av1_nvenc-quality",
                )
            )

    if not candidates and not _any_nvenc_available():
        # Last resort only when this machine has zero NVENC.
        candidates.append(
            (
                [
                    "-c:v",
                    "libx264",
                    "-preset",
                    "medium",
                    "-crf",
                    "17",
                    "-pix_fmt",
                    "yuv420p",
                    *color,
                ],
                "libx264-quality",
            )
        )

    log.info(
        "Quality encode candidates container=%s depth=%s codec=%s -> %s",
        c,
        depth,
        codec,
        [label for _, label in candidates],
    )
    return candidates


def lossless_video_encode_candidates(
    profile: SourceEncodeProfile,
    *,
    container: str,
    codec: VideoCodecPref = "auto",
) -> list[tuple[list[str], str]]:
    """
    Archive / master encode: keep source look (no mush, no extra noise), GPU NVENC,
    size near the source — not giant bit-exact FFV1/x264.
    """
    c = container.lower().lstrip(".")
    depth = bit_depth(profile)
    color = color_tag_args(profile)
    candidates: list[tuple[list[str], str]] = []

    # Archive on 10-bit HEVC masters: prefer HEVC NVENC so we do not crush to 8-bit H.264.
    effective: VideoCodecPref = codec
    if (
        depth >= 10
        and (profile.vcodec or "").startswith("hevc")
        and nvenc_hevc_available()
        and codec in {"h264_nvenc", "auto"}
    ):
        effective = "hevc_nvenc"

    src_br = _estimate_source_video_bitrate(profile)
    # Slight headroom vs source so re-encode does not add banding; hard cap vs balloon.
    if src_br:
        target = int(src_br * 1.06)
        maxrate = int(min(src_br * 1.22, max(src_br * 1.06, 100_000_000)))
    else:
        target = None
        maxrate = None

    for enc in _codec_order(effective, profile):
        if enc == "hevc_nvenc" and nvenc_hevc_available():
            if depth >= 10:
                candidates.append(
                    (
                        _nvenc_vbr_args(
                            "hevc_nvenc",
                            cq=15,
                            pix="p010le",
                            color=color,
                            preset="p7",
                            bitrate=target,
                            maxrate=maxrate,
                        ),
                        "hevc_nvenc-archive-10",
                    )
                )
            candidates.append(
                (
                    _nvenc_vbr_args(
                        "hevc_nvenc",
                        cq=16,
                        pix="yuv420p",
                        color=color,
                        preset="p7",
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "hevc_nvenc-archive",
                )
            )
        elif enc == "h264_nvenc" and nvenc_h264_available():
            candidates.append(
                (
                    _nvenc_vbr_args(
                        "h264_nvenc",
                        cq=14,
                        pix="yuv420p",
                        color=color,
                        preset="p7",
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "h264_nvenc-archive",
                )
            )
        elif enc == "av1_nvenc" and nvenc_av1_available():
            candidates.append(
                (
                    _nvenc_vbr_args(
                        "av1_nvenc",
                        cq=17,
                        pix="yuv420p",
                        color=color,
                        preset="p7",
                        bitrate=target,
                        maxrate=maxrate,
                    ),
                    "av1_nvenc-archive",
                )
            )

    if not candidates:
        raise RuntimeError(
            "Archive export needs NVIDIA NVENC. No GPU encoder found in ffmpeg."
        )

    log.info(
        "Archive encode candidates container=%s depth=%s codec=%s br=%s -> %s",
        c,
        depth,
        codec,
        src_br,
        [label for _, label in candidates],
    )
    return candidates


def audio_encode_args(
    profile: SourceEncodeProfile,
    *,
    container: str,
    mode: ExportMode = "quality",
) -> list[str]:
    """Copy when possible; otherwise AAC (avoid FLAC/PCM size blow-up on archive)."""
    del container  # container only mattered for old lossless PCM/FLAC paths
    if profile.acodec in _SAFE_AUDIO_COPY:
        return ["-c:a", "copy"]
    # quality + archive: transparent-enough AAC, not giant PCM/FLAC
    if mode in {"quality", "lossless"}:
        return ["-c:a", "aac", "-b:a", "320k"]
    return ["-c:a", "aac", "-b:a", "320k"]


def video_encode_args(profile: SourceEncodeProfile) -> tuple[list[str], str]:
    cands = quality_video_encode_candidates(profile, container="mp4")
    return cands[0]


def scale_filter_keep_res() -> str:
    return "scale=trunc(iw/2)*2:trunc(ih/2)*2"


def _parse_int(value: object) -> int | None:
    if value is None or value == "N/A":
        return None
    try:
        n = int(str(value))
    except (TypeError, ValueError):
        return None
    return n if n > 0 else None


def _parse_fps(value: object) -> float | None:
    if not value or value == "N/A" or value == "0/0":
        return None
    text = str(value)
    try:
        if "/" in text:
            num, den = text.split("/", 1)
            d = float(den)
            if d <= 0:
                return None
            return float(num) / d
        return float(text)
    except (TypeError, ValueError, ZeroDivisionError):
        return None
