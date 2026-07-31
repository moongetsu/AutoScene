from __future__ import annotations

import logging
import os
import sys
from dataclasses import dataclass
from pathlib import Path
from typing import Any

log = logging.getLogger(__name__)

# Prefer stable cuDNN classic path over frontend heuristics that break SCRFD depthwise convs.
os.environ.setdefault("CUDA_MODULE_LOADING", "LAZY")


def is_darwin() -> bool:
    """macOS has no onnxruntime-gpu build; acceleration comes from CoreML."""
    return sys.platform == "darwin"


@dataclass
class GpuInfo:
    available: bool
    name: str | None
    provider: str
    detail: str


def _nvidia_wheel_bin_dirs() -> list[Path]:
    """Locate bin dirs from pip nvidia-* packages (incl. cu13/bin/x86_64)."""
    try:
        import nvidia  # type: ignore
    except ImportError:
        return []

    root = Path(nvidia.__path__[0]).resolve()
    bins: list[Path] = []
    seen: set[str] = set()
    for candidate in root.rglob("*"):
        if not candidate.is_dir():
            continue
        name = candidate.name.lower()
        if name not in {"bin", "x86_64", "lib"}:
            continue
        if not any(candidate.glob("*.dll")) and name != "bin":
            continue
        if name == "bin" or any(candidate.glob("*.dll")):
            key = str(candidate).lower()
            if key not in seen:
                seen.add(key)
                bins.append(candidate)

    def sort_key(p: Path) -> tuple[int, int, str]:
        s = str(p).lower()
        # Prefer dirs that contain the cuDNN engines DLLs ORT needs for Conv
        has_engines = 0 if (p / "cudnn_engines_precompiled64_9.dll").exists() else 1
        has_cu13 = 0 if "cu13" in s else 1
        return (has_engines, has_cu13, s)

    bins.sort(key=sort_key)
    return bins


def _torch_ships_cudnn() -> Path | None:
    """If torch bundles cuDNN, prefer it over nvidia-cudnn wheels (mixed load → WinError 127)."""
    try:
        import importlib.util

        spec = importlib.util.find_spec("torch")
    except Exception:  # noqa: BLE001
        return None
    if not spec:
        return None
    roots: list[Path] = []
    if spec.submodule_search_locations:
        roots.extend(Path(p) for p in spec.submodule_search_locations)
    if spec.origin:
        roots.append(Path(spec.origin).resolve().parent)
    for root in roots:
        lib = root / "lib"
        if (lib / "cudnn_adv64_9.dll").exists():
            return lib
    return None


def ensure_cuda_dll_path() -> list[str]:
    """Add NVIDIA pip wheel DLL dirs to process PATH / add_dll_directory."""
    if is_darwin():
        return []
    added: list[str] = []
    torch_cudnn = _torch_ships_cudnn()
    if torch_cudnn is not None:
        s = str(torch_cudnn.resolve())
        if hasattr(os, "add_dll_directory"):
            try:
                os.add_dll_directory(s)
            except OSError:
                pass
        path = os.environ.get("PATH", "")
        parts = [p for p in path.split(os.pathsep) if p and p.lower() != s.lower()]
        os.environ["PATH"] = s + os.pathsep + os.pathsep.join(parts)
        added.append(s)
        log.info("Using torch bundled cuDNN: %s", s)

    for d in _nvidia_wheel_bin_dirs():
        s = str(d)
        low = s.lower().replace("/", "\\")
        # Mixing nvidia-cudnn wheels with torch's bundled cuDNN breaks torch import
        # (WinError 127 on cudnn_adv64_9.dll). Keep nvidia\cu13 for ORT CUDA 13.
        if torch_cudnn is not None and "\\nvidia\\cudnn\\" in low:
            continue
        if hasattr(os, "add_dll_directory"):
            try:
                os.add_dll_directory(s)
            except OSError:
                pass
        path = os.environ.get("PATH", "")
        parts = path.split(os.pathsep)
        if all(p.lower() != s.lower() for p in parts):
            os.environ["PATH"] = s + os.pathsep + path
        added.append(s)
    if added:
        log.info("CUDA DLL dirs: %s", added[:8])
        engines = next(
            (
                a
                for a in added
                if Path(a).joinpath("cudnn_engines_precompiled64_9.dll").exists()
            ),
            None,
        )
        if engines:
            log.info("cuDNN engines DLL dir: %s", engines)
        else:
            log.warning(
                "cudnn_engines_precompiled64_9.dll not found on PATH — Conv nodes may fail"
            )
    return added


def cuda_provider_options(device_id: int = 0) -> dict[str, Any]:
    """
    CUDA EP options that avoid fragile cuDNN frontend Conv plans.

    DEFAULT algo search is more compatible with SCRFD / antelope depthwise convs
    that otherwise hit CUDNN_FE failure 11 (CUDNN_BACKEND_API_FAILED).
    """
    return {
        "device_id": device_id,
        "arena_extend_strategy": "kNextPowerOfTwo",
        "cudnn_conv_algo_search": "DEFAULT",
        "do_copy_in_default_stream": True,
    }


def cuda_providers(device_id: int = 0) -> list[tuple[str, dict[str, Any]]]:
    """GPU-only provider list with options (no CPU)."""
    return [("CUDAExecutionProvider", cuda_provider_options(device_id))]


def coreml_provider_options() -> dict[str, Any]:
    """
    CoreML EP options (Apple Silicon).

    MLProgram + ALL compute units lets the ANE/GPU take what it can. SCRFD runs
    with dynamic input shapes, so static shapes must stay off or the whole graph
    is rejected back to CPU.
    """
    return {
        "ModelFormat": "MLProgram",
        "MLComputeUnits": "ALL",
        "RequireStaticInputShapes": "0",
        "EnableOnSubgraphs": "0",
    }


def coreml_providers() -> list[tuple[str, dict[str, Any]]]:
    """
    CoreML first, CPU behind it.

    Unlike CUDA this is not GPU-only by design: CoreML EP has partial operator
    coverage and unsupported nodes need somewhere to land. Dropping CPU here
    makes model load fail outright instead of running the graph split.
    """
    return [
        ("CoreMLExecutionProvider", coreml_provider_options()),
        ("CPUExecutionProvider", {}),
    ]


def cpu_providers() -> list[tuple[str, dict[str, Any]]]:
    return [("CPUExecutionProvider", {})]


def accel_providers(device_id: int = 0) -> list[tuple[str, dict[str, Any]]]:
    """Platform-appropriate provider list: CoreML on macOS, CUDA elsewhere."""
    if is_darwin():
        return coreml_providers()
    return cuda_providers(device_id)


def detect_gpu() -> GpuInfo:
    """Detect a working execution provider for this platform."""
    if is_darwin():
        return _detect_darwin()
    return _detect_cuda()


def _detect_darwin() -> GpuInfo:
    """
    macOS: prefer CoreML, accept CPU.

    CPU is a legitimate outcome here — unlike the CUDA path, where falling back
    silently means the user thinks they are on GPU. The provider string reports
    what actually happened.
    """
    name = _apple_chip_name()
    try:
        import onnxruntime as ort
    except Exception as exc:  # noqa: BLE001
        return GpuInfo(
            available=False,
            name=name,
            provider="",
            detail=f"onnxruntime unavailable: {exc}",
        )

    providers = ort.get_available_providers()
    if "CoreMLExecutionProvider" in providers:
        ok, detail = _probe_session(coreml_providers(), "CoreMLExecutionProvider")
        if ok:
            return GpuInfo(
                available=True,
                name=name,
                provider="CoreMLExecutionProvider",
                detail=detail,
            )
        log.warning("CoreML EP unusable, falling back to CPU: %s", detail)

    ok, detail = _probe_session(cpu_providers(), "CPUExecutionProvider")
    if ok:
        return GpuInfo(
            available=True,
            name=name,
            provider="CPUExecutionProvider",
            detail=f"CoreML unavailable — running on CPU (slower). {detail}",
        )
    return GpuInfo(available=False, name=name, provider="", detail=detail)


def _detect_cuda() -> GpuInfo:
    """Require working CUDAExecutionProvider. No CPU fallback."""
    ensure_cuda_dll_path()
    name = _nvidia_name()
    try:
        import onnxruntime as ort
    except Exception as exc:  # noqa: BLE001
        return GpuInfo(
            available=False,
            name=name,
            provider="",
            detail=f"onnxruntime unavailable: {exc}",
        )

    providers = ort.get_available_providers()
    if "CUDAExecutionProvider" not in providers:
        return GpuInfo(
            available=False,
            name=name,
            provider="",
            detail=f"CUDAExecutionProvider missing. Providers={providers}. Install onnxruntime-gpu.",
        )

    ok, detail = _probe_cuda_session()
    if ok:
        return GpuInfo(
            available=True,
            name=name,
            provider="CUDAExecutionProvider",
            detail=detail,
        )
    return GpuInfo(
        available=False,
        name=name,
        provider="",
        detail=detail,
    )


def require_providers() -> list[tuple[str, dict[str, Any]]]:
    """Return the platform's provider list with options, or raise."""
    info = detect_gpu()
    if not info.available:
        if is_darwin():
            raise RuntimeError(
                "AutoScene could not create any onnxruntime session on macOS. "
                f"{info.detail} Chip={info.name or 'unknown'}. "
                "Install: pip install onnxruntime (not onnxruntime-gpu — it has no macOS build)."
            )
        raise RuntimeError(
            "AutoScene requires working NVIDIA CUDA via onnxruntime-gpu (CPU disabled). "
            f"{info.detail} GPU={info.name or 'unknown'}. "
            "Install: pip install nvidia-cublas nvidia-cudnn-cu13 nvidia-cuda-runtime "
            "nvidia-cuda-nvrtc nvidia-nvjitlink — plus current NVIDIA drivers."
        )
    if is_darwin():
        # Honour what actually probed clean rather than re-offering CoreML.
        if info.provider == "CPUExecutionProvider":
            return cpu_providers()
        return coreml_providers()
    return cuda_providers()


# Back-compat alias — Windows/Linux callers still read as "require CUDA".
def require_cuda_providers() -> list[tuple[str, dict[str, Any]]]:
    return require_providers()


def ort_providers() -> list[tuple[str, dict[str, Any]]]:
    return require_providers()


def _probe_session(
    providers: list[tuple[str, dict[str, Any]]],
    expected: str,
) -> tuple[bool, str]:
    """Build and run a trivial graph to prove the provider really loads."""
    try:
        import numpy as np
        from onnx import TensorProto, helper
        import onnxruntime as ort

        x = helper.make_tensor_value_info("x", TensorProto.FLOAT, [1])
        y = helper.make_tensor_value_info("y", TensorProto.FLOAT, [1])
        node = helper.make_node("Identity", ["x"], ["y"])
        graph = helper.make_graph([node], "probe", [x], [y])
        model = helper.make_model(graph, opset_imports=[helper.make_opsetid("", 13)])
        sess = ort.InferenceSession(
            model.SerializeToString(),
            providers=providers,
        )
        active = sess.get_providers()
        if not active or active[0] != expected:
            return False, f"Requested {expected} but session providers={active}"
        _ = sess.run(None, {"x": np.array([1.0], dtype=np.float32)})
        return True, f"{expected} session ok"
    except Exception as exc:  # noqa: BLE001
        return False, f"{expected} session failed: {exc}"


def _probe_cuda_session() -> tuple[bool, str]:
    ok, detail = _probe_session(cuda_providers(), "CUDAExecutionProvider")
    if ok:
        return True, "CUDAExecutionProvider session ok (cudnn_conv_algo_search=DEFAULT)"
    return ok, detail


def _apple_chip_name() -> str | None:
    try:
        import subprocess

        out = subprocess.check_output(
            ["sysctl", "-n", "machdep.cpu.brand_string"],
            text=True,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
        return out.strip() or None
    except Exception:  # noqa: BLE001
        return None


def _nvidia_name() -> str | None:
    try:
        import subprocess

        out = subprocess.check_output(
            ["nvidia-smi", "--query-gpu=name", "--format=csv,noheader"],
            text=True,
            stderr=subprocess.DEVNULL,
            timeout=5,
        )
        line = out.strip().splitlines()[0].strip()
        return line or None
    except Exception:  # noqa: BLE001
        return None


def is_cudnn_conv_failure(exc: BaseException) -> bool:
    msg = str(exc).upper()
    return any(
        token in msg
        for token in (
            "CUDNN_FE",
            "CUDNN_BACKEND_API_FAILED",
            "CUDNN FRONTEND",
            "CONV_12_DW",
            "RUNNING CONV NODE",
        )
    )
