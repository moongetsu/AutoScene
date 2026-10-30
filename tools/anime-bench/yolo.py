"""Ultralytics-style YOLO ONNX detectors (deepghs anime heads, faces, persons) on CPU.

Accuracy only: the app runs the same graphs through ONNX Runtime from Rust with
static shapes, and every timing comes from there.
"""
from dataclasses import dataclass

import numpy as np
import onnxruntime as ort
from PIL import Image

INPUTS = {
    "letterbox640": (640, 640),
    "rect640": (640, 384),
    "rect960": (960, 544),
    "rect1280": (1280, 736),
}


@dataclass
class Detector:
    name: str
    session: ort.InferenceSession

    @classmethod
    def load(cls, path: str, name: str) -> "Detector":
        opts = ort.SessionOptions()
        opts.intra_op_num_threads = 8
        return cls(name, ort.InferenceSession(path, opts, providers=["CPUExecutionProvider"]))

    def detect(self, img: Image.Image, mode: str, conf: float = 0.05, iou: float = 0.5) -> np.ndarray:
        """Boxes as rows `x0, y0, x1, y1, score`, shares of the frame."""
        tw, th = INPUTS[mode]
        w, h = img.size
        s = min(tw / w, th / h)
        nw, nh = round(w * s), round(h * s)
        canvas = Image.new("RGB", (tw, th), (114, 114, 114))
        ox, oy = (tw - nw) // 2, (th - nh) // 2
        canvas.paste(img.convert("RGB").resize((nw, nh), Image.BILINEAR), (ox, oy))
        x = np.asarray(canvas, dtype=np.float32).transpose(2, 0, 1)[None] / 255.0
        out = self.session.run(None, {"images": x})[0][0]
        # Head graphs give [5, N], the older face/person exports [N, 5].
        if out.shape[0] < out.shape[1]:
            out = out.T
        out = out[out[:, 4] >= conf]
        if not len(out):
            return np.zeros((0, 5), np.float32)
        cx, cy, bw, bh, sc = out.T
        x0 = (cx - bw / 2 - ox) / s / w
        y0 = (cy - bh / 2 - oy) / s / h
        x1 = (cx + bw / 2 - ox) / s / w
        y1 = (cy + bh / 2 - oy) / s / h
        boxes = np.clip(np.stack([x0, y0, x1, y1], 1), 0, 1)
        keep = nms(boxes, sc, iou)
        return np.concatenate([boxes[keep], sc[keep, None]], 1).astype(np.float32)


def iou_matrix(a: np.ndarray, b: np.ndarray) -> np.ndarray:
    ix0 = np.maximum(a[:, None, 0], b[None, :, 0])
    iy0 = np.maximum(a[:, None, 1], b[None, :, 1])
    ix1 = np.minimum(a[:, None, 2], b[None, :, 2])
    iy1 = np.minimum(a[:, None, 3], b[None, :, 3])
    inter = np.clip(ix1 - ix0, 0, None) * np.clip(iy1 - iy0, 0, None)
    area = lambda r: (r[:, 2] - r[:, 0]) * (r[:, 3] - r[:, 1])
    return inter / (area(a)[:, None] + area(b)[None, :] - inter + 1e-12)


def nms(boxes: np.ndarray, scores: np.ndarray, iou: float) -> np.ndarray:
    order = np.argsort(-scores)
    keep = []
    while len(order):
        i = order[0]
        keep.append(i)
        if len(order) == 1:
            break
        rest = order[1:]
        order = rest[iou_matrix(boxes[i : i + 1], boxes[rest])[0] < iou]
    return np.array(keep, dtype=int)
