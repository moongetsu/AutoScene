"""AnimeCV face backend: EfficientDet detect + OML character embeds."""

from __future__ import annotations

import logging

import cv2
import numpy as np
from PIL import Image

from autoscene.faces.base import FaceDetection
from autoscene.models_vault import (
    ANIMECV_ENCODERS,
    ensure_animecv_runtime,
    patch_torchvision_pretrained,
    resolve_animecv_model_id,
)

log = logging.getLogger(__name__)

# Process-wide cache: ResNet-152 + EfficientDet load is heavy; reuse across scans.
_PREPARED: dict[tuple[str, int, float], tuple[object, object, str]] = {}


def _patch_numpy_legacy_aliases() -> None:
    """AnimeCV still uses removed np.int / np.float aliases (NumPy >= 1.24)."""
    for name, typ in (("int", int), ("float", float), ("bool", bool)):
        if not hasattr(np, name):
            setattr(np, name, typ)


def _is_junk_anime_box(
    frame_bgr: np.ndarray,
    x1: int,
    y1: int,
    x2: int,
    y2: int,
) -> bool:
    """
    Drop title-card / logo false faces: odd aspect, huge plate, dark glyph plate.
    Conservative: only rejects clear junk, not normal dark cel faces.
    """
    h, w = frame_bgr.shape[:2]
    bw, bh = max(1, x2 - x1), max(1, y2 - y1)
    aspect = bw / float(bh)
    if aspect < 0.45 or aspect > 2.15:
        return True
    area_frac = (bw * bh) / float(max(1, w * h))
    if area_frac > 0.38:
        return True

    crop = frame_bgr[y1:y2, x1:x2]
    if crop.size == 0:
        return True
    gray = cv2.cvtColor(crop, cv2.COLOR_BGR2GRAY)
    mean = float(gray.mean())
    std = float(gray.std())
    if std < 7.0:
        return True
    # Near-black plate + busy edges + sparse bright glyphs → logo / title text
    bright_frac = float(np.mean(gray > 175))
    if mean < 32.0 and bright_frac < 0.28:
        edges = cv2.Canny(gray, 70, 150)
        edge_frac = float(np.mean(edges > 0))
        if edge_frac > 0.11:
            return True
    return False


class AnimeCVBackend:
    """2D / anime cast path via kosuke1701/AnimeCV."""

    name = "animecv"

    def __init__(
        self,
        model_name: str = "cel_canon",
        min_face_size: int = 32,
        det_coef: int = 2,
        score_threshold: float = 0.58,
    ) -> None:
        self.model_id = resolve_animecv_model_id(model_name)
        self.min_face_size = min_face_size
        self.det_coef = det_coef if det_coef in (0, 2) else 2
        self.score_threshold = score_threshold
        self._detector = None
        self._encoder = None
        self._device = "cuda"

    def prepare(self) -> None:
        from autoscene.models_vault import ensure_torch_dll_path

        _patch_numpy_legacy_aliases()
        ensure_torch_dll_path()
        import torch
        from animecv.object_detection import FaceDetector_EfficientDet
        from animecv.general import OML_ImageFolder_Pretrained
        from animecv.module import ImageBBEncoder
        from torchvision import transforms

        cache_key = (self.model_id, self.det_coef, self.score_threshold)
        cached = _PREPARED.get(cache_key)
        if cached is not None:
            self._detector, self._encoder, self._device = cached  # type: ignore[assignment]
            log.info(
                "AnimeCV prepare cache hit model=%s coef=%s",
                self.model_id,
                self.det_coef,
            )
            return

        patch_torchvision_pretrained()
        log.info(
            "AnimeCV prepare begin model_id=%s label=%s coef=%s "
            "torch=%s cuda=%s",
            self.model_id,
            ANIMECV_ENCODERS[self.model_id]["label"],
            self.det_coef,
            torch.__version__,
            torch.cuda.is_available(),
        )
        model_dir = ensure_animecv_runtime(
            self.model_id,
            auto_pip=True,
            on_progress=lambda m, p: log.info("AnimeCV ensure %.0f%% %s", p * 100, m),
        )
        if not torch.cuda.is_available():
            raise RuntimeError(
                "AnimeCV requires PyTorch CUDA. Settings → Models → Install all, "
                "then restart the API."
            )
        self._device = "cuda"
        log.info(
            "AnimeCV weights ready dir=%s cuda_device=%s",
            model_dir,
            torch.cuda.get_device_name(0),
        )
        self._detector = FaceDetector_EfficientDet(
            coef=self.det_coef, use_cuda=True
        )
        log.info("AnimeCV detector ready coef=%s", self.det_coef)

        conf_input = 224
        try:
            import json

            with open(model_dir / "conf.json", encoding="utf-8") as h:
                conf, _params = json.load(h)
            conf_input = int(conf.get("input_size", 224))
        except Exception:  # noqa: BLE001
            pass

        torch_model = OML_ImageFolder_Pretrained(str(model_dir))
        post = transforms.Compose(
            [
                transforms.Resize((conf_input, conf_input)),
                transforms.ToTensor(),
                transforms.Normalize(
                    mean=[0.485, 0.456, 0.406],
                    std=[0.229, 0.224, 0.225],
                ),
            ]
        )
        self._encoder = ImageBBEncoder(torch_model, post_trans=post, scale=1.15)
        self._encoder.to(self._device)
        _PREPARED[cache_key] = (self._detector, self._encoder, self._device)
        log.info(
            "AnimeCV encoder ready input=%s device=%s model=%s",
            conf_input,
            self._device,
            self.model_id,
        )

    def detect(self, frame_bgr: np.ndarray, timestamp_sec: float) -> list[FaceDetection]:
        if self._detector is None or self._encoder is None:
            self.prepare()
        assert self._detector is not None and self._encoder is not None
        _patch_numpy_legacy_aliases()

        # BGR → RGB PIL (AnimeCV expects PIL)
        rgb = cv2.cvtColor(frame_bgr, cv2.COLOR_BGR2RGB)
        pil = Image.fromarray(rgb)
        bbox_lists = self._detector.detect(
            [pil], threshold=self.score_threshold, iou_threshold=0.5
        )
        boxes = bbox_lists[0] if bbox_lists else []
        if not boxes:
            return []

        # Filter tiny / junk (logo, title-card) faces before encode
        kept: list[dict] = []
        h, w = frame_bgr.shape[:2]
        for b in boxes:
            x1, y1, x2, y2 = [int(v) for v in b["coordinates"]]
            x1, y1 = max(0, x1), max(0, y1)
            x2, y2 = min(w - 1, x2), min(h - 1, y2)
            bw, bh = x2 - x1, y2 - y1
            if bw < self.min_face_size or bh < self.min_face_size:
                continue
            if _is_junk_anime_box(frame_bgr, x1, y1, x2, y2):
                continue
            kept.append(
                {
                    "coordinates": [x1, y1, x2, y2],
                    "score": float(b.get("score", 1.0)),
                }
            )
        if not kept:
            return []

        embs, _i_img, i_bbox = self._encoder.encode([pil], [kept])
        embs_np = embs.detach().cpu().numpy()

        out: list[FaceDetection] = []
        for row, bi in enumerate(i_bbox):
            box = kept[bi]
            x1, y1, x2, y2 = box["coordinates"]
            emb = np.asarray(embs_np[row], dtype=np.float32)
            norm = float(np.linalg.norm(emb))
            if norm > 0:
                emb = emb / norm
            out.append(
                FaceDetection.with_thumb(
                    timestamp_sec=timestamp_sec,
                    bbox=(x1, y1, x2, y2),
                    embedding=emb,
                    det_score=float(box["score"]),
                    frame_bgr=frame_bgr,
                )
            )
        return out
