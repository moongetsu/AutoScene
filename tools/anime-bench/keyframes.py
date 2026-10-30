"""Sample keyframes of an episode, run every detector variant on them, and
render labelling sheets of the merged candidates.

    uv run python -I keyframes.py <key> <media.mp4> <models_dir> <out_dir> [n=150] [seed=1]

Frames are decoded like the app's face pass (fit into 1280x720, sides a
multiple of 4) by frame index on the decoded grid.
"""
import json
import os
import random
import subprocess
import sys
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw, ImageFont

sys.path.insert(0, str(Path(__file__).parent))  # run with `python -I`, which drops the script folder
from yolo import Detector, iou_matrix  # noqa: E402

VARIANTS = [  # (model folder, input modes)
    ("anime_head_detection/head_detect_v2.0_n", ["letterbox640"]),
    ("anime_head_detection/head_detect_v2.0_s", ["letterbox640", "rect640", "rect960", "rect1280"]),
    ("anime_head_detection/head_detect_v2.0_m", ["letterbox640"]),
    ("anime_head_detection/head_detect_v2.0_l", ["letterbox640"]),
    ("anime_head_detection/head_detect_v2.0_x", ["letterbox640", "rect960"]),
    ("anime_head_detection/head_detect_v2.0_s_yv11", ["letterbox640"]),
    ("anime_head_detection/head_detect_v2.0_m_yv11", ["letterbox640"]),
    ("anime_face_detection/face_detect_v1.4_s", ["letterbox640"]),
    ("anime_person_detection/person_detect_v1.1_m", ["letterbox640"]),
    ("anime_person_detection/person_detect_v1.3_s", ["letterbox640"]),
    ("anime_halfbody_detection/halfbody_detect_v1.0_s", ["letterbox640"]),
]
COLORS = ["#ff3b30", "#34c759", "#0a84ff", "#ffcc00", "#bf5af2", "#ff9f0a", "#64d2ff", "#ff375f"]


def probe(path: str) -> tuple[int, int, int]:
    j = json.loads(subprocess.run(
        ["ffprobe", "-v", "error", "-select_streams", "v:0", "-count_packets", "-show_entries",
         "stream=width,height,nb_read_packets", "-of", "json", path], capture_output=True, text=True, check=True).stdout)
    s = j["streams"][0]
    return int(s["width"]), int(s["height"]), int(s["nb_read_packets"])


def size(w: int, h: int) -> tuple[int, int]:
    s = min(1280 / w, 720 / h, 1.0)
    return int(w * s) // 4 * 4, int(h * s) // 4 * 4


def extract(path: str, frames: list[int], out: Path) -> None:
    w, h = size(*probe(path)[:2])
    out.mkdir(parents=True, exist_ok=True)
    # A long select expression makes ffmpeg fail, so take the frames in chunks.
    for c in range(0, len(frames), 25):
        chunk = frames[c : c + 25]
        sel = "+".join(f"eq(n\\,{f})" for f in chunk)
        tmp = out / "tmp"
        tmp.mkdir(exist_ok=True)
        subprocess.run(["ffmpeg", "-v", "error", "-y", "-i", path, "-vf", f"select='{sel}',scale={w}:{h}:flags=bilinear",
                        "-fps_mode", "passthrough", "-q:v", "2", str(tmp / "%05d.jpg")], check=True)
        for i, f in enumerate(chunk):
            (tmp / f"{i + 1:05d}.jpg").rename(out / f"{f}.jpg")
        tmp.rmdir()


def merge_heads(dets: dict[str, np.ndarray]) -> list[dict]:
    """Union of every variant's boxes — head, face, person, halfbody and any
    `EXTRA_MODEL` lineage — merged at IoU 0.5 into one id (README "Labels")."""
    cands: list[dict] = []
    for name, d in dets.items():
        for row in d:
            box, score = row[:4], float(row[4])
            if cands:
                ious = iou_matrix(box[None], np.array([c["box"] for c in cands]))[0]
                j = int(np.argmax(ious))
                if ious[j] >= 0.5:
                    c = cands[j]
                    c["by"][name] = max(c["by"].get(name, 0.0), score)
                    if score > c["score"]:
                        c["box"], c["score"] = box, score
                    continue
            cands.append({"box": box, "score": score, "by": {name: score}})
    for i, c in enumerate(cands):
        c["id"] = i + 1
        c["box"] = [round(float(v), 5) for v in c["box"]]
    return cands


def sheet(frame: Image.Image, cands: list[dict], label: str) -> Image.Image:
    w, h = frame.size
    top = frame.copy()
    d = ImageDraw.Draw(top)
    font = ImageFont.load_default(size=18)
    for c in cands:
        x0, y0, x1, y1 = (c["box"][0] * w, c["box"][1] * h, c["box"][2] * w, c["box"][3] * h)
        col = COLORS[c["id"] % len(COLORS)]
        d.rectangle([x0, y0, x1, y1], outline=col, width=3)
        d.text((x0 + 3, y0 + 2), str(c["id"]), fill=col, font=font, stroke_width=2, stroke_fill="black")
    d.text((8, 8), label, fill="white", font=font, stroke_width=2, stroke_fill="black")
    cell = 200
    per_row = max(1, w // cell)
    rows = (len(cands) + per_row - 1) // per_row
    out = Image.new("RGB", (w, h + rows * (cell + 24)), "black")
    out.paste(top, (0, 0))
    dd = ImageDraw.Draw(out)
    for k, c in enumerate(cands):
        x0, y0, x1, y1 = c["box"]
        cx, cy, side = (x0 + x1) / 2 * w, (y0 + y1) / 2 * h, max((x1 - x0) * w, (y1 - y0) * h) * 1.6
        crop = frame.crop((int(cx - side / 2), int(cy - side / 2), int(cx + side / 2), int(cy + side / 2))).resize((cell, cell))
        px, py = (k % per_row) * cell, h + (k // per_row) * (cell + 24)
        out.paste(crop, (px, py))
        dd.text((px + 4, py + cell + 2), f"#{c['id']}  {c['score']:.2f}", fill=COLORS[c["id"] % len(COLORS)], font=font)
    return out


def main() -> None:
    key, media, models, out = sys.argv[1:5]
    n = int(sys.argv[5]) if len(sys.argv) > 5 else 150
    seed = int(sys.argv[6]) if len(sys.argv) > 6 else 1
    out = Path(out) / key
    total = probe(media)[2]
    rng = random.Random(f"{key}:{seed}")
    frames = sorted(rng.sample(range(int(total * 0.01), int(total * 0.99)), n))
    # TARGETED=<file>: extra frame numbers (one per line or space-separated),
    # chosen to fill rare strata; they are recorded for `"targeted": true`.
    targeted: list[int] = []
    if (t := os.environ.get("TARGETED")) and Path(t).exists():
        targeted = sorted({int(x) for x in Path(t).read_text().split()})
        frames = sorted(set(frames) | set(targeted))
    if not (out / "frames").exists():
        extract(media, frames, out / "frames")
    (out / "targeted.txt").write_text("\n".join(str(f) for f in targeted) + "\n")
    all_dets: dict[str, dict] = {}
    variants = list(VARIANTS)
    # EXTRA_MODEL=<onnx>: a second source of another lineage (same
    # Ultralytics-style [5,N] output) so labellers see its misses too.
    if e := os.environ.get("EXTRA_MODEL"):
        variants.append((e, ["letterbox640"]))
    for folder, modes in variants:
        # VARIANTS entries name a folder under the models dir; EXTRA_MODEL
        # names the .onnx itself.
        model = Path(folder) if Path(folder).is_file() else Path(models) / folder / "model.onnx"
        det = Detector.load(str(model), Path(folder).stem)
        for mode in modes:
            name = f"{det.name}@{mode}"
            res = {}
            for f in frames:
                img = Image.open(out / "frames" / f"{f}.jpg")
                res[str(f)] = det.detect(img, mode).round(5).tolist()
            json.dump({"model": det.name, "input": mode, "frames": res}, open(out / f"{name}.dets.json", "w"))
            all_dets[name] = res
            print(key, name, sum(len(v) for v in res.values()), flush=True)
    (out / "sheets").mkdir(exist_ok=True)
    cands_all = {}
    for f in frames:
        per = {name: np.array(r[str(f)], np.float32).reshape(-1, 5) for name, r in all_dets.items()}
        cands = merge_heads({k: v[v[:, 4] >= 0.10] for k, v in per.items()})
        cands_all[str(f)] = cands
        img = Image.open(out / "frames" / f"{f}.jpg")
        sheet(img, cands, f"{key} frame {f}").save(out / "sheets" / f"{f}.jpg", quality=88)
    json.dump({"frames": {k: [{**c, "by": {a: round(b, 3) for a, b in c["by"].items()}} for c in v] for k, v in cands_all.items()}},
              open(out / "cands.json", "w"))


if __name__ == "__main__":
    main()
