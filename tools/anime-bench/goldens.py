"""Bake the parity goldens the Rust side checks itself against: head crops of
the shipped CC-BY anime stills as raw RGB bytes plus their CPU CCIP vectors,
and each still's detector boxes.

    uv run python -I goldens.py <stills_dir> <models_dir> <out_dir>

Outputs, per crop i:
    ccip_<i>.rgb   raw RGB24 bytes of the 384x384 crop
    ccip_<i>.vec   the model's raw output, little-endian f32
    golden.json    still file, source box, crop rect, model ids + sha256,
                   input tensor names/shapes, CLIP normalisation
    dets_golden.json  per still, per detector variant: [x0,y0,x1,y1,score]
                   frame-share boxes for the IoU parity check

The crop rule (kept exact so Rust can reproduce it): a square of side
1.5 * max(box_w, box_h) on the box centre, shifted — never scaled — to lie
inside the image, its side capped by the smaller image side; then PIL
bilinear to 384x384 and CLIP mean/std.
"""
import hashlib
import json
import sys
from pathlib import Path

import numpy as np
import onnxruntime as ort
from PIL import Image

sys.path.insert(0, str(Path(__file__).parent))  # `python -I` drops the script folder
from yolo import Detector  # noqa: E402

CLIP_MEAN = np.array([0.48145466, 0.4578275, 0.40821073], np.float32)
CLIP_STD = np.array([0.26862954, 0.26130258, 0.27577711], np.float32)
SIDE = 384
DET_VARIANTS = ["anime_head_detection/head_detect_v2.0_s", "anime_person_detection/person_detect_v1.3_s"]
CCIP = "ccip_onnx/ccip-caformer-24-randaug-pruned/model_feat.onnx"


def sha256(path: Path) -> str:
    h = hashlib.sha256()
    h.update(path.read_bytes())
    return h.hexdigest()


def crop_rect(box: list[float], w: int, h: int) -> tuple[int, int, int]:
    """x, y, side of the x1.5 head square, per the module docstring."""
    cx, cy = (box[0] + box[2]) / 2 * w, (box[1] + box[3]) / 2 * h
    side = min(1.5 * max((box[2] - box[0]) * w, (box[3] - box[1]) * h), min(w, h))
    x = min(max(round(cx - side / 2), 0), w - int(round(side)))
    y = min(max(round(cy - side / 2), 0), h - int(round(side)))
    return x, y, int(round(side))


def main() -> None:
    stills_dir, models, out = map(Path, sys.argv[1:4])
    out.mkdir(parents=True, exist_ok=True)
    stills = sorted(stills_dir.glob("anime-*.webp"))
    assert stills, f"no anime-*.webp in {stills_dir}"

    det = Detector.load(str(models / DET_VARIANTS[0] / "model.onnx"), "head_detect_v2.0_s")
    ccip = ort.InferenceSession(str(models / CCIP), providers=["CPUExecutionProvider"])
    inp = ccip.get_inputs()[0]

    meta, det_golden = {"crops": [], "detectors": {}}, {}
    for f in stills:
        img = Image.open(f).convert("RGB")
        w, h = img.size
        dets = det.detect(img, "letterbox640", conf=0.10)
        det_golden[f.name] = {"size": [w, h], "boxes": np.round(dets, 6).tolist()}
        for j, b in enumerate(dets[:2]):
            x, y, side = crop_rect(b[:4].tolist(), w, h)
            crop = img.crop((x, y, x + side, y + side)).resize((SIDE, SIDE), Image.BILINEAR)
            x_in = (np.asarray(crop, np.float32) / 255.0 - CLIP_MEAN) / CLIP_STD
            vec = ccip.run(None, {inp.name: x_in.transpose(2, 0, 1)[None]})[0][0]
            i = len(meta["crops"])
            (out / f"ccip_{i}.rgb").write_bytes(crop.tobytes())
            (out / f"ccip_{i}.vec").write_bytes(vec.astype("<f4").tobytes())
            meta["crops"].append({"still": f.name, "box": [round(float(v), 6) for v in b[:4]], "score": round(float(b[4]), 5),
                                  "rect": [x, y, side], "rgb": f"ccip_{i}.rgb", "vec": f"ccip_{i}.vec", "dim": int(vec.shape[0]),
                                  "norm": round(float(np.linalg.norm(vec)), 4)})

    for folder in DET_VARIANTS:
        d = Detector.load(str(models / folder / "model.onnx"), folder.split("/")[1])
        meta["detectors"][d.name] = {"model": folder + "/model.onnx", "sha256": sha256(models / folder / "model.onnx"),
                                     "input": d.session.get_inputs()[0].name,
                                     "shape": d.session.get_inputs()[0].shape}
    meta["ccip"] = {"model": CCIP, "sha256": sha256(models / CCIP),
                    "input": inp.name, "shape": inp.shape, "mean": CLIP_MEAN.tolist(), "std": CLIP_STD.tolist()}
    json.dump(meta, open(out / "golden.json", "w"), indent=1)
    json.dump({"model": "letterbox640", "stills": det_golden}, open(out / "dets_golden.json", "w"), indent=1)
    print(json.dumps({"crops": len(meta["crops"]), "stills": len(stills),
                      "boxes": {k: len(v["boxes"]) for k, v in det_golden.items()}}))


if __name__ == "__main__":
    main()
